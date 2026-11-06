//! Changes another user's role or projects on the signed-in admin's account.
//!
//! Admins cannot change their own access, so an account always keeps the
//! admin making the change; see [`lock_account_as_admin`] for two admins
//! demoting each other at once.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::auth::{self, AuthError};
use crate::authorize::AdminAccount;
use crate::db::open_main_database_write;
use crate::server_context::ServerContext;
use crate::user_access::{
    AccessRequest, UserAccess, forget_user, lock_account_as_admin, revoke_user_tokens,
    write_user_access,
};
use crate::util::block_on;
use crate::uuid_text::{format_uuid, parse_hyphenated_uuid};

const CHANGE_OWN_ACCESS: &str = "You can't change your own access. Ask another admin.";

#[derive(Debug, Deserialize)]
pub(crate) struct UpdateUserRequest {
    pub(crate) user_id: String,
    #[serde(flatten)]
    pub(crate) access: AccessRequest,
}

#[derive(Debug, Serialize)]
pub(crate) struct UpdateUserResponse {
    ok: bool,
}

pub(crate) fn update_user(
    server_context: Arc<ServerContext>,
    admin: &AdminAccount,
    request: UpdateUserRequest,
) -> Result<UpdateUserResponse, AuthError> {
    let admin = admin.user();
    let target = other_user_id(admin, &request.user_id, CHANGE_OWN_ACCESS)?;
    let access = UserAccess::from_request(request.access)?;

    block_on(async move {
        let conn = open_main_database_write(server_context.clone()).await?;
        let updated = conn
            .in_transaction(async |conn| {
                lock_account_as_admin(conn, &admin.account_id, &admin.user_id).await?;
                write_user_access(conn, &admin.account_id, &target, &access).await?;
                revoke_user_tokens(conn, &target, &access).await
            })
            .await;
        drop(conn);
        forget_user(&server_context, &target);
        updated.map(|()| UpdateUserResponse { ok: true })
    })
}

/// `user_id` hyphenated as Postgres renders it, which is how the session cache
/// names users, or `self_message` when it names `admin`. Anything that cannot
/// be one of our ids is reported as not found, like a user on another account.
pub(crate) fn other_user_id(
    admin: &auth::SessionUser,
    user_id: &str,
    self_message: &'static str,
) -> Result<String, AuthError> {
    let key = parse_hyphenated_uuid(user_id)
        .ok_or(AuthError::NotFound("no such user on this account"))?;
    if parse_hyphenated_uuid(&admin.user_id) == Some(key) {
        return Err(AuthError::Forbidden(self_message));
    }
    Ok(format_uuid(key))
}
