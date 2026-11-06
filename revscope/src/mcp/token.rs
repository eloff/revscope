//! Access tokens for the hosted MCP server.
//!
//! Same storage as a query token — a row in `api_tokens` holding only the
//! sha256 of the secret — but a different `kind`, a year-long lifetime, and a
//! different way in: an agent gets one by walking its user through the OAuth
//! consent page rather than by copying it out of the dashboard.
//!
//! It is also scoped to an account rather than a project, so the agent can move
//! between projects (and reach ones created later) without a second consent.

use std::sync::Arc;

use tokio_postgres::types::ToSql;

use crate::auth::AuthError;
use crate::db::{open_main_database_read, open_main_database_write};
use crate::server_context::ServerContext;
use crate::tokens::{MCP_TOKEN_TTL_SECONDS, TokenSecret};
use crate::user_access::lock_user_project_access;
use crate::util::seconds_since_epoch;

/// Marks the token as ours in an `Authorization: Bearer` header, which
/// otherwise carries a session id or a query token.
const MCP_TOKEN_PREFIX: &str = "rsm_";

/// How long a verified token is trusted without asking Postgres again, matching
/// query tokens. Deleting a token locks the holder out after at most this long.
const VERIFIED_TOKEN_CACHE_TTL_SECONDS: u32 = 5 * 60;

/// Recognizes an MCP token in an `Authorization` header. Session ids are UUIDs
/// and query tokens carry a different prefix, so the three cannot be confused.
pub(crate) fn parse_authorization(authorization_header: Option<&str>) -> Option<TokenSecret> {
    let header = authorization_header?.trim();
    let value = header.strip_prefix("Bearer ").unwrap_or(header).trim();
    TokenSecret::parse(value.strip_prefix(MCP_TOKEN_PREFIX)?)
}

/// The account a live token speaks for. Every project on it is readable and
/// nothing else is, so this is the whole of what the token proves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct McpAccount(String);

impl McpAccount {
    /// The account's UUID in canonical text form, as Postgres rendered it.
    pub(crate) fn id(&self) -> &str {
        &self.0
    }

    /// Stands in for a token nobody minted. Test-only so that outside tests the
    /// only way to hold one is to have had a live row in `api_tokens`.
    #[cfg(test)]
    pub(crate) fn for_tests(account_id: &str) -> Self {
        Self(account_id.to_string())
    }
}

/// Mints a token for `account_id`, owned by `user_id`, on behalf of the
/// registered client `client_id`, and returns the plaintext, which is not
/// recoverable afterwards. `None` when the client's registration no longer
/// exists. Refused with [`AuthError::InvalidCredentials`] unless the user is
/// still an admin on the account, since another admin may have demoted or
/// removed them after they approved.
pub(crate) async fn issue_token(
    server_context: Arc<ServerContext>,
    user_id: &str,
    account_id: &str,
    client_id: &str,
) -> Result<Option<String>, AuthError> {
    let secret = TokenSecret::generate()?;
    let conn = open_main_database_write(server_context).await?;
    let inserted = conn
        .in_transaction(async |conn| {
            lock_user_project_access(conn, user_id, account_id, None).await?;
            conn.execute(
                // One statement, so the client is marked as issued, which exempts it
                // from `delete_unused_clients`, exactly when the token is written.
                // project_id is left NULL, which is what makes the token
                // account-wide. The interval parameter is cast for the same reason
                // as the query token insert: uncast, Postgres infers float8 and the
                // multiply fails.
                "WITH client AS (
                     UPDATE oauth_clients SET token_issued_at = coalesce(token_issued_at, NOW())
                     WHERE id = $5::text::uuid
                     RETURNING id
                 )
                 INSERT INTO api_tokens (token_hash, kind, user_id, account_id, expires_at)
                 SELECT $1, 'mcp', $2::text::uuid, $3::text::uuid,
                        NOW() + ($4::int * INTERVAL '1 second')
                 FROM client",
                &[
                    &secret.hash() as &(dyn ToSql + Sync),
                    &user_id,
                    &account_id,
                    &MCP_TOKEN_TTL_SECONDS,
                    &client_id,
                ],
            )
            .await
            .map_err(AuthError::from)
        })
        .await?;

    Ok((inserted == 1).then(|| format!("{MCP_TOKEN_PREFIX}{secret}")))
}

/// Deletes every MCP token on `account_id` and forgets them in this process's
/// cache. Other processes may keep serving a just-deleted token for up to
/// [`VERIFIED_TOKEN_CACHE_TTL_SECONDS`].
pub(crate) async fn revoke_account_tokens(
    server_context: Arc<ServerContext>,
    account_id: &str,
) -> Result<u64, AuthError> {
    let conn = open_main_database_write(server_context.clone()).await?;
    let revoked = conn
        .execute(
            "DELETE FROM api_tokens
             WHERE account_id = $1::text::uuid
             AND kind = 'mcp'",
            &[&account_id],
        )
        .await
        .map_err(AuthError::from)?;
    server_context
        .mcp_token_cache
        .retain(|account| account.id() != account_id);
    Ok(revoked)
}

/// The account a live token grants, or [`AuthError::InvalidSession`] when the
/// token is unknown or expired. Deleting the account cascades to its tokens, so
/// a row that is still here is one whose account is too.
pub(crate) async fn authorize_token(
    server_context: Arc<ServerContext>,
    secret: TokenSecret,
) -> Result<McpAccount, AuthError> {
    if let Some(account) = server_context.mcp_token_cache.get(secret) {
        return Ok(account);
    }

    let live = select_token(server_context.clone(), &secret)
        .await?
        .ok_or(AuthError::InvalidSession)?;
    // Never outlast the token itself: one that expires in a minute must not be
    // cached for five.
    let remaining = u32::try_from(
        live.expires_at
            .saturating_sub(i64::from(seconds_since_epoch())),
    )
    .unwrap_or(0);
    let account = McpAccount(live.account_id);
    server_context.mcp_token_cache.insert(
        secret,
        account.clone(),
        seconds_since_epoch().saturating_add(remaining.min(VERIFIED_TOKEN_CACHE_TTL_SECONDS)),
    );
    Ok(account)
}

struct LiveToken {
    account_id: String,
    /// Seconds since the unix epoch.
    expires_at: i64,
}

async fn select_token(
    server_context: Arc<ServerContext>,
    secret: &TokenSecret,
) -> Result<Option<LiveToken>, AuthError> {
    let conn = open_main_database_read(server_context).await?;
    let row = conn
        .query_opt(
            "SELECT account_id::text, trunc(EXTRACT(EPOCH FROM expires_at))::bigint
             FROM api_tokens
             WHERE token_hash = $1
             AND kind = 'mcp'
             -- The column is nullable for the table as a whole, so the filter
             -- is what lets the row be read as a non-null account below.
             AND account_id IS NOT NULL
             AND expires_at > NOW()
             LIMIT 1",
            &[&secret.hash()],
        )
        .await
        .map_err(AuthError::from)?;
    Ok(row.map(|row| LiveToken {
        account_id: row.get(0),
        expires_at: row.get(1),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prefixed_bearer_token_is_recognized_with_or_without_the_scheme() {
        let secret = TokenSecret::generate().expect("generate");
        let token = format!("{MCP_TOKEN_PREFIX}{secret}");
        assert_eq!(
            parse_authorization(Some(&format!("Bearer {token}"))),
            Some(secret)
        );
        assert_eq!(parse_authorization(Some(&token)), Some(secret));
    }

    #[test]
    fn other_credentials_are_not_mistaken_for_an_mcp_token() {
        assert_eq!(parse_authorization(None), None);
        assert_eq!(
            parse_authorization(Some("Bearer 0198f0a1-2b3c-7d4e-8f90-1a2b3c4d5e6f")),
            None
        );
        assert_eq!(
            parse_authorization(Some(&format!("Bearer rsq_{}", "a".repeat(32)))),
            None
        );
        assert_eq!(parse_authorization(Some("Bearer rsm_not-hex")), None);
    }
}
