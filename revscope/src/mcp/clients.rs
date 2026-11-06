//! OAuth clients that registered themselves through RFC 7591.
//!
//! They are public clients: there is no secret to check, so the only thing
//! binding an authorization code to the client that asked for it is the exact
//! redirect URI it registered, plus the PKCE challenge it sent.

use std::sync::Arc;
use std::time::Duration;

use url::Url;

use crate::auth::AuthError;
use crate::db::{open_main_database_read, open_main_database_write};
use crate::server_context::ServerContext;
use crate::util::truncate_chars;
use crate::uuid_text::parse_hyphenated_uuid;

/// Enough for a client that lists a callback per environment, few enough that a
/// registration cannot be used as storage.
const MAX_REDIRECT_URIS: usize = 10;
const MAX_REDIRECT_URI_LEN: usize = 2048;
const MAX_CLIENT_NAME_LEN: usize = 200;

/// Registration is public, so a client that has not redeemed a code by now is
/// deleted. Authorization codes live for minutes, so this is generous.
const UNUSED_CLIENT_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// Schemes that would run in our own origin if the consent page navigated to
/// them. Editors and desktop apps legitimately register their own scheme, so
/// the check is a denylist of the dangerous ones rather than an allowlist.
const FORBIDDEN_REDIRECT_SCHEMES: [&str; 6] =
    ["javascript", "data", "vbscript", "blob", "file", "about"];

pub(crate) struct RegisteredClient {
    pub(crate) client_id: String,
    pub(crate) client_name: String,
    pub(crate) redirect_uris: Vec<String>,
}

impl RegisteredClient {
    pub(crate) fn allows_redirect_uri(&self, redirect_uri: &str) -> bool {
        self.redirect_uris
            .iter()
            .any(|registered| registered == redirect_uri)
    }
}

/// Rejects a redirect URI a browser must never be sent to, and anything that is
/// not a usable absolute URI. Fragments are disallowed by RFC 6749 section 3.1.2
/// because the authorization response appends its own query.
pub(crate) fn validate_redirect_uri(redirect_uri: &str) -> Result<(), String> {
    if redirect_uri.is_empty() || redirect_uri.len() > MAX_REDIRECT_URI_LEN {
        return Err(format!(
            "redirect_uris entries must be 1 to {MAX_REDIRECT_URI_LEN} characters"
        ));
    }
    let parsed =
        Url::parse(redirect_uri).map_err(|_| format!("{redirect_uri} is not an absolute URI"))?;
    if parsed.fragment().is_some() {
        return Err(format!("{redirect_uri} must not contain a fragment"));
    }
    let scheme = parsed.scheme().to_ascii_lowercase();
    if FORBIDDEN_REDIRECT_SCHEMES.contains(&scheme.as_str()) {
        return Err(format!("{scheme} redirect URIs are not accepted"));
    }
    // Plain http is only safe to redirect to when it stays on the user's machine.
    if scheme == "http" && !is_loopback_url(&parsed) {
        return Err(format!(
            "{redirect_uri} must use https unless it points at loopback"
        ));
    }
    Ok(())
}

/// Whether a redirect stays on the machine that asked for it.
pub(crate) fn is_loopback_redirect(redirect_uri: &str) -> bool {
    Url::parse(redirect_uri).is_ok_and(|url| url.scheme() == "http" && is_loopback_url(&url))
}

fn is_loopback_url(url: &Url) -> bool {
    match url.host_str() {
        Some(host) => {
            host.eq_ignore_ascii_case("localhost")
                || host
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        }
        None => false,
    }
}

/// Validates a registration request and stores it, returning the new client id.
pub(crate) async fn register_client(
    server_context: Arc<ServerContext>,
    client_name: &str,
    redirect_uris: &[String],
) -> Result<RegisteredClient, AuthError> {
    if redirect_uris.is_empty() || redirect_uris.len() > MAX_REDIRECT_URIS {
        return Err(AuthError::InvalidRequest(
            "redirect_uris must list between 1 and 10 URIs",
        ));
    }
    for redirect_uri in redirect_uris {
        validate_redirect_uri(redirect_uri).map_err(|_| {
            AuthError::InvalidRequest("redirect_uris contains a URI we cannot redirect to")
        })?;
    }
    let client_name = match truncate_chars(client_name.trim(), MAX_CLIENT_NAME_LEN) {
        "" => "Unnamed MCP client".to_string(),
        client_name => client_name.to_string(),
    };

    let conn = open_main_database_write(server_context).await?;
    let row = conn
        .query_one(
            "INSERT INTO oauth_clients (client_name, redirect_uris)
             VALUES ($1, $2)
             RETURNING id::text",
            &[&client_name, &redirect_uris],
        )
        .await
        .map_err(AuthError::from)?;

    Ok(RegisteredClient {
        client_id: row.get(0),
        client_name,
        redirect_uris: redirect_uris.to_vec(),
    })
}

/// Deletes registrations older than [`UNUSED_CLIENT_MAX_AGE`] that were never
/// issued a token, as recorded by [`crate::mcp::token::issue_token`].
pub(crate) async fn delete_unused_clients(
    server_context: Arc<ServerContext>,
) -> Result<u64, AuthError> {
    let conn = open_main_database_write(server_context).await?;
    conn.execute(
        "DELETE FROM oauth_clients
         WHERE token_issued_at IS NULL
         AND created_at < NOW() - ($1::int * INTERVAL '1 second')",
        &[&(UNUSED_CLIENT_MAX_AGE.as_secs() as i32)],
    )
    .await
    .map_err(AuthError::from)
}

/// Looks a client up by the id we handed out at registration. An unparseable id
/// is simply an unknown client, not a database error.
pub(crate) async fn find_client(
    server_context: Arc<ServerContext>,
    client_id: &str,
) -> Result<Option<RegisteredClient>, AuthError> {
    if parse_hyphenated_uuid(client_id).is_none() {
        return Ok(None);
    }
    let conn = open_main_database_read(server_context).await?;
    let row = conn
        .query_opt(
            "SELECT id::text, client_name, redirect_uris
             FROM oauth_clients
             WHERE id = $1::text::uuid",
            &[&client_id],
        )
        .await
        .map_err(AuthError::from)?;
    Ok(row.map(|row| RegisteredClient {
        client_id: row.get(0),
        client_name: row.get(1),
        redirect_uris: row.get(2),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_the_redirect_uris_real_clients_register() {
        assert!(validate_redirect_uri("https://claude.ai/api/mcp/auth_callback").is_ok());
        assert!(validate_redirect_uri("http://127.0.0.1:33418/oauth/callback").is_ok());
        assert!(validate_redirect_uri("http://localhost:6274/callback").is_ok());
        assert!(validate_redirect_uri("cursor://anysphere.cursor-retrieval/oauth").is_ok());
        assert!(validate_redirect_uri("vscode://revscope.mcp/did-authenticate").is_ok());
    }

    #[test]
    fn rejects_redirects_a_browser_must_not_follow() {
        assert!(validate_redirect_uri("javascript:alert(1)").is_err());
        assert!(validate_redirect_uri("data:text/html,<script>").is_err());
        assert!(validate_redirect_uri("http://evil.example.com/callback").is_err());
        assert!(validate_redirect_uri("https://example.com/cb#frag").is_err());
        assert!(validate_redirect_uri("/relative/callback").is_err());
        assert!(validate_redirect_uri("").is_err());
        assert!(validate_redirect_uri(&"https://example.com/".repeat(200)).is_err());
    }

    #[test]
    fn a_client_only_allows_the_exact_uris_it_registered() {
        let client = RegisteredClient {
            client_id: "id".to_string(),
            client_name: "Test".to_string(),
            redirect_uris: vec!["https://claude.ai/api/mcp/auth_callback".to_string()],
        };
        assert!(client.allows_redirect_uri("https://claude.ai/api/mcp/auth_callback"));
        assert!(!client.allows_redirect_uri("https://claude.ai/api/mcp/auth_callback?x=1"));
        assert!(!client.allows_redirect_uri("https://evil.example.com/cb"));
    }
}
