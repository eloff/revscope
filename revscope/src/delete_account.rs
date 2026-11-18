//! The site admin deleting another account, with all of its users and
//! projects and the projects' SQLite databases.
//!
//! Nothing here can be undone. Postgres cascades the account's users, projects
//! and subscriptions, and what hangs off them, from its row. As when deleting
//! a user or a project, the users' Ask history on the public demo and the
//! projects' logged ingest errors stay, since neither has a key to cascade
//! from. The projects' names stay retired with no account to claim
//! them back (see `claim_project_name`), since sites may still send them
//! traffic. As with deleting a project, the caller names the account in the
//! body, so a stray request for the wrong id deletes nothing.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::auth::AuthError;
use crate::authorize::{SiteAdmin, not_site_admin};
use crate::db::open_main_database_write;
use crate::db::project::delete_project_database;
use crate::delete_project::revoke_project_tokens;
use crate::ga_import::take_project_tokens;
use crate::projects::is_public_project_name;
use crate::server_context::ServerContext;
use crate::subscription_status;
use crate::user_access::{forget_user, lock_account_as_admin};
use crate::util::block_on;
use crate::uuid_text::{format_uuid, parse_hyphenated_uuid};

const NO_SUCH_ACCOUNT: &str = "no such account";
const DELETE_OWN_ACCOUNT: &str = "You can't delete the account you're signed in to.";
const DELETE_DEMO: &str = "This account holds the public demo, so it can't be deleted.";
const LIVE_SUBSCRIPTION: &str = "Stripe or Polar may still charge this account. Cancel its subscription with the provider first, so they don't keep charging for an account that's gone.";

#[derive(Debug, Deserialize)]
pub(crate) struct DeleteAccountRequest {
    pub(crate) account_id: String,
    /// Must match the account's name exactly.
    pub(crate) confirm_name: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct DeleteAccountResponse {
    deleted: String,
    users_deleted: usize,
    projects_deleted: usize,
}

pub(crate) fn delete_account(
    server_context: Arc<ServerContext>,
    admin: &SiteAdmin,
    request: DeleteAccountRequest,
) -> Result<DeleteAccountResponse, AuthError> {
    let actor = admin.user();
    let account_id = parse_hyphenated_uuid(request.account_id.trim())
        .map(format_uuid)
        .ok_or(AuthError::NotFound(NO_SUCH_ACCOUNT))?;
    if account_id == actor.account_id {
        return Err(AuthError::Conflict(DELETE_OWN_ACCOUNT));
    }

    let (project_ids, user_ids, stranded) = block_on(async {
        let conn = open_main_database_write(server_context.clone()).await?;
        conn.in_transaction(async |conn| {
            // The site admin's role is read from a session that another process
            // may serve for a while after a demotion, so it's checked again here.
            lock_account_as_admin(conn, &actor.account_id, &actor.user_id)
                .await
                .map_err(not_site_admin)?;
            // The billing webhooks lock a period before its account, so this
            // does too, or the two could deadlock.
            if has_live_subscription(conn, &account_id).await? {
                return Err(AuthError::Conflict(LIVE_SUBSCRIPTION));
            }
            let name: String = conn
                .query_opt(
                    "SELECT name FROM accounts WHERE id = $1::text::uuid FOR UPDATE",
                    &[&account_id],
                )
                .await?
                .ok_or(AuthError::NotFound(NO_SUCH_ACCOUNT))?
                .get(0);
            // A plan change that committed while the check above waited on its
            // period opened a new one the check couldn't see. Holding the
            // account now keeps any more from being inserted, so look again.
            if has_live_subscription(conn, &account_id).await? {
                return Err(AuthError::Conflict(LIVE_SUBSCRIPTION));
            }
            if request.confirm_name.trim() != name {
                return Err(AuthError::InvalidRequest(
                    "Type the account name exactly to confirm.",
                ));
            }
            let projects: Vec<(i32, String)> = conn
                .query(
                    "SELECT id, name FROM projects WHERE account_id = $1::text::uuid",
                    &[&account_id],
                )
                .await?
                .iter()
                .map(|row| (row.get(0), row.get(1)))
                .collect();
            if projects
                .iter()
                .any(|(_, name)| is_public_project_name(name))
            {
                return Err(AuthError::Conflict(DELETE_DEMO));
            }
            let mut stranded = Vec::new();
            for (project_id, _) in &projects {
                stranded.extend(take_project_tokens(conn, *project_id).await?);
            }
            let user_ids: Vec<String> = conn
                .query(
                    "SELECT id::text FROM users WHERE account_id = $1::text::uuid",
                    &[&account_id],
                )
                .await?
                .iter()
                .map(|row| row.get(0))
                .collect();
            conn.execute(
                "INSERT INTO retired_project_names (name, account_id)
                 SELECT name, NULL FROM projects WHERE account_id = $1::text::uuid
                 ON CONFLICT (name) DO UPDATE
                 SET account_id = NULL, retired_at = NOW()",
                &[&account_id],
            )
            .await?;
            conn.execute(
                "DELETE FROM accounts WHERE id = $1::text::uuid",
                &[&account_id],
            )
            .await?;
            let project_ids: Vec<i32> = projects.into_iter().map(|(id, _)| id).collect();
            Ok((project_ids, user_ids, stranded))
        })
        .await
    })?;

    for user_id in &user_ids {
        forget_user(&server_context, user_id);
    }
    revoke_project_tokens(&server_context, stranded);
    // The account is already gone from Postgres, so failing here would only
    // make a retry answer 404; leave a stray file for an operator instead.
    for project_id in &project_ids {
        let project_id = project_id.to_string();
        if let Err(error) = delete_project_database(server_context.as_ref(), &project_id) {
            log::error!(
                "deleted account {account_id} but could not remove project {project_id}'s database: {error}"
            );
        }
    }
    Ok(DeleteAccountResponse {
        deleted: account_id,
        users_deleted: user_ids.len(),
        projects_deleted: project_ids.len(),
    })
}

/// Whether a provider may still charge the account: its current period has a
/// subscription that isn't canceled (a status we can't place counts as live),
/// or an earlier one ended unpaid and the billing sweep hasn't yet stopped its
/// provider collecting. Deleting the account would also delete the row that
/// sweep retries from. Locks the periods it reads, so a webhook can't change
/// one between this check and the delete; it can't see a period inserted
/// after its statement began.
async fn has_live_subscription(
    conn: &crate::db::main::Connection,
    account_id: &str,
) -> Result<bool, AuthError> {
    let rows = conn
        .query(
            "SELECT ended_at IS NULL, ended_unpaid, subscription_status,
                    stripe_subscription_id IS NOT NULL OR polar_subscription_id IS NOT NULL
             FROM account_subscriptions
             WHERE account_id = $1::text::uuid
             FOR UPDATE",
            &[&account_id],
        )
        .await?;
    Ok(rows.iter().any(|row| {
        let (open, ended_unpaid, status, has_subscription): (bool, bool, Option<String>, bool) =
            (row.get(0), row.get(1), row.get(2), row.get(3));
        let status = status.as_deref().unwrap_or("");
        (open
            && has_subscription
            && subscription_status::normalize(status) != subscription_status::CANCELED)
            || (ended_unpaid && status == "unpaid")
    }))
}
