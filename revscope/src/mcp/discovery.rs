//! The `.well-known` documents an MCP client reads to find its way from a 401
//! at `/v2/mcp` to a token.
//!
//! Both live at the root rather than under `/v2/`, because the specs that define
//! them (RFC 9728 for the protected resource, RFC 8414 for the authorization
//! server) put them there. They are served next to the ACME challenge for the
//! same reason: routing under `/v2/` cannot reach them.
//!
//! Everything here is public, unauthenticated, and readable from any origin: a
//! browser-hosted client has to fetch it before it holds any credential.

use revscope_api::oauth::AuthorizationServerMetadata;
use serde::Serialize;

use crate::http::{ParsedRequest, Response};
use crate::mcp::{MCP_PATH, MCP_SCOPE, api_base_url};

const PROTECTED_RESOURCE_PATH: &str = "/.well-known/oauth-protected-resource";
const AUTHORIZATION_SERVER_PATH: &str = "/.well-known/oauth-authorization-server";

/// Where the browser part of the flow happens: a page in the web app, not an API
/// route, because only a browser carrying our session cookie can approve.
const AUTHORIZE_PAGE_PATH: &str = "/oauth/authorize";

const CORS_ALLOWED_METHODS: &str = "GET, POST, DELETE, OPTIONS";
/// `Mcp-Session-Id` and the SEP-2243 `Mcp-*` headers are sent by clients that
/// negotiate the newer protocol revisions; `Last-Event-ID` by resuming ones.
const CORS_ALLOWED_HEADERS: &str = "Content-Type, Authorization, Accept, Mcp-Session-Id, Mcp-Protocol-Version, Last-Event-ID, Mcp-Method, Mcp-Name";
/// `WWW-Authenticate` carries the pointer to the metadata below, so a browser
/// client has to be allowed to read it.
const CORS_EXPOSED_HEADERS: &str = "Mcp-Session-Id, Mcp-Protocol-Version, WWW-Authenticate";

#[derive(Serialize)]
struct ProtectedResourceMetadata {
    resource: String,
    authorization_servers: [String; 1],
    scopes_supported: [&'static str; 1],
    bearer_methods_supported: [&'static str; 1],
}

/// Answers the discovery documents, and the preflight for anything MCP, ahead of
/// normal routing. `None` when the request is not one of those.
pub(crate) fn well_known_response(parsed: &ParsedRequest<'_>) -> Option<Response> {
    let path = parsed.target.split('?').next().unwrap_or(parsed.target);
    let is_metadata = matches_document(path, PROTECTED_RESOURCE_PATH)
        || matches_document(path, AUTHORIZATION_SERVER_PATH);
    if !is_metadata && path != MCP_PATH {
        return None;
    }

    // Clients running in a browser preflight the POST because of the
    // `Authorization` and `Mcp-*` headers. The dashboard's credentialed
    // allowlist cannot answer that: an MCP client's origin is not ours.
    if parsed.method == "OPTIONS" {
        let mut response = Response::new(
            b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n".to_vec(),
            Vec::new(),
        );
        with_cors_headers(&mut response);
        return Some(response);
    }
    if !is_metadata {
        return None;
    }
    if parsed.method != "GET" && parsed.method != "HEAD" {
        return Some(Response::method_not_allowed());
    }

    let api_base_url = api_base_url(parsed.host);
    let mut response = if matches_document(path, PROTECTED_RESOURCE_PATH) {
        Response::ok_json(ProtectedResourceMetadata {
            resource: format!("{api_base_url}{MCP_PATH}"),
            authorization_servers: [api_base_url.clone()],
            scopes_supported: [MCP_SCOPE],
            bearer_methods_supported: ["header"],
        })
    } else {
        Response::ok_json(AuthorizationServerMetadata {
            issuer: api_base_url.clone(),
            authorization_endpoint: format!("{}{AUTHORIZE_PAGE_PATH}", crate::util::web_base_url()),
            token_endpoint: format!("{api_base_url}/v2/public/mcp/token"),
            registration_endpoint: format!("{api_base_url}/v2/public/mcp/register"),
            response_types_supported: vec!["code".to_string()],
            grant_types_supported: vec!["authorization_code".to_string()],
            // PKCE is mandatory: these are public clients with no secret.
            code_challenge_methods_supported: vec!["S256".to_string()],
            token_endpoint_auth_methods_supported: vec!["none".to_string()],
            scopes_supported: vec![MCP_SCOPE.to_string()],
        })
    };
    with_cors_headers(&mut response);
    Some(response)
}

/// RFC 9728 appends the resource path to the document path, and some clients
/// append it to the authorization server document too, so both forms resolve.
fn matches_document(path: &str, document: &str) -> bool {
    let Some(rest) = path.strip_prefix(document) else {
        return false;
    };
    rest.is_empty() || rest == "/" || rest == MCP_PATH
}

/// Wildcard CORS for the MCP surface. Safe because none of it is authenticated
/// by a cookie: the endpoint takes a bearer token, which a hostile page cannot
/// obtain from the browser's credential store.
pub(crate) fn with_cors_headers(response: &mut Response) {
    debug_assert!(
        response.headers.ends_with(b"\r\n\r\n"),
        "response headers must end with the header terminator before adding CORS headers"
    );
    let len = response.headers.len();
    response.headers.truncate(len - 2);
    use std::io::Write as _;
    let _ = write!(
        response.headers,
        "Access-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: {CORS_ALLOWED_METHODS}\r\nAccess-Control-Allow-Headers: {CORS_ALLOWED_HEADERS}\r\nAccess-Control-Expose-Headers: {CORS_EXPOSED_HEADERS}\r\nAccess-Control-Max-Age: 86400\r\n",
    );
    response.headers.extend_from_slice(b"\r\n");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::parse_request_head;
    use serde_json::Value;

    fn request(head: &str) -> Vec<u8> {
        head.as_bytes().to_vec()
    }

    fn respond(head: &str) -> Option<Response> {
        let raw = request(head);
        let parsed = parse_request_head(&raw).expect("parse");
        well_known_response(&parsed)
    }

    fn json_body(response: &Response) -> Value {
        serde_json::from_slice(&response.body).expect("json body")
    }

    #[test]
    fn the_protected_resource_document_points_at_this_origin() {
        let response = respond(
            "GET /.well-known/oauth-protected-resource/v2/mcp HTTP/1.1\r\nHost: api.revscope.co\r\n\r\n",
        )
        .expect("metadata response");
        let body = json_body(&response);
        assert_eq!(body["resource"], "https://api.revscope.co/v2/mcp");
        assert_eq!(
            body["authorization_servers"],
            serde_json::json!(["https://api.revscope.co"])
        );
        let headers = String::from_utf8(response.headers.clone()).expect("utf8");
        assert!(headers.contains("Access-Control-Allow-Origin: *"));
    }

    #[test]
    fn the_authorization_server_document_names_the_consent_page_and_pkce() {
        let response = respond(
            "GET /.well-known/oauth-authorization-server HTTP/1.1\r\nHost: api.revscope.co\r\n\r\n",
        )
        .expect("metadata response");
        let body = json_body(&response);
        assert_eq!(body["issuer"], "https://api.revscope.co");
        assert_eq!(
            body["token_endpoint"],
            "https://api.revscope.co/v2/public/mcp/token"
        );
        assert!(
            body["authorization_endpoint"]
                .as_str()
                .expect("authorization_endpoint")
                .ends_with("/oauth/authorize")
        );
        assert_eq!(
            body["code_challenge_methods_supported"],
            serde_json::json!(["S256"])
        );
    }

    #[test]
    fn a_loopback_host_is_described_over_plain_http() {
        let response = respond(
            "GET /.well-known/oauth-protected-resource HTTP/1.1\r\nHost: localhost:4320\r\n\r\n",
        )
        .expect("metadata response");
        assert_eq!(
            json_body(&response)["resource"],
            "http://localhost:4320/v2/mcp"
        );
    }

    #[test]
    fn the_mcp_endpoint_answers_a_preflight_from_any_origin() {
        let response = respond(
            "OPTIONS /v2/mcp HTTP/1.1\r\nHost: api.revscope.co\r\nOrigin: https://claude.ai\r\n\r\n",
        )
        .expect("preflight response");
        let headers = String::from_utf8(response.headers).expect("utf8");
        assert!(headers.starts_with("HTTP/1.1 204 No Content"));
        assert!(headers.contains("Access-Control-Allow-Origin: *"));
        assert!(headers.contains("Authorization"));
        assert!(headers.contains("Access-Control-Expose-Headers: "));
    }

    #[test]
    fn unrelated_paths_and_methods_are_left_to_normal_routing() {
        assert!(respond("GET /v2/mcp HTTP/1.1\r\nHost: api.revscope.co\r\n\r\n").is_none());
        assert!(respond("POST /v2/mcp HTTP/1.1\r\nHost: api.revscope.co\r\n\r\n").is_none());
        assert!(respond("GET /demo HTTP/1.1\r\nHost: api.revscope.co\r\n\r\n").is_none());
        assert!(
            respond("GET /.well-known/acme-challenge/x HTTP/1.1\r\nHost: api.revscope.co\r\n\r\n")
                .is_none()
        );
    }
}
