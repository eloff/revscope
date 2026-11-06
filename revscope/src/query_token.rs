//! Project-scoped tokens for the query and schema APIs.
//!
//! The MCP server runs outside the browser, has no session cookie, and is
//! configured once and left alone, so the dashboard trades its session for a
//! token bound to a single project. The token grants nothing but the read-only
//! `/v2/query/projects/{id}/sql` and `/v2/query/projects/{id}/schema`
//! endpoints for that project.
//!
//! Tokens are rows in `api_tokens` (see the migration): they have to survive a
//! restart, unlike the in-memory download tokens. Only their hash is stored,
//! so the plaintext exists in the mint response and nowhere else.

use std::sync::Arc;

use serde::Serialize;
use tokio_postgres::types::ToSql;

use crate::auth::{self, AuthError, SessionCookie};
use crate::authorize::{Account, OwnedProject, ensure_project_access};
use crate::db::{open_main_database_read, open_main_database_write, parse_project_id};
use crate::server_context::ServerContext;
use crate::tokens::{QUERY_TOKEN_TTL_SECONDS, TokenSecret};
use crate::user_access::lock_user_project_access;
use crate::util::{block_on, seconds_since_epoch};

/// Marks the token as ours in an `Authorization: Bearer` header, which
/// otherwise carries a session id.
const QUERY_TOKEN_PREFIX: &str = "rsq_";

/// How long a verified token is trusted without asking Postgres again. Deleting
/// a token therefore takes up to this long to lock the holder out.
const VERIFIED_TOKEN_CACHE_TTL_SECONDS: u32 = 5 * 60;

/// How the caller proved it may read a project.
pub(crate) enum ProjectAuth {
    /// A logged-in dashboard user, who may read the projects on their account
    /// their role allows.
    Session(SessionCookie),
    /// A token, which may read the one project it was minted for.
    QueryToken(TokenSecret),
}

#[derive(Serialize)]
pub(crate) struct QueryTokenResponse {
    /// The only time the plaintext token is available.
    token: String,
    project_id: String,
    /// Seconds since the unix epoch.
    expires_at: i64,
}

/// Recognizes a query token in an `Authorization` header. Session ids are UUIDs
/// and never carry the prefix, so the two cannot be confused.
pub(crate) fn parse_authorization(authorization_header: Option<&str>) -> Option<TokenSecret> {
    let header = authorization_header?.trim();
    let value = header.strip_prefix("Bearer ").unwrap_or(header).trim();
    TokenSecret::parse(value.strip_prefix(QUERY_TOKEN_PREFIX)?)
}

/// Exchanges an authenticated session for a token scoped to one project.
pub(crate) fn create_query_token(
    server_context: Arc<ServerContext>,
    account: &Account,
    project: &OwnedProject,
) -> Result<QueryTokenResponse, AuthError> {
    let user = account.user();
    let secret = TokenSecret::generate()?;
    let expires_at = block_on(insert_query_token(
        server_context,
        &user.user_id,
        &user.account_id,
        project.key(),
        &secret,
    ))?;

    Ok(QueryTokenResponse {
        token: format!("{QUERY_TOKEN_PREFIX}{secret}"),
        project_id: project.id().to_string(),
        expires_at,
    })
}

/// Confirms the caller may read `project_id`, whichever way it authenticated.
pub(crate) fn authorize_project(
    server_context: Arc<ServerContext>,
    auth: &ProjectAuth,
    project_id: &str,
) -> Result<(), AuthError> {
    match auth {
        ProjectAuth::Session(session) => {
            let user = block_on(auth::session_user(server_context.clone(), session))?;
            ensure_project_access(server_context, &user, project_id)
        }
        ProjectAuth::QueryToken(secret) => {
            let requested =
                parse_project_id(project_id).map_err(|error| AuthError::Db(error.to_string()))?;
            let granted = project_key_for_token(server_context.clone(), *secret)?;
            if granted != requested {
                return Err(AuthError::InvalidCredentials);
            }
            crate::projects::fetch_project_blocking(server_context, requested)
                .map_err(|error| AuthError::Db(error.to_string()))?
                .ok_or(AuthError::InvalidCredentials)?;
            Ok(())
        }
    }
}

/// The project a live token grants. Verified tokens are cached briefly so a
/// busy agent does not hit Postgres on every query.
fn project_key_for_token(
    server_context: Arc<ServerContext>,
    secret: TokenSecret,
) -> Result<i32, AuthError> {
    if let Some(project_key) = server_context.query_token_cache.get(secret) {
        return Ok(project_key);
    }

    let live = block_on(select_query_token(server_context.clone(), &secret))?
        .ok_or(AuthError::InvalidSession)?;
    // Never outlast the token itself: one that expires in a minute must not be
    // cached for five.
    let remaining = u32::try_from(
        live.expires_at
            .saturating_sub(i64::from(seconds_since_epoch())),
    )
    .unwrap_or(0);
    server_context.query_token_cache.insert(
        secret,
        live.project_key,
        seconds_since_epoch().saturating_add(remaining.min(VERIFIED_TOKEN_CACHE_TTL_SECONDS)),
    );
    Ok(live.project_key)
}

struct LiveToken {
    project_key: i32,
    /// Seconds since the unix epoch.
    expires_at: i64,
}

async fn insert_query_token(
    server_context: Arc<ServerContext>,
    user_id: &str,
    account_id: &str,
    project_key: i32,
    secret: &TokenSecret,
) -> Result<i64, AuthError> {
    let conn = open_main_database_write(server_context).await?;
    // The session that authorized this may be cached from before an admin
    // narrowed the user's projects, and the token lasts far longer.
    conn.in_transaction(async |conn| {
        lock_user_project_access(conn, user_id, account_id, Some(project_key)).await?;
        let row = conn
            .query_one(
                // account_id is redundant with the project's own, but every token
                // records the account it belongs to; only the project half is
                // optional (see the migration).
                "INSERT INTO api_tokens (token_hash, kind, user_id, account_id, project_id, expires_at)
                 -- Without the cast Postgres infers a float8 parameter, because
                 -- that is the preferred operand type for multiplying an interval.
                 -- trunc: EXTRACT returns float8 and ::bigint would round, which
                 -- can report TTL+1 when NOW()'s fractional second is ≥ 0.5.
                 VALUES (
                     $1, 'query', $2::text::uuid, $3::text::uuid, $4,
                     NOW() + ($5::int * INTERVAL '1 second')
                 )
                 RETURNING trunc(EXTRACT(EPOCH FROM expires_at))::bigint",
                &[
                    &secret.hash() as &(dyn ToSql + Sync),
                    &user_id,
                    &account_id,
                    &project_key,
                    &QUERY_TOKEN_TTL_SECONDS,
                ],
            )
            .await
            .map_err(AuthError::from)?;
        Ok(row.get(0))
    })
    .await
}

async fn select_query_token(
    server_context: Arc<ServerContext>,
    secret: &TokenSecret,
) -> Result<Option<LiveToken>, AuthError> {
    let conn = open_main_database_read(server_context).await?;
    let row = conn
        .query_opt(
            "SELECT project_id, trunc(EXTRACT(EPOCH FROM expires_at))::bigint
             FROM api_tokens
             WHERE token_hash = $1
             AND kind = 'query'
             -- Nullable since MCP tokens landed; a query token without one
             -- grants nothing rather than every project.
             AND project_id IS NOT NULL
             AND expires_at > NOW()
             LIMIT 1",
            &[&secret.hash()],
        )
        .await
        .map_err(AuthError::from)?;
    Ok(row.map(|row| LiveToken {
        project_key: row.get(0),
        expires_at: row.get(1),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prefixed_bearer_token_is_recognized_with_or_without_the_scheme() {
        let secret = TokenSecret::generate().expect("generate");
        let token = format!("{QUERY_TOKEN_PREFIX}{secret}");
        assert_eq!(
            parse_authorization(Some(&format!("Bearer {token}"))),
            Some(secret)
        );
        assert_eq!(parse_authorization(Some(&token)), Some(secret));
    }

    #[test]
    fn a_session_id_is_not_mistaken_for_a_query_token() {
        assert_eq!(parse_authorization(None), None);
        assert_eq!(
            parse_authorization(Some("Bearer 0198f0a1-2b3c-4d4e-8f90-1a2b3c4d5e6f")),
            None
        );
        assert_eq!(parse_authorization(Some("Bearer rsq_not-hex")), None);
    }
}
