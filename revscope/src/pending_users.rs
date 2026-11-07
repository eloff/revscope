//! Users with no way to log in yet, nearly always because they were invited and
//! haven't accepted.

use std::sync::Arc;

use log::info;

use crate::auth::{AuthError, map_query_error};
use crate::db::open_main_database_write;
use crate::server_context::ServerContext;

/// Days after the latest invite before a user who hasn't accepted is deleted.
const LAPSED_INVITE_DAYS: i32 = 30;

/// SQL for whether the user row aliased `user` is pending. Google and Apple users
/// have no password either, but have accepted.
pub(crate) fn pending_user_sql(user: &str) -> String {
    format!(
        "{user}.hashed_password IS NULL
         AND NOT EXISTS (SELECT 1 FROM user_identities i WHERE i.user_id = {user}.id)"
    )
}

/// Deletes invitees who never accepted, so they stop reserving their email address:
/// no live invite link, and invited long enough ago for an admin to have sent
/// another. Having logged in, or a legacy login provider, rules a user out, since
/// those aren't invitees whatever their credentials look like. So does nobody else
/// on the account having accepted, because then no one invited them.
pub(crate) async fn delete_lapsed_invites(
    server_context: Arc<ServerContext>,
) -> Result<(), AuthError> {
    let conn = open_main_database_write(server_context).await?;
    let deleted = conn
        .query(
            &format!(
                "DELETE FROM users u
                 WHERE {pending}
                   AND u.last_login_at IS NULL
                   AND u.login_provider = 0
                   AND u.invited_at < NOW() - INTERVAL '{LAPSED_INVITE_DAYS} days'
                   AND NOT EXISTS (
                       SELECT 1 FROM password_reset_tokens t
                       WHERE t.user_id = u.id AND t.expires_at > NOW()
                   )
                   AND EXISTS (
                       SELECT 1 FROM users other
                       WHERE other.account_id = u.account_id AND NOT ({other_pending})
                   )
                 RETURNING u.id::text, u.account_id::text",
                pending = pending_user_sql("u"),
                other_pending = pending_user_sql("other"),
            ),
            &[],
        )
        .await
        .map_err(map_query_error)?;
    for row in &deleted {
        let (user_id, account_id): (&str, &str) = (row.get(0), row.get(1));
        info!("deleted lapsed invite: user {user_id} on account {account_id}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::main::Connection;

    /// Inserts a user invited `age` ago, as a Postgres interval.
    async fn insert_user(conn: &Connection, account_id: &str, age: &str, password: bool) -> String {
        let hash: Option<&[u8]> = password.then_some(b"hash");
        conn.query_one(
            "INSERT INTO users (account_id, invited_at, name, email, hashed_password)
             VALUES (
                $1::text::uuid, NOW() - $2::text::interval, 'Someone',
                gen_random_uuid()::text || '@example.com', $3
             )
             RETURNING id::text",
            &[&account_id, &age, &hash],
        )
        .await
        .expect("insert user")
        .get(0)
    }

    /// `expires_in` is an offset from now, as a Postgres interval.
    async fn insert_token(conn: &Connection, user_id: &str, expires_in: &str) {
        conn.execute(
            "INSERT INTO password_reset_tokens (token_hash, user_id, expires_at)
             VALUES (uuid_send(gen_random_uuid()), $1::text::uuid, NOW() + $2::text::interval)",
            &[&user_id, &expires_in],
        )
        .await
        .expect("insert invite link");
    }

    async fn exists(conn: &Connection, user_id: &str) -> bool {
        conn.query_one(
            "SELECT EXISTS (SELECT 1 FROM users WHERE id = $1::text::uuid)",
            &[&user_id],
        )
        .await
        .expect("look up user")
        .get(0)
    }

    #[tokio::test]
    async fn only_invites_that_can_no_longer_be_accepted_are_deleted() {
        let (server_context, conn) = crate::test_utils::main_test_db("lapsed-invites").await;
        let account_id = crate::test_utils::insert_test_account(&conn, "lapsed_invites").await;
        let accepted = insert_user(&conn, &account_id, "40 days", true).await;
        let lapsed = insert_user(&conn, &account_id, "40 days", false).await;
        let expired_link = insert_user(&conn, &account_id, "40 days", false).await;
        insert_token(&conn, &expired_link, "-1 day").await;
        let logged_in = insert_user(&conn, &account_id, "40 days", false).await;
        conn.execute(
            "UPDATE users SET last_login_at = NOW() WHERE id = $1::text::uuid",
            &[&logged_in],
        )
        .await
        .expect("record login");
        let legacy_login = insert_user(&conn, &account_id, "40 days", false).await;
        conn.execute(
            "UPDATE users SET login_provider = 1 WHERE id = $1::text::uuid",
            &[&legacy_login],
        )
        .await
        .expect("record legacy provider");
        let recent = insert_user(&conn, &account_id, "1 day", false).await;
        let reinvited = insert_user(&conn, &account_id, "40 days", false).await;
        insert_token(&conn, &reinvited, "1 day").await;
        let federated = insert_user(&conn, &account_id, "40 days", false).await;
        conn.execute(
            "INSERT INTO user_identities (user_id, provider, subject)
             VALUES ($1::text::uuid, 'google', $1::text)",
            &[&federated],
        )
        .await
        .expect("link identity");

        delete_lapsed_invites(server_context.clone())
            .await
            .expect("delete lapsed invites");

        assert!(!exists(&conn, &lapsed).await);
        assert!(!exists(&conn, &expired_link).await);
        for kept in [
            &accepted,
            &recent,
            &reinvited,
            &logged_in,
            &legacy_login,
            &federated,
        ] {
            assert!(exists(&conn, kept).await);
        }
        conn.execute(
            "DELETE FROM accounts WHERE id = $1::text::uuid",
            &[&account_id],
        )
        .await
        .expect("delete account");
    }

    #[tokio::test]
    async fn a_pending_user_is_kept_when_nobody_on_the_account_has_accepted() {
        let (server_context, conn) = crate::test_utils::main_test_db("lone-pending").await;
        let account_id = crate::test_utils::insert_test_account(&conn, "lone_pending").await;
        let lone = insert_user(&conn, &account_id, "40 days", false).await;

        delete_lapsed_invites(server_context.clone())
            .await
            .expect("delete lapsed invites");

        assert!(exists(&conn, &lone).await);
        conn.execute(
            "DELETE FROM accounts WHERE id = $1::text::uuid",
            &[&account_id],
        )
        .await
        .expect("delete account");
    }
}
