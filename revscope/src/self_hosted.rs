//! Accounts on a self-hosted instance (the `self-hosted` cargo feature).
//!
//! There is no billing and no plan to pick. The instance has one shared
//! account, and only the first user signs up, as its admin: that signup
//! creates the account, on a complimentary period with no event limit, and its
//! first project, named as the signup chose. Everyone after that is invited by
//! an admin (see `invite_user.rs`), and the account may add more projects.

use std::sync::Arc;

use crate::account_subscription::{PlanTerms, insert_open_period};
use crate::auth::{AuthError, map_account_subscription_error};
use crate::db::main::Connection;
use crate::projects::PROJECT_NAME_TAKEN;
use crate::server_context::ServerContext;

pub(crate) const ENABLED: bool = cfg!(feature = "self-hosted");
const ACCOUNT_NAME: &str = "self-hosted";
/// Postgres advisory lock key serializing signups, so two people racing to be
/// the first user can't both get in. Arbitrary, but must not collide with
/// other advisory locks on the database.
const SIGNUP_LOCK_KEY: i64 = 0x7265_7673_6967_6e75;
/// Advisory locks span the database, but the users a signup checks are per
/// schema, and e2e test runs each have their own.
const SIGNUP_LOCK_ID: &str = "$1::bigint + hashtext(current_schema())";
pub(crate) const SIGNUP_CLOSED: &str = "Signup is closed. Ask an existing user to invite you.";

/// Whether a new visitor may still sign up. Always on revscope.co; on a
/// self-hosted instance, only until the first user exists.
pub(crate) async fn signup_open(server_context: Arc<ServerContext>) -> Result<bool, AuthError> {
    if !ENABLED {
        return Ok(true);
    }
    let conn = crate::db::open_main_database_read(server_context).await?;
    signup_is_open(&conn).await
}

/// Whether the instance still has no users, so signup is open. Any user
/// closes it, whichever account or project they belong to.
pub(crate) async fn signup_is_open(conn: &Connection) -> Result<bool, AuthError> {
    let row = conn
        .query_one("SELECT NOT EXISTS (SELECT 1 FROM users)", &[])
        .await?;
    Ok(row.get(0))
}

/// Takes the signup lock on this connection, or fails with a retryable error
/// if another signup holds it. Callers must [`unlock_signup`] on the same
/// connection, since the lock outlives the statement and returns to the pool.
pub(crate) async fn lock_signup(conn: &Connection) -> Result<(), AuthError> {
    let row = conn
        .query_one(
            &format!("SELECT pg_try_advisory_lock({SIGNUP_LOCK_ID})"),
            &[&SIGNUP_LOCK_KEY],
        )
        .await?;
    if row.get::<_, bool>(0) {
        Ok(())
    } else {
        Err(AuthError::Temporary(
            "another signup is in progress, please try again",
        ))
    }
}

pub(crate) async fn unlock_signup(conn: &Connection) {
    if let Err(error) = conn
        .execute(
            &format!("SELECT pg_advisory_unlock({SIGNUP_LOCK_ID})"),
            &[&SIGNUP_LOCK_KEY],
        )
        .await
    {
        log::error!("failed to release the self-hosted signup lock: {error}");
    }
}

/// The shared account for the instance's first user, creating it and the
/// project `project_name` if they don't exist yet. `timezone` becomes that
/// project's timezone. Callers hold the signup lock, have checked
/// [`signup_is_open`], and have validated `project_name`. Returns the account
/// and project ids; the caller caches the project once its transaction commits.
///
/// Callers run this in the same transaction as the user insert, so a failed
/// signup leaves nothing behind. A failed signup on a version before that
/// transaction could leave the account behind: it's reused, and given its
/// subscription period if it has none. A project such a signup left keeps its
/// name and stays on the account.
pub(crate) async fn signup_account(
    conn: &Connection,
    project_name: &str,
    timezone: &str,
) -> Result<(String, i32), AuthError> {
    // Checked before writing anything, so refusing leaves nothing behind.
    let existing_owner = conn
        .query_opt(
            "SELECT a.name FROM projects p
             INNER JOIN accounts a ON a.id = p.account_id
             WHERE p.name = $1",
            &[&project_name],
        )
        .await?;
    if let Some(owner) = existing_owner
        && owner.get::<_, String>(0) != ACCOUNT_NAME
    {
        return Err(AuthError::Conflict(PROJECT_NAME_TAKEN));
    }

    // `DO UPDATE` rather than `DO NOTHING` so the id comes back either way.
    let account_id: String = conn
        .query_one(
            "INSERT INTO accounts (name, trial_ends_at)
             VALUES ($1, NULL)
             ON CONFLICT (name) DO UPDATE SET name = EXCLUDED.name
             RETURNING id::text",
            &[&ACCOUNT_NAME],
        )
        .await?
        .get(0);

    let has_open_period: bool = conn
        .query_one(
            "SELECT EXISTS (
                SELECT 1 FROM account_subscriptions
                WHERE account_id = $1::text::uuid AND ended_at IS NULL
             )",
            &[&account_id],
        )
        .await?
        .get(0);
    if !has_open_period {
        insert_open_period(conn, &account_id, &PlanTerms::COMPLIMENTARY)
            .await
            .map_err(map_account_subscription_error)?;
    }

    conn.execute(
        "INSERT INTO projects (account_id, name)
         VALUES ($1::text::uuid, $2)
         ON CONFLICT (name) DO NOTHING",
        &[&account_id, &project_name],
    )
    .await?;
    let project_id: i32 = conn
        .query_one(
            "SELECT id FROM projects WHERE name = $1 AND account_id = $2::text::uuid",
            &[&project_name, &account_id],
        )
        .await?
        .get(0);
    conn.execute(
        "INSERT INTO project_settings (project_id, timezone) VALUES ($1, $2)
         ON CONFLICT (project_id) DO UPDATE SET timezone = EXCLUDED.timezone",
        &[&project_id, &timezone],
    )
    .await?;
    Ok((account_id, project_id))
}
