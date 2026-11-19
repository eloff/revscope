//! Every account on the server, newest first, a page at a time, with its
//! users, projects with their last 30 days, and current subscription. For the
//! site admin only.

use std::sync::Arc;

use fxhash::FxHashMap;
use serde::Serialize;

use crate::auth::{AuthError, map_query_error};
use crate::authorize::{SiteAdmin, not_site_admin};
use crate::comparison::DAY_SECONDS;
use crate::db::project::parse_uuid_key;
use crate::db::{DbError, open_main_database_read, open_project_database_uncached_read};
use crate::overview::{ProjectTotals, project_totals};
use crate::pending_users::pending_user_sql;
use crate::query_params::{parse_offset, query_pairs};
use crate::server_context::ServerContext;
use crate::user_access::require_admin_role;
use crate::util::{block_on, seconds_since_epoch};

pub(crate) const PAGE_SIZE: i64 = 100;
const RECENT_SECONDS: i64 = 30 * DAY_SECONDS;

#[derive(Debug, Serialize)]
pub(crate) struct SiteAdminAccountsResponse {
    accounts: Vec<SiteAdminAccount>,
    /// Every account on the server, across all pages.
    total: i64,
    offset: i64,
    page_size: i64,
}

#[derive(Debug, Serialize)]
struct SiteAdminAccount {
    account_id: String,
    name: String,
    /// Unix seconds. Read from the account's UUIDv7 id, which records when the
    /// row was made, since accounts have no creation column.
    signed_up_at: Option<i64>,
    /// The site admin's own account, which they can't delete.
    is_own: bool,
    /// `None` only for an account whose open period is missing, which signup
    /// never leaves.
    subscription: Option<Subscription>,
    /// Oldest first.
    users: Vec<User>,
    /// By name.
    projects: Vec<Project>,
}

#[derive(Debug, Serialize)]
struct Subscription {
    plan: String,
    billing_period: String,
    regular_price_cents: i32,
    actual_price_cents: i32,
    /// Unix seconds when the discounted price ends.
    discount_ends_at: Option<i64>,
    /// `None` for unmetered plans.
    events_monthly_limit: Option<i32>,
    /// Unix seconds when these terms started.
    started_at: i64,
    trial_ends_at: Option<i64>,
    /// `stripe` or `polar`; `None` for a plan nobody pays a provider for.
    provider: Option<&'static str>,
    customer_id: Option<String>,
    subscription_id: Option<String>,
    /// The provider's status, as it reported it.
    status: Option<String>,
    /// Unix seconds when the period already paid for ends.
    current_period_end: Option<i64>,
}

#[derive(Debug, Serialize)]
struct User {
    user_id: String,
    name: String,
    email: String,
    is_admin: bool,
    /// Invited but hasn't chosen a password yet.
    pending: bool,
    /// Unix seconds when the user was made, from the UUIDv7 id like the
    /// account's: their signup, or an invitee's first invite.
    signed_up_at: Option<i64>,
    /// Joined the account by invite: anyone but its earliest remaining user,
    /// who signed it up unless an admin has since removed them. `invited_at`
    /// can't tell, since its migration set it on every user without a
    /// password, Google and Apple signups included.
    invited: bool,
    /// Unix seconds when they followed the link that confirms their address,
    /// or else when they last proved it another way: a password reset link, or
    /// signing in with Google or Apple at an address the provider hosts.
    email_verified_at: Option<i64>,
    /// Google or Apple, linked to the user. A sign-in that vouched for the
    /// address set `email_verified_at`; these name the providers either way.
    sign_in_providers: Vec<String>,
    last_login_at: Option<i64>,
}

#[derive(Debug, Serialize)]
struct Project {
    id: i32,
    name: String,
    /// Tracked visitors and revenue in the 30 days up to now; `None` when its
    /// database couldn't be read.
    last_30_days: Option<ProjectTotals>,
}

/// The `offset` of the first account to list; absent means the first page.
pub(crate) fn parse_accounts_query(target: &str) -> Result<i64, &'static str> {
    let mut offset = 0;
    for (key, value) in query_pairs(target) {
        if key == "offset" {
            offset = parse_offset(value)?;
        }
    }
    Ok(offset)
}

pub(crate) fn list_accounts(
    server_context: Arc<ServerContext>,
    admin: &SiteAdmin,
    offset: i64,
) -> Result<SiteAdminAccountsResponse, AuthError> {
    let own_account_id = admin.user().account_id.clone();
    let admin_user_id = admin.user().user_id.clone();
    let main_context = server_context.clone();
    let mut response = block_on(async move {
        let conn = open_main_database_read(main_context).await?;
        // Another process may serve the session for a while after a demotion,
        // and this lists every customer, so the role is read again here.
        require_admin_role(&conn, &own_account_id, &admin_user_id)
            .await
            .map_err(not_site_admin)?;
        let total: i64 = conn
            .query_one("SELECT COUNT(*) FROM accounts", &[])
            .await
            .map_err(map_query_error)?
            .get(0);
        // Accounts made by signup have UUIDv7 ids, which record when. The two
        // the first migration seeded don't, so they go last.
        let rows = conn
            .query(
                "SELECT
                    a.id::text,
                    a.name,
                    EXTRACT(EPOCH FROM uuid_extract_timestamp(a.id))::bigint,
                    EXTRACT(EPOCH FROM a.trial_ends_at)::bigint,
                    sub.plan_type::text,
                    sub.billing_period::text,
                    sub.regular_price_cents,
                    sub.actual_price_cents,
                    EXTRACT(EPOCH FROM sub.discount_ends_at)::bigint,
                    sub.events_monthly_limit,
                    EXTRACT(EPOCH FROM sub.started_at)::bigint,
                    sub.stripe_customer_id,
                    sub.stripe_subscription_id,
                    sub.polar_customer_id,
                    sub.polar_subscription_id,
                    sub.subscription_status,
                    EXTRACT(EPOCH FROM sub.current_period_end)::bigint
                 FROM accounts a
                 LEFT JOIN account_subscriptions sub
                    ON sub.account_id = a.id AND sub.ended_at IS NULL
                 ORDER BY uuid_extract_timestamp(a.id) DESC NULLS LAST, a.id DESC
                 LIMIT $1 OFFSET $2",
                &[&PAGE_SIZE, &offset],
            )
            .await
            .map_err(map_query_error)?;
        let account_ids: Vec<String> = rows.iter().map(|row| row.get(0)).collect();

        let mut users = FxHashMap::<String, Vec<User>>::default();
        for row in conn
            .query(
                &format!(
                    "SELECT
                        u.account_id::text,
                        u.id::text,
                        u.name,
                        COALESCE(u.original_email, u.email),
                        u.is_admin,
                        {pending},
                        EXTRACT(EPOCH FROM uuid_extract_timestamp(u.id))::bigint,
                        u.id <> (
                            SELECT earliest.id FROM users earliest
                            WHERE earliest.account_id = u.account_id
                            ORDER BY earliest.id
                            LIMIT 1
                        ),
                        EXTRACT(EPOCH FROM COALESCE(u.email_confirmed_at, u.email_verified_at))::bigint,
                        ARRAY(
                            SELECT i.provider FROM user_identities i
                            WHERE i.user_id = u.id
                            ORDER BY i.provider
                        ),
                        EXTRACT(EPOCH FROM u.last_login_at)::bigint
                     FROM users u
                     WHERE u.account_id = ANY($1::text[]::uuid[])
                     ORDER BY u.id",
                    pending = pending_user_sql("u"),
                ),
                &[&account_ids],
            )
            .await
            .map_err(map_query_error)?
        {
            users.entry(row.get(0)).or_default().push(User {
                user_id: row.get(1),
                name: row.get(2),
                email: row.get(3),
                is_admin: row.get(4),
                pending: row.get(5),
                signed_up_at: row.get(6),
                invited: row.get(7),
                email_verified_at: row.get(8),
                sign_in_providers: row.get(9),
                last_login_at: row.get(10),
            });
        }

        let mut projects = FxHashMap::<String, Vec<Project>>::default();
        for row in conn
            .query(
                "SELECT account_id::text, id, name FROM projects
                 WHERE account_id = ANY($1::text[]::uuid[])
                 ORDER BY name",
                &[&account_ids],
            )
            .await
            .map_err(map_query_error)?
        {
            projects.entry(row.get(0)).or_default().push(Project {
                id: row.get(1),
                name: row.get(2),
                last_30_days: None,
            });
        }

        let accounts = rows
            .iter()
            .map(|row| {
                let account_id: String = row.get(0);
                SiteAdminAccount {
                    name: row.get(1),
                    signed_up_at: row.get(2),
                    is_own: account_id == own_account_id,
                    subscription: subscription(row),
                    users: users.remove(&account_id).unwrap_or_default(),
                    projects: projects.remove(&account_id).unwrap_or_default(),
                    account_id,
                }
            })
            .collect();
        Ok::<_, AuthError>(SiteAdminAccountsResponse {
            accounts,
            total,
            offset,
            page_size: PAGE_SIZE,
        })
    })?;

    // Read after the Postgres work so no SQLite checkout blocks inside `block_on`.
    // The end is exclusive and whole seconds, so this one counts what landed in
    // the current second.
    let end = i64::from(seconds_since_epoch()) + 1;
    for account in &mut response.accounts {
        for project in &mut account.projects {
            project.last_30_days =
                recent_totals(&server_context, &account.account_id, project, end)
                    .inspect_err(|error| {
                        log::warn!(
                            "site admin: no totals for project {}: {error:?}",
                            project.id
                        );
                    })
                    .ok();
        }
    }
    Ok(response)
}

fn recent_totals(
    server_context: &ServerContext,
    account_id: &str,
    project: &Project,
    end: i64,
) -> Result<ProjectTotals, AuthError> {
    let account_key = parse_uuid_key(account_id).ok_or_else(|| {
        DbError::InvalidProjectId(format!("bad account_id for project {}", project.id))
    })?;
    let conn = open_project_database_uncached_read(
        server_context,
        project.id,
        project.name.clone(),
        account_key,
    )?;
    project_totals(&conn, end - RECENT_SECONDS, end)
}

/// The open period from a [`list_accounts`] row, starting at column 3.
fn subscription(row: &tokio_postgres::Row) -> Option<Subscription> {
    let plan: Option<String> = row.get(4);
    let stripe_customer_id: Option<String> = row.get(11);
    let stripe_subscription_id: Option<String> = row.get(12);
    let polar_customer_id: Option<String> = row.get(13);
    let polar_subscription_id: Option<String> = row.get(14);
    // Customer ids carry over into every later period, so an account that moved
    // provider holds both; its subscription id says which one bills it now.
    let (provider, customer_id, subscription_id) =
        match (stripe_subscription_id, polar_subscription_id) {
            (Some(id), _) => (Some("stripe"), stripe_customer_id, Some(id)),
            (None, Some(id)) => (Some("polar"), polar_customer_id, Some(id)),
            (None, None) => (None, None, None),
        };
    Some(Subscription {
        plan: plan?,
        trial_ends_at: row.get(3),
        billing_period: row.get(5),
        regular_price_cents: row.get(6),
        actual_price_cents: row.get(7),
        discount_ends_at: row.get(8),
        events_monthly_limit: row.get(9),
        started_at: row.get(10),
        provider,
        customer_id,
        subscription_id,
        status: row.get(15),
        current_period_end: row.get(16),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_defaults_to_the_first_page() {
        assert_eq!(
            parse_accounts_query("/v2/session/site-admin/accounts"),
            Ok(0)
        );
        assert_eq!(
            parse_accounts_query("/v2/session/site-admin/accounts?offset=200"),
            Ok(200)
        );
        assert!(parse_accounts_query("/v2/session/site-admin/accounts?offset=-1").is_err());
    }
}
