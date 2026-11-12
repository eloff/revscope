//! Confirming an account's email address, to keep free accounts from being
//! made in bulk. A password signup proves nothing about its address, so hosted
//! signup emails a link, and the features that cost us money or reach other
//! services wait until someone on the account has followed one: Ask AI, GA
//! imports and integrations. A Google or Apple sign-in, or a password reset,
//! counts too. A self-hosted build doesn't verify, since it may have no
//! outgoing email.
//!
//! Following the link doesn't let a Google or Apple sign-in join the user, as a
//! reset link does: it leaves whoever made the user with their password and
//! tokens, so it is recorded apart, as `email_confirmed_at`.

use std::sync::Arc;

use log::{error, info};
use serde::Deserialize;

use crate::auth::{AuthError, SessionUser, map_query_error};
use crate::db::{open_main_database_read, open_main_database_write};
use crate::email::EmailError;
use crate::server_context::ServerContext;
use crate::tokens::EMAIL_VERIFICATION_TTL_SECONDS;
use crate::uuid_token::UuidToken;

pub(crate) const EMAIL_UNVERIFIED: &str = "Confirm your email address to use this: send \
     yourself a link from the banner at the top of the dashboard, or reset your password, \
     which confirms it too.";

const LINK_INVALID: &str = "This confirmation link is invalid or has expired. Send a new one \
     from the banner at the top of the dashboard, or reset your password, which confirms it too.";

#[derive(Debug, Deserialize)]
pub(crate) struct VerifyEmailRequest {
    pub(crate) token: String,
}

pub(crate) fn require_verified(user: &SessionUser) -> Result<(), AuthError> {
    if user.account_confirmed {
        Ok(())
    } else {
        Err(AuthError::EmailUnverified)
    }
}

/// Emails a confirmation link to the user with `normalized_email` if they
/// haven't confirmed it yet. Sent in the background, as a password reset is, so
/// the caller's answer doesn't wait on the mail provider.
pub(crate) fn send_verification(server_context: Arc<ServerContext>, normalized_email: String) {
    if crate::self_hosted::ENABLED {
        return;
    }
    crate::task::spawn(server_context.clone(), async move {
        if let Err(error) = send(server_context, &normalized_email).await {
            crate::sentry_report::report_error!(
                "auth.verify_email",
                "failed to send an email verification: {error:?}"
            );
        }
    });
}

async fn send(server_context: Arc<ServerContext>, normalized_email: &str) -> Result<(), AuthError> {
    let conn = open_main_database_read(server_context.clone()).await?;
    let row = conn
        .query_opt(
            "SELECT id::text, name, COALESCE(original_email, email)
             FROM users
             WHERE email = $1 AND email_confirmed_at IS NULL AND email_verified_at IS NULL",
            &[&normalized_email],
        )
        .await
        .map_err(map_query_error)?;
    drop(conn);
    let Some(row) = row else {
        return Ok(());
    };
    let (user_id, name, email): (String, String, String) = (row.get(0), row.get(1), row.get(2));

    let token = UuidToken::generate()?;
    let conn = open_main_database_write(server_context.clone()).await?;
    conn.execute(
        "INSERT INTO email_verification_tokens (token_hash, user_id, expires_at)
         VALUES ($1, $2::text::uuid, NOW() + ($3::int * INTERVAL '1 second'))",
        &[&token.hash(), &user_id, &EMAIL_VERIFICATION_TTL_SECONDS],
    )
    .await
    .map_err(map_query_error)?;
    drop(conn);

    let url = format!("{}/verify-email?token={token}", crate::util::web_base_url());
    if let Err(send_error) =
        crate::email::send_verification_email(&server_context.http, &email, &name, &url).await
    {
        // Unconfigured outside production, where nobody is waiting for it.
        if matches!(send_error, EmailError::MissingEnv(_)) {
            info!("not sending a verification email: {send_error}");
        } else {
            error!("failed to send verification email: {send_error}");
        }
        let conn = open_main_database_write(server_context).await?;
        conn.execute(
            "DELETE FROM email_verification_tokens WHERE token_hash = $1",
            &[&token.hash()],
        )
        .await
        .map_err(map_query_error)?;
    }
    Ok(())
}

/// Spends a link from [`send_verification`], confirming its user's address
/// and so their account. A link stays good until it expires or its user is
/// confirmed, so an older email still works after a newer one was sent.
pub(crate) async fn verify_email(
    server_context: Arc<ServerContext>,
    token: &str,
) -> Result<(), AuthError> {
    let token = UuidToken::parse(token.trim()).ok_or(AuthError::InvalidRequest(LINK_INVALID))?;
    let conn = open_main_database_write(server_context.clone()).await?;
    // Read first, so the account's cached sessions can be dropped even if the
    // update's answer is lost after it commits.
    let account_id: String = conn
        .query_opt(
            "SELECT u.account_id::text
             FROM email_verification_tokens t
             INNER JOIN users u ON u.id = t.user_id
             WHERE t.token_hash = $1 AND t.expires_at > NOW()",
            &[&token.hash()],
        )
        .await
        .map_err(map_query_error)?
        .ok_or(AuthError::InvalidRequest(LINK_INVALID))?
        .get(0);
    let confirmed = conn
        .query_opt(
            "WITH confirmed AS (
                UPDATE users
                SET email_confirmed_at = COALESCE(email_confirmed_at, NOW())
                WHERE id = (
                    SELECT user_id FROM email_verification_tokens
                    WHERE token_hash = $1 AND expires_at > NOW()
                )
                RETURNING id
             ),
             spent AS (
                DELETE FROM email_verification_tokens
                WHERE user_id IN (SELECT id FROM confirmed)
             )
             SELECT 1 FROM confirmed",
            &[&token.hash()],
        )
        .await;
    drop(conn);
    // Every session on the account caches whether it is confirmed.
    server_context.session_cache.invalidate_account(&account_id);
    confirmed
        .map_err(map_query_error)?
        .ok_or(AuthError::InvalidRequest(LINK_INVALID))?;
    Ok(())
}
