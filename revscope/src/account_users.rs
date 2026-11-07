//! The users on the signed-in admin's account, including invites not yet
//! accepted, with each one's role and projects.

use std::sync::Arc;

use serde::Serialize;

use crate::auth::{AuthError, map_query_error};
use crate::authorize::AdminAccount;
use crate::db::open_main_database_read;
use crate::pending_users::pending_user_sql;
use crate::server_context::ServerContext;
use crate::util::block_on;

#[derive(Debug, Serialize)]
pub(crate) struct AccountUsersResponse {
    /// Oldest first.
    users: Vec<AccountUser>,
}

#[derive(Debug, Serialize)]
struct AccountUser {
    user_id: String,
    name: String,
    email: String,
    /// Invited but hasn't chosen a password yet.
    pending: bool,
    /// Unix seconds; `None` if they've never logged in.
    last_login_at: Option<i64>,
    is_admin: bool,
    /// `None` for every project on the account, now and later.
    project_ids: Option<Vec<i32>>,
}

pub(crate) fn account_users(
    server_context: Arc<ServerContext>,
    admin: &AdminAccount,
) -> Result<AccountUsersResponse, AuthError> {
    let user = admin.user();

    block_on(async move {
        let conn = open_main_database_read(server_context).await?;
        // Ids are UUIDv7, so they sort by creation time.
        let rows = conn
            .query(
                &format!(
                    "SELECT
                    u.id::text,
                    u.name,
                    COALESCE(u.original_email, u.email),
                    {pending},
                    EXTRACT(EPOCH FROM u.last_login_at)::bigint,
                    u.is_admin,
                    CASE WHEN u.all_projects THEN NULL ELSE ARRAY(
                        SELECT up.project_id FROM user_projects up
                        WHERE up.user_id = u.id
                        ORDER BY up.project_id
                    ) END
                 FROM users u
                 WHERE u.account_id = $1::text::uuid
                 ORDER BY u.id",
                    pending = pending_user_sql("u"),
                ),
                &[&user.account_id],
            )
            .await
            .map_err(map_query_error)?;
        let users = rows
            .iter()
            .map(|row| AccountUser {
                user_id: row.get(0),
                name: row.get(1),
                email: row.get(2),
                pending: row.get(3),
                last_login_at: row.get(4),
                is_admin: row.get(5),
                project_ids: row.get(6),
            })
            .collect();
        Ok(AccountUsersResponse { users })
    })
}
