//! `/v2/session/mcp/authorize` — the consent step, authenticated by the
//! dashboard session.
//!
//! The authorization endpoint a client is told to open is a page in the web app,
//! not an API route: only a browser carrying our session cookie can approve. The
//! page calls this route, which validates the request against the registered
//! client, mints a single-use code, and hands back the URL to send the browser
//! to.
//!
//! What is approved is read access to the whole account. There is no project to
//! pick, because the token the code buys reaches every project on the account,
//! including ones created after the fact. The CLI asks for [`CLI_SCOPE`] instead,
//! which is everything the approving user can do.
//!
//! Building that URL here rather than in the page is deliberate: the redirect
//! target comes from an unauthenticated registration, so it is checked against
//! the client's registered URIs before any browser is pointed at it.

use std::sync::Arc;

use revscope_api::oauth::CLI_SCOPE;
use serde::{Deserialize, Serialize};

use crate::authorize::{Account, authorize_admin};
use crate::http::Response;
use crate::mcp::{clients, invalid_request, oauth_error, oauth_json};
use crate::routes::map_auth_error;
use crate::server_context::ServerContext;
use crate::tokens::MCP_AUTHORIZATION_CODE_TTL_SECONDS;
use crate::util::block_on;

/// PKCE challenges are the base64url of a sha256, but RFC 7636 allows the whole
/// 43 to 128 character verifier range, so accept that and compare later.
const MIN_CODE_CHALLENGE_LEN: usize = 43;
const MAX_CODE_CHALLENGE_LEN: usize = 128;
/// Opaque to us and echoed back untouched, so it only needs a sane ceiling.
const MAX_STATE_LEN: usize = 512;

/// What a redeemed authorization code stands for. Held in memory only: a
/// restart during the seconds between approval and exchange just means the
/// client starts the flow again.
#[derive(Clone)]
pub(crate) struct AuthorizationGrant {
    pub(crate) client_id: String,
    pub(crate) redirect_uri: String,
    pub(crate) code_challenge: String,
    pub(crate) account_id: String,
    pub(crate) user_id: String,
    pub(crate) scope: GrantScope,
}

/// What the token an authorization code buys may do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GrantScope {
    /// An MCP token: read every project on the account.
    Mcp,
    /// An API token: anything the approving user can do.
    Cli,
}

#[derive(Deserialize)]
pub(crate) struct AuthorizeRequest {
    client_id: String,
    redirect_uri: String,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    code_challenge: String,
    #[serde(default)]
    code_challenge_method: Option<String>,
    /// False when the user declined, which still redirects, with an error.
    #[serde(default = "approve_by_default")]
    approve: bool,
    /// [`CLI_SCOPE`] for the CLI. Anything else is an MCP client, which may send
    /// scopes of its own, and gets what MCP clients get.
    #[serde(default)]
    scope: Option<String>,
}

fn approve_by_default() -> bool {
    true
}

#[derive(Deserialize)]
struct AuthorizeQuery {
    client_id: String,
    redirect_uri: String,
}

#[derive(Serialize)]
struct ClientResponse {
    client_id: String,
    client_name: String,
    redirect_uri: String,
}

#[derive(Serialize)]
struct RedirectResponse {
    redirect_url: String,
}

/// `GET` — what the consent page shows the user: the registered name of the
/// client asking, confirmed against the redirect URI it wants to be sent to.
pub(crate) fn describe_client(server_context: Arc<ServerContext>, target: &str) -> Response {
    let query = target.split_once('?').map(|(_path, query)| query);
    let Some(request) = query.and_then(|query| {
        serde_urlencoded::from_str::<AuthorizeQuery>(query)
            .ok()
            .filter(|request| !request.client_id.is_empty())
    }) else {
        return invalid_request("client_id and redirect_uri are required");
    };

    let client = match block_on(clients::find_client(server_context, &request.client_id)) {
        Ok(Some(client)) => client,
        Ok(None) => return unknown_client(),
        Err(error) => return map_auth_error(error, "mcp_authorize"),
    };
    if !client.allows_redirect_uri(&request.redirect_uri) {
        return mismatched_redirect_uri();
    }

    oauth_json(
        "200 OK",
        ClientResponse {
            client_id: client.client_id,
            client_name: client.client_name,
            redirect_uri: request.redirect_uri,
        },
    )
}

/// `POST` — approve or deny, and hand back the URL the browser should follow.
/// The grant covers every project on the approving user's account for a year,
/// and only admins can revoke it, so only an admin may approve.
pub(crate) fn authorize(
    server_context: Arc<ServerContext>,
    account: &Account,
    body: &[u8],
) -> Response {
    let request = match serde_json::from_slice::<AuthorizeRequest>(body) {
        Ok(request) => request,
        Err(_) => return invalid_request("client_id and redirect_uri are required"),
    };
    if request
        .state
        .as_ref()
        .is_some_and(|state| state.len() > MAX_STATE_LEN)
    {
        return invalid_request("state is too long");
    }

    let client = match block_on(clients::find_client(
        server_context.clone(),
        &request.client_id,
    )) {
        Ok(Some(client)) => client,
        Ok(None) => return unknown_client(),
        Err(error) => return map_auth_error(error, "mcp_authorize"),
    };
    // Checked before anything else is decided: this is the only thing that keeps
    // a code, or an error, from being delivered to a URI the client never named.
    if !client.allows_redirect_uri(&request.redirect_uri) {
        return mismatched_redirect_uri();
    }

    if !request.approve {
        return redirect_response(
            &request.redirect_uri,
            &[
                ("error", "access_denied"),
                ("error_description", "the user declined the request"),
            ],
            request.state.as_deref(),
        );
    }

    let scope = if request.scope.as_deref() == Some(CLI_SCOPE) {
        GrantScope::Cli
    } else {
        GrantScope::Mcp
    };
    match scope {
        // Full access, so the code may only ever be delivered to this machine.
        GrantScope::Cli if !clients::is_loopback_redirect(&request.redirect_uri) => {
            return invalid_request("the cli scope needs a loopback redirect_uri");
        }
        // The token carries this user's own role, so any user may approve one.
        GrantScope::Cli => {}
        // After the deny path, so a member can still turn the agent away.
        GrantScope::Mcp => {
            if authorize_admin(account).is_err() {
                return oauth_error(
                    "403 Forbidden",
                    "access_denied",
                    "only an account admin can connect an agent",
                );
            }
        }
    }

    if !is_valid_code_challenge(&request.code_challenge) {
        return invalid_request("code_challenge must be a base64url S256 challenge");
    }
    if request
        .code_challenge_method
        .as_deref()
        .is_some_and(|method| method != "S256")
    {
        return invalid_request("code_challenge_method must be S256");
    }

    let issued = match server_context.mcp_authorization_codes.issue(
        AuthorizationGrant {
            client_id: client.client_id,
            redirect_uri: request.redirect_uri.clone(),
            code_challenge: request.code_challenge,
            account_id: account.user().account_id.clone(),
            user_id: account.user().user_id.clone(),
            scope,
        },
        MCP_AUTHORIZATION_CODE_TTL_SECONDS,
    ) {
        Ok(issued) => issued,
        Err(error) => return map_auth_error(error, "mcp_authorize"),
    };

    redirect_response(
        &request.redirect_uri,
        &[("code", &issued.secret.to_string())],
        request.state.as_deref(),
    )
}

/// Appends the authorization response to the redirect URI's query string,
/// keeping any query the client registered.
fn redirect_response(
    redirect_uri: &str,
    parameters: &[(&str, &str)],
    state: Option<&str>,
) -> Response {
    let mut pairs: Vec<(&str, &str)> = parameters.to_vec();
    if let Some(state) = state {
        pairs.push(("state", state));
    }
    let Ok(appended) = serde_urlencoded::to_string(&pairs) else {
        return invalid_request("the authorization response could not be encoded");
    };
    let Ok(mut url) = url::Url::parse(redirect_uri) else {
        return mismatched_redirect_uri();
    };
    let query = match url.query() {
        Some(existing) if !existing.is_empty() => format!("{existing}&{appended}"),
        _ => appended,
    };
    url.set_query(Some(&query));

    oauth_json(
        "200 OK",
        RedirectResponse {
            redirect_url: url.to_string(),
        },
    )
}

/// True for the base64url alphabet at a PKCE-legal length. A challenge we
/// cannot compare is rejected here rather than at the token endpoint, where the
/// user is no longer around to see the error.
fn is_valid_code_challenge(code_challenge: &str) -> bool {
    (MIN_CODE_CHALLENGE_LEN..=MAX_CODE_CHALLENGE_LEN).contains(&code_challenge.len())
        && code_challenge
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn unknown_client() -> Response {
    oauth_error(
        "400 Bad Request",
        "invalid_client",
        "that client_id is not registered",
    )
}

fn mismatched_redirect_uri() -> Response {
    oauth_error(
        "400 Bad Request",
        "invalid_request",
        "redirect_uri does not match one this client registered",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_base64url_challenges_of_a_legal_length() {
        assert!(is_valid_code_challenge(&"a".repeat(43)));
        assert!(is_valid_code_challenge(&"-_9Az".repeat(20)));
        assert!(!is_valid_code_challenge(&"a".repeat(42)));
        assert!(!is_valid_code_challenge(&"a".repeat(129)));
        assert!(!is_valid_code_challenge(&format!("{}+", "a".repeat(42))));
        assert!(!is_valid_code_challenge(""));
    }

    #[test]
    fn the_redirect_keeps_the_query_the_client_registered() {
        let response = redirect_response(
            "https://claude.ai/api/mcp/auth_callback?tenant=7",
            &[("code", "abc")],
            Some("state value/&"),
        );
        let body = String::from_utf8(response.body).expect("utf8");
        assert!(
            body.contains(
                "https://claude.ai/api/mcp/auth_callback?tenant=7&code=abc&state=state+value%2F%26"
            ),
            "{body}"
        );
    }

    #[test]
    fn a_denial_redirects_with_an_error_and_no_code() {
        let response = redirect_response(
            "cursor://anysphere.cursor-retrieval/oauth",
            &[("error", "access_denied")],
            None,
        );
        let body = String::from_utf8(response.body).expect("utf8");
        assert!(body.contains("error=access_denied"), "{body}");
        assert!(!body.contains("code="), "{body}");
    }
}
