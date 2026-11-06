//! Removes another user, or a pending invite, from the signed-in admin's
//! account. Their sessions, API tokens and invite link go with them; their Ask
//! usage stays, so it still counts against the account's budget.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::auth::{AuthError, map_query_error};
use crate::authorize::AdminAccount;
use crate::db::open_main_database_write;
use crate::server_context::ServerContext;
use crate::update_user::other_user_id;
use crate::user_access::{forget_user, lock_account_as_admin};
use crate::util::block_on;

const REMOVE_SELF: &str = "You can't remove yourself. Ask another admin.";

#[derive(Debug, Deserialize)]
pub(crate) struct RemoveUserRequest {
    pub(crate) user_id: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct RemoveUserResponse {
    ok: bool,
}

pub(crate) fn remove_user(
    server_context: Arc<ServerContext>,
    admin: &AdminAccount,
    request: RemoveUserRequest,
) -> Result<RemoveUserResponse, AuthError> {
    let admin = admin.user();
    let target = other_user_id(admin, &request.user_id, REMOVE_SELF)?;

    block_on(async move {
        let conn = open_main_database_write(server_context.clone()).await?;
        let removed = conn
            .in_transaction(async |conn| {
                lock_account_as_admin(conn, &admin.account_id, &admin.user_id).await?;
                let deleted = conn
                    .execute(
                        "DELETE FROM users
                         WHERE id = $1::text::uuid AND account_id = $2::text::uuid",
                        &[&target, &admin.account_id],
                    )
                    .await
                    .map_err(map_query_error)?;
                if deleted == 0 {
                    return Err(AuthError::NotFound("no such user on this account"));
                }
                Ok(())
            })
            .await;
        drop(conn);
        forget_user(&server_context, &target);
        removed.map(|()| RemoveUserResponse { ok: true })
    })
}
