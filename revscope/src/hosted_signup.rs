//! Signup on revscope.co: each signup is a new account and project for the
//! signer's website, on the plan they chose. Compiled out of self-hosted builds,
//! where signup joins the instance's shared account instead (see `self_hosted.rs`).
//! A build without the private `billing` module gives every signup a
//! complimentary account.

use std::sync::Arc;

use tokio_postgres::error::SqlState;

use crate::account_subscription::{PlanTerms, insert_open_period};
use crate::auth::{
    AuthError, AuthSuccess, NewAccountRequest, NewUser, db_constraint, insert_new_user,
    is_timestamp_conflict, issue_session, map_account_subscription_error, map_query_error,
    timezone_for_signup,
};
#[cfg(feature = "billing")]
use crate::billing::plans::signup_terms;
use crate::db::project::{delete_project_database, parse_uuid_key};
use crate::db::{insert_project, open_main_database_write, remove_project_database};
use crate::projects::{
    PROJECT_NAME_TAKEN, claim_project_name, is_project_name_retired, is_project_name_taken,
    normalize_website, signup_project_name,
};
use crate::server_context::ServerContext;

/// Without the private billing module there is no catalog to price a plan
/// from, so the plan fields the signup names are ignored.
#[cfg(not(feature = "billing"))]
fn signup_terms(_account: &NewAccountRequest) -> Result<PlanTerms, AuthError> {
    Ok(PlanTerms::COMPLIMENTARY)
}

const MIN_ACCOUNT_NAME_LENGTH: usize = 5;
const ACCOUNT_FOR_WEBSITE_EXISTS: &str = "An account with this website already exists.";
/// A name taken between the pre-check and the project insert. Its own message,
/// not [`PROJECT_NAME_TAKEN`], because by then an account was written and
/// rolled back, so it isn't refunded like a refusal that wrote nothing.
const PROJECT_NAME_JUST_TAKEN: &str = "Someone just took this project name. Try another name.";

/// Turns away a signup whose website or plan can't make an account, before it
/// writes anything.
pub(crate) fn validate_account(account: &NewAccountRequest) -> Result<(), AuthError> {
    validated_account(account).map(|_| ())
}

struct ValidatedAccount {
    account_name: String,
    project_name: String,
    terms: PlanTerms,
}

fn validated_account(account: &NewAccountRequest) -> Result<ValidatedAccount, AuthError> {
    let account_name = normalize_website(account.account_name.as_str());
    if account_name.len() < MIN_ACCOUNT_NAME_LENGTH {
        return Err(AuthError::InvalidRequest(
            "Website must be at least 5 characters.",
        ));
    }
    if !is_valid_website_account_name(&account_name) {
        return Err(AuthError::InvalidRequest(
            "Website must include a domain (e.g. mywebsite.com).",
        ));
    }
    let project_name = signup_project_name(&account_name, account.project_name.as_deref())?;
    let terms = signup_terms(account)?;
    Ok(ValidatedAccount {
        account_name,
        project_name,
        terms,
    })
}

/// Signup on revscope.co: a new account and project for the website, on the
/// chosen plan, with `user` as its first user.
pub(crate) async fn signup(
    server_context: Arc<ServerContext>,
    account: &NewAccountRequest,
    user: NewUser<'_>,
) -> Result<AuthSuccess, AuthError> {
    let ValidatedAccount {
        account_name,
        project_name,
        terms,
    } = validated_account(account)?;
    let timezone = timezone_for_signup(account.timezone.as_deref());
    let conn = open_main_database_write(server_context.clone()).await?;
    // Turns the usual cases away before any row exists, which is what lets a
    // taken name not use up a signup (see `is_fixable_signup_error`). The
    // inserts below still catch one taken in between.
    if conn
        .query_opt("SELECT 1 FROM accounts WHERE name = $1", &[&account_name])
        .await
        .map_err(map_query_error)?
        .is_some()
    {
        return Err(AuthError::Conflict(ACCOUNT_FOR_WEBSITE_EXISTS));
    }
    if is_project_name_retired(&conn, &project_name).await?
        || conn
            .query_opt("SELECT 1 FROM projects WHERE name = $1", &[&project_name])
            .await
            .map_err(map_query_error)?
            .is_some()
    {
        return Err(AuthError::Conflict(PROJECT_NAME_TAKEN));
    }

    let account_row = conn
        .query_one(
            // trial_ends_at is NULL until payments are enabled; then set to
            // NOW() + INTERVAL '{TRIAL_DAYS} days' (sync with web/site/src/lib/pricing.ts).
            "INSERT INTO accounts (name, trial_ends_at)
             VALUES ($1, NULL)
             RETURNING id::text",
            &[&account_name],
        )
        .await
        .map_err(map_account_insert_error)?;
    let account_id: String = account_row.get(0);

    // Opens the account's first subscription period. Prices are locked in at
    // signup; catalog changes do not rewrite them.
    if let Err(error) = insert_open_period(&conn, &account_id, &terms).await {
        let _ = conn
            .execute(
                "DELETE FROM accounts WHERE id = $1::text::uuid",
                &[&account_id],
            )
            .await;
        return Err(map_account_subscription_error(error));
    }

    let project_row = match conn
        .query_one(
            "INSERT INTO projects (account_id, name) VALUES ($1::text::uuid, $2) RETURNING id",
            &[&account_id, &project_name],
        )
        .await
    {
        Ok(row) => row,
        Err(error) => {
            let _ = conn
                .execute(
                    "DELETE FROM accounts WHERE id = $1::text::uuid",
                    &[&account_id],
                )
                .await;
            return Err(map_project_insert_error(error));
        }
    };
    let project_id: i32 = project_row.get(0);
    if let Err(error) = claim_project_name(&conn, &project_name, &account_id).await {
        // Signup isn't one transaction, so the project row was visible: an
        // ingest for the name may have cached it. The tombstone stops that
        // cache entry from recording anything, even if the delete below fails.
        let _ = delete_project_database(server_context.as_ref(), &project_id.to_string());
        let _ = conn
            .execute(
                "DELETE FROM accounts WHERE id = $1::text::uuid",
                &[&account_id],
            )
            .await;
        return Err(match error {
            AuthError::Conflict(_) => AuthError::Conflict(PROJECT_NAME_JUST_TAKEN),
            error => error,
        });
    }
    if let Err(error) = conn
        .execute(
            "INSERT INTO project_settings (project_id, timezone) VALUES ($1, $2)",
            &[&project_id, &timezone],
        )
        .await
    {
        let _ = conn
            .execute(
                "DELETE FROM accounts WHERE id = $1::text::uuid",
                &[&account_id],
            )
            .await;
        return Err(map_query_error(error));
    }
    let project_id_str = project_id.to_string();
    // Identity is already validated and persisted; the SQLite file is created
    // lazily on first open. Do not roll back the account/project if caching fails.
    let account_key =
        parse_uuid_key(&account_id).expect("postgres account UUID text must be a valid UUID");
    let _project = insert_project(
        server_context.as_ref(),
        project_id,
        project_name.clone(),
        account_key,
    );

    match insert_new_user(&conn, &account_id, &user).await {
        Ok(user_id) => {
            drop(conn);
            issue_session(server_context, &user_id).await
        }
        Err(error) => {
            // Best effort cleanup if user insert fails after creating account/project/sqlite.
            let _ = remove_project_database(server_context.as_ref(), &project_id_str);
            let _ = conn
                .execute(
                    "DELETE FROM accounts WHERE id = $1::text::uuid",
                    &[&account_id],
                )
                .await;
            Err(error)
        }
    }
}

/// Website/account names must have at least one character on either side of a `.`.
/// A host, optionally with a port: something on both sides of a dot, and only
/// characters a hostname can hold. Non-ASCII is let through for IDNs.
fn is_valid_website_account_name(account_name: &str) -> bool {
    let host = match account_name.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => host,
        Some(_) => return false,
        None => account_name,
    };
    let host_chars = host.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.') || !byte.is_ascii()
    });
    match host.split_once('.') {
        Some((before, after)) => host_chars && !before.is_empty() && !after.is_empty(),
        None => false,
    }
}

fn map_account_insert_error(error: tokio_postgres::Error) -> AuthError {
    if is_timestamp_conflict(&error, "accounts_pkey") {
        return AuthError::Temporary("a temporary error occurred, please try again");
    }
    if error.code() == Some(&SqlState::UNIQUE_VIOLATION)
        && db_constraint(&error) == Some("accounts_name_key")
    {
        return AuthError::Conflict(ACCOUNT_FOR_WEBSITE_EXISTS);
    }
    map_query_error(error)
}

fn map_project_insert_error(error: tokio_postgres::Error) -> AuthError {
    if is_project_name_taken(&error) {
        return AuthError::Conflict(PROJECT_NAME_JUST_TAKEN);
    }
    map_query_error(error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_website_that_does_not_reduce_to_a_host_is_rejected() {
        // Not a scheme: it must start with a letter, and must not be empty.
        // Whatever is left must still look like a host.
        for raw in [
            "1http://example.com",
            "://example.com",
            "1web.site://example.com",
            "exa mple.com",
            "example.com:",
            "example.com:80a",
            "user@example.com",
        ] {
            assert!(
                !is_valid_website_account_name(&normalize_website(raw)),
                "{raw}"
            );
        }
    }

    #[test]
    fn a_website_needs_something_on_both_sides_of_a_dot() {
        assert!(is_valid_website_account_name("example.com"));
        assert!(is_valid_website_account_name("a.b"));
        assert!(is_valid_website_account_name("shop.example.co.uk"));
        assert!(is_valid_website_account_name("my-shop.example.com:8080"));
        assert!(is_valid_website_account_name("bücher.de"));
        assert!(!is_valid_website_account_name("localhost"));
        assert!(!is_valid_website_account_name(".com"));
        assert!(!is_valid_website_account_name("example."));
        assert!(!is_valid_website_account_name(""));
    }
}
