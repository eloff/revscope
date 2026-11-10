//! The glue between our HTTP stack and `rmcp`.
//!
//! `rmcp` owns the protocol: JSON-RPC framing, the MCP lifecycle, tool routing,
//! and the Streamable HTTP rules about which method may return what. It speaks
//! `http::Request`/`http::Response`, so all that is left here is translating our
//! parsed request into one, checking the bearer token on the way in, and turning
//! the answer back into a [`Response`] — including the case where the answer is
//! an event stream rather than a buffer.
//!
//! This is the one route that does not run under `spawn_blocking`: `handle` is
//! async, and a streaming answer outlives the call that produced it. The blocking
//! work inside the tools is handed to `spawn_blocking` individually instead.

use std::sync::Arc;

use bytes::Bytes;
use http::header::CONTENT_TYPE;
use http::{HeaderName, HeaderValue};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use rmcp::transport::streamable_http_server::session::never::NeverSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use serde_json::json;
use tokio::sync::mpsc;

use crate::auth::AuthError;
use crate::http::{ParsedRequest, Response, parse_request_head, request_body};
use crate::mcp::service::{McpRequest, McpService};
use crate::mcp::{discovery, resource_metadata_url, token};
use crate::server_context::ServerContext;

/// Frames buffered between `rmcp` producing an event and the socket taking it.
const STREAM_CHANNEL_CAPACITY: usize = 16;

/// Response headers `rmcp` may set that describe its own framing rather than the
/// payload. We re-derive them from how the body is actually written.
const FRAMING_HEADERS: [&str; 3] = ["content-length", "transfer-encoding", "connection"];

pub(crate) type McpHttpService = StreamableHttpService<McpService, NeverSessionManager>;

/// Builds the service [`ServerContext`] holds. Stateless: both tools are pure
/// request-response, so there is no session worth keeping and no `Mcp-Session-Id`
/// to hand out. Answers come back as JSON, and `rmcp` only falls back to an event
/// stream if a handler emits something before its result.
pub(crate) fn build_mcp_service() -> McpHttpService {
    StreamableHttpService::new(
        || Ok(McpService::default()),
        Arc::new(NeverSessionManager::default()),
        StreamableHttpServerConfig::default()
            .with_legacy_session_mode(false)
            .with_json_response(true)
            .with_sse_keep_alive(None)
            // `Host` validation exists to protect servers running on a
            // developer's machine from DNS rebinding. This one is on the public
            // internet behind a proxy, so the `Host` it sees is not ours to
            // allowlist; the bearer token is what guards it.
            .disable_allowed_hosts(),
    )
}

pub(crate) async fn handle_mcp_request(
    raw_request: &[u8],
    server_context: Arc<ServerContext>,
) -> Response {
    let Some(parsed) = parse_request_head(raw_request) else {
        return Response::bad_request();
    };
    // The 401 is not just a rejection: its `WWW-Authenticate` header is how a
    // client that has never seen us finds the OAuth metadata and starts the flow.
    let Some(secret) = token::parse_authorization(parsed.authorization) else {
        return unauthorized(parsed.host, "an MCP access token is required");
    };
    let grant = match token::authorize_grant(server_context.clone(), secret).await {
        Ok(grant) => grant,
        Err(AuthError::InvalidSession | AuthError::InvalidCredentials) => {
            return unauthorized(parsed.host, "the MCP access token is not valid");
        }
        Err(error) => return crate::routes::map_auth_error(error, "mcp"),
    };

    let Some(_permit) = server_context.acquire_request_slot(None).await else {
        let mut response = Response::service_unavailable();
        discovery::with_cors_headers(&mut response);
        return response;
    };

    let grant = McpRequest {
        server_context: server_context.clone(),
        grant,
    };
    let Some(request) =
        build_http_request(&parsed, request_body(raw_request).unwrap_or(&[]), grant)
    else {
        return Response::bad_request();
    };

    let answer = server_context.mcp_service.handle(request).await;
    let mut response = convert_response(server_context, answer).await;
    discovery::with_cors_headers(&mut response);
    response
}

/// Rebuilds the request as `rmcp` wants it. Every header is copied rather than
/// picked from [`ParsedRequest`], which keeps only the ones our own routes read:
/// `rmcp` needs `Accept`, `Content-Type`, and the `Mcp-*` protocol headers. The
/// grant rides along in the extensions, which is the only channel from here into
/// a tool.
fn build_http_request(
    parsed: &ParsedRequest<'_>,
    body: &[u8],
    grant: McpRequest,
) -> Option<http::Request<Full<Bytes>>> {
    let mut builder = http::Request::builder()
        .method(parsed.method)
        .uri(parsed.target)
        .extension(grant);
    if let Some(headers) = builder.headers_mut() {
        for (name, value) in parsed.header_lines() {
            // A header we cannot represent is one `rmcp` could not have read
            // either, so dropping it is better than failing the request.
            if let (Ok(name), Ok(value)) = (
                HeaderName::from_bytes(name.as_bytes()),
                HeaderValue::from_str(value),
            ) {
                headers.append(name, value);
            }
        }
    }
    builder.body(Full::new(Bytes::copy_from_slice(body))).ok()
}

/// Serializes an `http::Response` into our header block plus a body. An event
/// stream is forwarded frame by frame; anything else is buffered, which is the
/// normal case because the service is configured to prefer JSON answers.
async fn convert_response(
    server_context: Arc<ServerContext>,
    response: http::Response<BoxBody<Bytes, std::convert::Infallible>>,
) -> Response {
    let (parts, body) = response.into_parts();
    let is_event_stream = parts
        .headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/event-stream"));

    let mut headers = status_line(parts.status);
    for (name, value) in &parts.headers {
        if FRAMING_HEADERS.contains(&name.as_str()) {
            continue;
        }
        let Ok(value) = value.to_str() else { continue };
        headers.extend_from_slice(name.as_str().as_bytes());
        headers.extend_from_slice(b": ");
        headers.extend_from_slice(value.as_bytes());
        headers.extend_from_slice(b"\r\n");
    }

    if !is_event_stream {
        let buffered = body
            .collect()
            .await
            .map(|collected| collected.to_bytes().to_vec())
            .unwrap_or_default();
        headers.extend_from_slice(format!("Content-Length: {}\r\n\r\n", buffered.len()).as_bytes());
        return Response::new(headers, buffered);
    }

    // The length is unknown, so closing the connection delimits the body,
    // exactly as `Response::event_stream` does for the Ask endpoint.
    headers.extend_from_slice(
        b"Cache-Control: no-store\r\nX-Accel-Buffering: no\r\nConnection: close\r\n\r\n",
    );
    let (sender, receiver) = mpsc::channel(STREAM_CHANNEL_CAPACITY);
    crate::task::spawn(server_context, async move {
        let mut body = body;
        while let Some(Ok(frame)) = body.frame().await {
            // Trailers carry nothing an SSE reader can use.
            let Ok(data) = frame.into_data() else {
                continue;
            };
            if sender.send(data.to_vec()).await.is_err() {
                break;
            }
        }
    });
    let mut response = Response::new(headers, Vec::new());
    response.stream_body = Some(receiver);
    response
}

fn status_line(status: http::StatusCode) -> Vec<u8> {
    format!(
        "HTTP/1.1 {} {}\r\n",
        status.as_u16(),
        status.canonical_reason().unwrap_or("Unknown")
    )
    .into_bytes()
}

/// The challenge that starts the OAuth flow. RFC 9728 section 5.1 defines
/// `resource_metadata`, and MCP clients follow it to discover everything else.
fn unauthorized(host: Option<&str>, detail: &str) -> Response {
    let mut response = Response::json_with_headers(
        "401 Unauthorized",
        json!({ "error": "invalid_token", "error_description": detail }).to_string(),
        &[(
            "WWW-Authenticate",
            format!(
                "Bearer realm=\"revscope\", resource_metadata=\"{}\"",
                resource_metadata_url(host)
            ),
        )],
    );
    discovery::with_cors_headers(&mut response);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_ACCOUNT: &str = "00000000-0000-0000-0000-000000000001";

    fn test_grant() -> McpRequest {
        McpRequest {
            server_context: Arc::new(ServerContext::new_in_memory("mcp-transport-test")),
            grant: token::McpGrant::Account(token::McpAccount::for_tests(TEST_ACCOUNT)),
        }
    }

    #[test]
    fn every_header_the_protocol_needs_survives_the_rebuild() {
        let raw = b"POST /v2/mcp HTTP/1.1\r\nHost: api.revscope.co\r\nAccept: application/json, text/event-stream\r\nContent-Type: application/json\r\nMcp-Protocol-Version: 2025-06-18\r\nMcp-Session-Id: abc\r\nContent-Length: 2\r\n\r\n{}";
        let parsed = parse_request_head(raw).expect("parse");
        let request = build_http_request(&parsed, request_body(raw).expect("body"), test_grant())
            .expect("build");

        assert_eq!(request.method(), http::Method::POST);
        assert_eq!(request.uri().path(), "/v2/mcp");
        assert_eq!(
            request.headers().get("accept").expect("accept"),
            "application/json, text/event-stream"
        );
        assert_eq!(
            request
                .headers()
                .get("mcp-protocol-version")
                .expect("protocol version"),
            "2025-06-18"
        );
        assert_eq!(
            request.headers().get("mcp-session-id").expect("session id"),
            "abc"
        );
    }

    #[test]
    fn the_grant_reaches_the_tools_through_the_extensions() {
        let raw = b"POST /v2/mcp HTTP/1.1\r\nHost: api.revscope.co\r\nContent-Length: 0\r\n\r\n";
        let parsed = parse_request_head(raw).expect("parse");
        let request = build_http_request(&parsed, &[], test_grant()).expect("build");
        let grant = request
            .extensions()
            .get::<McpRequest>()
            .expect("grant extension");
        assert_eq!(
            grant.grant,
            token::McpGrant::Account(token::McpAccount::for_tests(TEST_ACCOUNT))
        );
    }

    #[tokio::test]
    async fn overloaded_tool_calls_return_503_with_retry_after() {
        let context = Arc::new(ServerContext::new_in_memory_with_request_slots(
            "mcp-slot-timeout",
            1,
        ));
        let secret = crate::tokens::TokenSecret::generate().expect("token");
        context.mcp_token_cache.insert(
            secret,
            token::McpAccount::for_tests(TEST_ACCOUNT),
            crate::util::seconds_since_epoch().saturating_add(60),
        );
        let _held = context
            .acquire_request_slot(None)
            .await
            .expect("the only slot must be available");

        let raw = format!(
            "POST /v2/mcp HTTP/1.1\r\nHost: api.revscope.co\r\nAuthorization: Bearer rsm_{secret}\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{{}}"
        );
        let response = handle_mcp_request(raw.as_bytes(), context).await;
        let headers = String::from_utf8(response.headers).expect("utf8");
        assert!(
            headers.starts_with("HTTP/1.1 503 Service Unavailable"),
            "{headers}"
        );
        assert!(headers.contains("Retry-After: 5\r\n"), "{headers}");
        assert!(
            headers.contains("Access-Control-Allow-Origin: *\r\n"),
            "{headers}"
        );
    }

    #[test]
    fn the_challenge_points_at_the_protected_resource_document() {
        let response = unauthorized(Some("api.revscope.co"), "an MCP access token is required");
        let headers = String::from_utf8(response.headers).expect("utf8");
        assert!(headers.starts_with("HTTP/1.1 401 Unauthorized"));
        assert!(
            headers.contains(
                "WWW-Authenticate: Bearer realm=\"revscope\", resource_metadata=\"https://api.revscope.co/.well-known/oauth-protected-resource/v2/mcp\""
            ),
            "{headers}"
        );
        assert!(headers.contains("Access-Control-Expose-Headers: "));
    }
}
