//! Invites someone into the signed-in admin's account, as an admin or with
//! read access to every project or to the ones picked.
//!
//! An invite is a user without a password plus a password reset token: the
//! invitee follows the link, chooses a password, and logs in. Inviting a
//! pending user again replaces their link and their access. The link is
//! emailed when MailChannels is configured, and always returned so the
//! inviter can share it.

use std::sync::Arc;

use log::error;
use serde::{Deserialize, Serialize};

use crate::auth::{
    AuthError, MIN_USER_NAME_LENGTH, insert_password_reset_token, insert_user, map_query_error,
    normalize_email, password_reset_url,
};
use crate::authorize::AdminAccount;
use crate::db::open_main_database_write;
use crate::email::{EmailError, send_invite_email};
use crate::pending_users::pending_user_sql;
use crate::server_context::ServerContext;
use crate::tokens::INVITE_TTL_SECONDS;
use crate::user_access::{AccessRequest, UserAccess, lock_account_as_admin, write_user_access};
use crate::util::block_on;

#[derive(Debug, Deserialize)]
pub(crate) struct InviteUserRequest {
    pub(crate) name: String,
    pub(crate) email: String,
    #[serde(flatten)]
    pub(crate) access: AccessRequest,
}

#[derive(Debug, Serialize)]
pub(crate) struct InviteUserResponse {
    invite_url: String,
    /// False when email isn't configured or sending failed; share the link
    /// some other way.
    emailed: bool,
}

pub(crate) fn invite_user(
    server_context: Arc<ServerContext>,
    admin: &AdminAccount,
    request: InviteUserRequest,
) -> Result<InviteUserResponse, AuthError> {
    let inviter = admin.user();
    let name = request.name.trim();
    if name.len() < MIN_USER_NAME_LENGTH {
        return Err(AuthError::InvalidRequest(
            "name must be at least 2 characters",
        ));
    }
    let email = normalize_email(request.email.as_str())?;
    let access = UserAccess::from_request(request.access)?;

    block_on(async move {
        let conn = open_main_database_write(server_context.clone()).await?;
        let invited = conn
            .in_transaction(async |conn| {
                lock_account_as_admin(conn, &inviter.account_id, &inviter.user_id).await?;
                // Locked so a pending user can't accept an earlier invite, or
                // link a Google or Apple sign-in, and so start minting tokens,
                // while this one changes their access. If they already did,
                // this reads them as no longer pending. Google and Apple users
                // have no password either, but have accepted: a new reset link
                // would hand the inviter their account.
                let existing = conn
                    .query_opt(
                        &format!(
                            "SELECT u.id::text, u.account_id::text, {}
                             FROM users u
                             WHERE u.email = $1
                             FOR UPDATE",
                            pending_user_sql("u")
                        ),
                        &[&email.normalized],
                    )
                    .await
                    .map_err(map_query_error)?;
                let user_id = match existing {
                    Some(row) => {
                        let account_id: String = row.get(1);
                        let pending: bool = row.get(2);
                        if account_id != inviter.account_id || !pending {
                            return Err(AuthError::Conflict(
                                "This person already has an account. They can log in.",
                            ));
                        }
                        row.get::<_, String>(0)
                    }
                    None => {
                        insert_user(conn, &inviter.account_id, name, &email, None, false).await?
                    }
                };
                write_user_access(conn, &inviter.account_id, &user_id, &access).await?;
                conn.execute(
                    "UPDATE users SET invited_at = NOW() WHERE id = $1::text::uuid",
                    &[&user_id],
                )
                .await
                .map_err(map_query_error)?;

                conn.execute(
                    "DELETE FROM password_reset_tokens WHERE user_id = $1::text::uuid",
                    &[&user_id],
                )
                .await
                .map_err(map_query_error)?;
                let token = insert_password_reset_token(conn, &user_id, INVITE_TTL_SECONDS).await?;
                Ok((user_id, token))
            })
            .await;
        drop(conn);
        // Only users still pending under the lock above are invited, and they
        // have no sessions or tokens for a change of access to strand.
        let (_user_id, token) = invited?;

        let invite_url = format!("{}&invite=1", password_reset_url(&token));
        let to_email = email.original.as_deref().unwrap_or(&email.normalized);
        let emailed = match send_invite_email(
            &server_context.http,
            to_email,
            name,
            &inviter.name,
            &invite_url,
        )
        .await
        {
            Ok(()) => true,
            Err(EmailError::MissingEnv(_)) => false,
            Err(send_error) => {
                error!("failed to send invite email: {send_error}");
                false
            }
        };
        Ok(InviteUserResponse {
            invite_url,
            emailed,
        })
    })
}
