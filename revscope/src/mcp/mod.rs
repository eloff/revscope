//! The hosted MCP server.
//!
//! An agent points its MCP client at `https://api.revscope.co/v2/mcp` and
//! nothing else: the client discovers the OAuth metadata from the 401 the
//! endpoint answers with, registers itself, sends the user through the consent
//! page in a browser, and exchanges the resulting code for a token that covers
//! every project on the account. From then on the endpoint speaks the Streamable
//! HTTP transport and exposes the same read-only tools the stdio `mcp/` package
//! does, plus `list_projects`, since it is not tied to one project.
//!
//! The pieces:
//!
//! * [`transport`] bridges our hand-rolled HTTP types to `rmcp`, which owns the
//!   JSON-RPC framing and the MCP lifecycle;
//! * [`service`] is the `rmcp` handler: `get_schema` and `run_query`;
//! * [`discovery`] serves the two `.well-known` documents that point a client
//!   at the endpoints below;
//! * [`register`], [`authorize`], and [`grant`] are the OAuth endpoints;
//! * [`revoke`] deletes every access token on the signed-in account;
//! * [`token`] mints and checks the `rsm_` access tokens they produce.

pub(crate) mod authorize;
pub(crate) mod clients;
pub(crate) mod discovery;
pub(crate) mod grant;
pub(crate) mod register;
pub(crate) mod revoke;
pub(crate) mod service;
pub(crate) mod token;
pub(crate) mod transport;

use serde::Serialize;

use crate::http::{Response, host_supports_secure_cookies};

/// Where the MCP endpoint is served, and the resource identifier the OAuth
/// metadata names. Also the path suffix of its protected-resource document.
pub(crate) const MCP_PATH: &str = "/v2/mcp";

pub(crate) use revscope_api::oauth::MCP_SCOPE;

/// Origin the caller reached us on, which every URL in the OAuth metadata has
/// to agree with: a client that dialed one host will not follow metadata that
/// names another. `REVSCOPE_API_BASE_URL` overrides it for deployments where
/// the public origin is not the `Host` we see.
pub(crate) fn api_base_url(host: Option<&str>) -> String {
    if let Ok(configured) = std::env::var("REVSCOPE_API_BASE_URL") {
        let configured = configured.trim().trim_end_matches('/');
        if !configured.is_empty() {
            return configured.to_string();
        }
    }
    let Some(host) = host.map(str::trim).filter(|host| !host.is_empty()) else {
        return "https://api.revscope.co".to_string();
    };
    // Development serves plain http on loopback; everything else is behind TLS.
    let scheme = if host_supports_secure_cookies(Some(host)) {
        "https"
    } else {
        "http"
    };
    format!("{scheme}://{host}")
}

/// The document a client fetches after a 401 to learn which authorization
/// server guards this endpoint.
pub(crate) fn resource_metadata_url(host: Option<&str>) -> String {
    format!(
        "{}/.well-known/oauth-protected-resource{MCP_PATH}",
        api_base_url(host)
    )
}

/// An OAuth error in the form RFC 6749 section 5.2 defines, which is what both
/// the token endpoint and the consent endpoint answer with so a client can act
/// on the code rather than parse prose.
#[derive(Serialize)]
pub(crate) struct OAuthErrorBody {
    error: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_description: Option<String>,
}

/// Tokens must never be cached, so every OAuth response says so, error or not.
pub(crate) fn oauth_json(status: &str, value: impl Serialize) -> Response {
    Response::json_with_headers(
        status,
        serde_json::to_vec(&value).unwrap_or_else(|_| b"{}".to_vec()),
        &[("Cache-Control", "no-store".to_string())],
    )
}

pub(crate) fn oauth_error(
    status: &str,
    error: &'static str,
    description: impl Into<String>,
) -> Response {
    oauth_json(
        status,
        OAuthErrorBody {
            error,
            error_description: Some(description.into()),
        },
    )
}

pub(crate) fn invalid_request(description: impl Into<String>) -> Response {
    oauth_error("400 Bad Request", "invalid_request", description)
}
