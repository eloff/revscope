use std::any::Any;
use std::io;
use std::net::IpAddr;
use std::path::PathBuf;
use std::str;
use std::sync::Arc;
use std::time::Duration;

use log::{debug, info};
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::mpsc;
use tokio::time::{timeout, timeout_at};

const CORS_ALLOWED_METHODS: &str = "GET, POST, PUT, PATCH, DELETE, OPTIONS, HEAD";
const CORS_DEFAULT_ALLOWED_HEADERS: &str = "Content-Type, Authorization";
/// Public ingest is credential-less, so `*` is valid and lets any customer
/// origin (including CNAME / first-party proxy hosts) read the JSON body.
const INGEST_CORS_ALLOWED_METHODS: &str = "GET, POST, OPTIONS, HEAD";
const INGEST_CORS_ALLOWED_HEADERS: &str = "Content-Type";

const NOT_FOUND_HEADERS: &[u8] = b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
const METHOD_NOT_ALLOWED_HEADERS: &[u8] =
    b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\n\r\n";
/// Protocol desync: always close after these so the peer cannot reuse the socket.
const REQUEST_TIMEOUT_HEADERS: &[u8] =
    b"HTTP/1.1 408 Request Timeout\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
const PAYLOAD_TOO_LARGE_HEADERS: &[u8] =
    b"HTTP/1.1 413 Payload Too Large\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
const BAD_REQUEST_HEADERS: &[u8] =
    b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
const NOT_IMPLEMENTED_HEADERS: &[u8] =
    b"HTTP/1.1 501 Not Implemented\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
const FORBIDDEN_HEADERS: &[u8] =
    b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
const INTERNAL_SERVER_ERROR_HEADERS: &[u8] =
    b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n";
/// Waited the request-slot timeout; client should retry after the same interval.
const SERVICE_UNAVAILABLE_HEADERS: &[u8] =
    b"HTTP/1.1 503 Service Unavailable\r\nRetry-After: 5\r\nContent-Length: 0\r\n\r\n";

const GIF_HEADERS: &[u8] =
    b"HTTP/1.1 200 OK\r\nContent-Type: image/gif\r\nContent-Length: 42\r\n\r\n";
// The answer to life, the universe, and everything
const GIF_BODY: &[u8] = b"\x47\x49\x46\x38\x39\x61\x01\0\x01\0\x80\0\0\0\0\0\xff\xff\xff\x21\xf9\x04\x01\0\0\0\0\x2c\0\0\0\0\x01\0\x01\0\0\x02\x01\x44\0\x3b";

/// Gzipped `"404 - Page Not Found\n"` repeated 40_000 times — for unmatched routes.
const ROUTE_NOT_FOUND_BODY_GZ: &[u8] = include_bytes!("../404.gz");

/// A byte range of a file to send as the response body, streamed by the writer
/// instead of being buffered in [`Response::body`].
#[derive(Debug, Clone)]
pub(crate) struct FileBody {
    pub(crate) path: PathBuf,
    pub(crate) offset: u64,
    pub(crate) length: u64,
    /// Held until the body has been written. The writer opens `path` only
    /// once it gets there, so whatever owns the file (and would delete it on
    /// drop) has to live that long.
    pub(crate) _keep_alive: Option<Arc<dyn Any + Send + Sync>>,
}

/// Response bytes split into a header buffer and a body buffer.
///
/// `headers` always ends with the `\r\n\r\n` header terminator so it can be
/// written directly to a TCP stream. CORS helpers mutate `headers` in place
/// without touching `body`, which lets callers reuse the body buffer across
/// the CORS injection step.
#[derive(Debug)]
pub(crate) struct Response {
    pub(crate) headers: Vec<u8>,
    pub(crate) body: Vec<u8>,
    /// Body to read off disk when it is too large to buffer in `body`. Boxed
    /// because downloads are rare and it is the largest field: inline, it made
    /// every small JSON response 32 bytes wider.
    pub(crate) file_body: Option<Box<FileBody>>,
    /// Body produced after the headers are written, one frame per message. The
    /// handler returns while a background task keeps sending.
    pub(crate) stream_body: Option<mpsc::Receiver<Vec<u8>>>,
    /// Server-side detail to log; never sent to the client.
    pub(crate) internal_error: Option<String>,
    /// When true, skip the usual completed-request access log line.
    pub(crate) skip_access_log: bool,
}

impl Response {
    pub(crate) fn new(headers: Vec<u8>, body: Vec<u8>) -> Self {
        Self {
            headers,
            body,
            file_body: None,
            stream_body: None,
            internal_error: None,
            skip_access_log: false,
        }
    }

    pub(crate) fn not_found() -> Self {
        Self::new(NOT_FOUND_HEADERS.to_vec(), Vec::new())
    }

    /// Response used when no route handler matched the request path.
    pub(crate) fn route_not_found() -> Self {
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Encoding: gzip\r\nCache-Control: no-store, no-cache, must-revalidate\r\nPragma: no-cache\r\nContent-Length: {}\r\n\r\n",
            ROUTE_NOT_FOUND_BODY_GZ.len(),
        )
        .into_bytes();
        let mut response = Self::new(headers, ROUTE_NOT_FOUND_BODY_GZ.to_vec());
        response.skip_access_log = true;
        response
    }

    pub(crate) fn method_not_allowed() -> Self {
        Self::new(METHOD_NOT_ALLOWED_HEADERS.to_vec(), Vec::new())
    }

    pub(crate) fn request_timeout() -> Self {
        Self::new(REQUEST_TIMEOUT_HEADERS.to_vec(), Vec::new())
    }

    pub(crate) fn payload_too_large() -> Self {
        Self::new(PAYLOAD_TOO_LARGE_HEADERS.to_vec(), Vec::new())
    }

    pub(crate) fn bad_request() -> Self {
        Self::new(BAD_REQUEST_HEADERS.to_vec(), Vec::new())
    }

    pub(crate) fn not_implemented() -> Self {
        Self::new(NOT_IMPLEMENTED_HEADERS.to_vec(), Vec::new())
    }

    pub(crate) fn forbidden() -> Self {
        Self::new(FORBIDDEN_HEADERS.to_vec(), Vec::new())
    }

    pub(crate) fn internal_server_error() -> Self {
        Self::new(INTERNAL_SERVER_ERROR_HEADERS.to_vec(), Vec::new())
    }

    pub(crate) fn service_unavailable() -> Self {
        Self::new(SERVICE_UNAVAILABLE_HEADERS.to_vec(), Vec::new())
    }

    pub(crate) fn gif() -> Self {
        Self::new(GIF_HEADERS.to_vec(), GIF_BODY.to_vec())
    }

    pub(crate) fn gif_with_headers(extra_headers: &[(&str, String)]) -> Self {
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: image/gif\r\n{}Content-Length: {}\r\n\r\n",
            format_header_block(extra_headers),
            GIF_BODY.len(),
        )
        .into_bytes();
        Self::new(headers, GIF_BODY.to_vec())
    }

    pub(crate) fn javascript(body: &'static [u8], content_encoding: Option<&str>) -> Self {
        let encoding_header = match content_encoding {
            Some(encoding) => format!("Content-Encoding: {encoding}\r\n"),
            None => String::new(),
        };
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/javascript; charset=utf-8\r\nCache-Control: public, max-age=14400\r\n{encoding_header}Vary: Accept-Encoding\r\nContent-Length: {}\r\n\r\n",
            body.len(),
        )
        .into_bytes();
        Self::new(headers, body.to_vec())
    }

    pub(crate) fn json(status: &str, body: impl Into<Vec<u8>>) -> Self {
        Self::json_with_headers(status, body, &[])
    }

    pub(crate) fn serialize_json(status: &str, value: impl Serialize) -> Self {
        let body = serde_json::to_vec(&value).unwrap_or_else(|_| b"{}".to_vec());
        Self::json(status, body)
    }

    pub(crate) fn ok_json(value: impl Serialize) -> Self {
        Self::serialize_json("200 OK", value)
    }

    pub(crate) fn json_with_headers(
        status: &str,
        body: impl Into<Vec<u8>>,
        extra_headers: &[(&str, String)],
    ) -> Self {
        let body = body.into();
        let headers = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{}Content-Length: {}\r\n\r\n",
            format_header_block(extra_headers),
            body.len(),
        )
        .into_bytes();
        Self::new(headers, body)
    }

    /// Buffered body with an explicit content type and optional extra headers.
    pub(crate) fn with_content_type(
        status: &str,
        content_type: &str,
        body: impl Into<Vec<u8>>,
        extra_headers: &[(&str, String)],
    ) -> Self {
        let body = body.into();
        let headers = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\n{}Content-Length: {}\r\n\r\n",
            format_header_block(extra_headers),
            body.len(),
        )
        .into_bytes();
        Self::new(headers, body)
    }

    /// Response whose body is read from disk as it is written to the socket.
    pub(crate) fn file(
        status: &str,
        file_body: FileBody,
        extra_headers: &[(&str, String)],
    ) -> Self {
        let headers = format!(
            "HTTP/1.1 {status}\r\n{}Content-Length: {}\r\n\r\n",
            format_header_block(extra_headers),
            file_body.length,
        )
        .into_bytes();
        let mut response = Self::new(headers, Vec::new());
        response.file_body = Some(Box::new(file_body));
        response
    }

    /// Server-sent events whose frames arrive on `frames` after the headers are
    /// written. The length is unknown up front, so the body is delimited by
    /// closing the connection; clients must treat a stream that ends without
    /// the handler's terminal event as truncated.
    pub(crate) fn event_stream(frames: mpsc::Receiver<Vec<u8>>) -> Self {
        let headers = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\nCache-Control: no-store\r\nX-Accel-Buffering: no\r\nConnection: close\r\n\r\n".to_vec();
        let mut response = Self::new(headers, Vec::new());
        response.stream_body = Some(frames);
        response
    }

    /// True when this response ends the TCP connection (SSE, or an explicit
    /// `Connection: close` on a protocol-error response).
    pub(crate) fn closes_connection(&self) -> bool {
        self.stream_body.is_some() || headers_request_connection_close(&self.headers)
    }

    pub(crate) fn with_internal_error(mut self, error: impl Into<String>) -> Self {
        self.internal_error = Some(error.into());
        self
    }
}

fn format_header_block(extra_headers: &[(&str, String)]) -> String {
    let mut header_block = String::new();
    for (name, value) in extra_headers {
        header_block.push_str(name);
        header_block.push_str(": ");
        header_block.push_str(value);
        header_block.push_str("\r\n");
    }
    header_block
}

#[derive(Debug)]
pub(crate) enum ReadRequestError {
    /// No request began before the idle timeout; close quietly.
    IdleTimeout,
    /// Peer closed before sending a complete request; close quietly.
    Closed,
    /// A request was started but not finished in time.
    Timeout,
    TooLarge,
    /// The request's body length can't be determined unambiguously, so whatever
    /// follows the headers can't be told apart from the next request.
    BadFraming(BadFraming),
    Io(io::Error),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BadFraming {
    /// Any `Transfer-Encoding`: we only frame bodies by `Content-Length`, and
    /// RFC 9112 makes `Transfer-Encoding` override it, so honouring the length
    /// would read the body differently from a proxy that decodes chunks.
    TransferEncoding,
    /// A `Content-Length` that isn't one non-negative integer, conflicting
    /// repeats of it, a bare CR or LF in the head, a folded header line, or a
    /// header name that isn't a token (which a proxy might trim and still
    /// treat as the framing header we skipped).
    Malformed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ContentEncoding {
    Identity,
    Gzip,
    Brotli,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ParsedRequest<'a> {
    pub(crate) method: &'a str,
    pub(crate) target: &'a str,
    /// The request head as sent, without the trailing terminator. Read through
    /// [`Self::header_lines`] by the MCP route, which needs headers this struct
    /// does not keep.
    pub(crate) head: &'a str,
    pub(crate) cookie: Option<&'a str>,
    pub(crate) authorization: Option<&'a str>,
    pub(crate) origin: Option<&'a str>,
    /// Raw `Host` header (may include a port), used to decide cookie flags.
    pub(crate) host: Option<&'a str>,
    pub(crate) access_control_request_headers: Option<&'a str>,
    pub(crate) accept_encoding: Option<&'a str>,
    /// Raw `User-Agent` header, as sent.
    pub(crate) user_agent: Option<&'a str>,
    /// Raw `Accept-Language` header, as sent. Used as the language fallback when
    /// the tracker is not allowed to read `navigator.language`.
    pub(crate) accept_language: Option<&'a str>,
    /// Byte range requested by a resuming download client.
    pub(crate) range: Option<&'a str>,
    /// Validator guarding `range`; the range is ignored when it no longer matches.
    pub(crate) if_range: Option<&'a str>,
    /// Untrusted forwarded-IP header, in priority order: `CF-Connecting-IP`,
    /// `CF-Connecting-IPv6`, first value of `X-Forwarded-For`, then `X-Real-IP`.
    /// Call [`resolve_client_ip`] with the network peer before using this.
    pub(crate) client_ip: Option<&'a str>,
    /// Last value of the last `X-Forwarded-For` header: the address a reverse
    /// proxy that appends to the header saw. With
    /// `REVSCOPE_TRUST_X_FORWARDED_FOR`, used instead of `client_ip` unless
    /// [`forwarded_headers_trusted`] holds.
    pub(crate) x_forwarded_for_last: Option<&'a str>,
    /// Raw `CF-IPCountry` header. Call [`trusted_cf_ipcountry`] before using it
    /// for geo or consent; a direct client can forge the header.
    pub(crate) cf_ipcountry: Option<&'a str>,
    /// Standard Webhooks signing headers, sent by Polar on every delivery.
    /// Parsed in every build, but only the billing webhooks read them.
    #[cfg_attr(not(feature = "billing"), allow(dead_code))]
    pub(crate) webhook_id: Option<&'a str>,
    #[cfg_attr(not(feature = "billing"), allow(dead_code))]
    pub(crate) webhook_timestamp: Option<&'a str>,
    #[cfg_attr(not(feature = "billing"), allow(dead_code))]
    pub(crate) webhook_signature: Option<&'a str>,
    /// Stripe's `Stripe-Signature` header (`t=…,v1=…`).
    #[cfg_attr(not(feature = "billing"), allow(dead_code))]
    pub(crate) stripe_signature: Option<&'a str>,
    /// True when the request's `Connection` header includes `close`.
    pub(crate) connection_close: bool,
}

impl<'a> ParsedRequest<'a> {
    /// Every header as `(name, trimmed value)`, in the order sent. Repeated
    /// names are yielded once each rather than folded together.
    pub(crate) fn header_lines(&self) -> impl Iterator<Item = (&'a str, &'a str)> {
        self.head
            .split("\r\n")
            .skip(1)
            .filter_map(|line| line.split_once(':'))
            .map(|(name, value)| (name.trim(), value.trim()))
            .filter(|(name, _)| !name.is_empty())
    }
}

pub(crate) fn parse_request_head(request: &[u8]) -> Option<ParsedRequest<'_>> {
    let head_end = find_header_end(request)?;
    let head = std::str::from_utf8(&request[..head_end]).ok()?;
    let mut lines = head.split("\r\n");
    let request_line = lines.next()?;
    let mut request_line_parts = request_line.split_whitespace();
    let method = request_line_parts.next()?;
    let target = request_line_parts.next()?;
    let _version = request_line_parts.next()?;
    let mut cookie = None;
    let mut authorization = None;
    let mut origin = None;
    let mut host = None;
    let mut access_control_request_headers = None;
    let mut accept_encoding = None;
    let mut user_agent = None;
    let mut accept_language = None;
    let mut range = None;
    let mut if_range = None;
    let mut cf_connecting_ip = None;
    let mut cf_connecting_ipv6 = None;
    let mut cf_ipcountry = None;
    let mut x_forwarded_for = None;
    let mut x_forwarded_for_last = None;
    let mut x_real_ip = None;
    let mut webhook_id = None;
    let mut webhook_timestamp = None;
    let mut webhook_signature = None;
    let mut stripe_signature = None;
    let mut connection_close = false;

    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("cookie") {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    cookie = Some(trimmed);
                }
            } else if name.eq_ignore_ascii_case("authorization") {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    authorization = Some(trimmed);
                }
            } else if name.eq_ignore_ascii_case("origin") {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    origin = Some(trimmed);
                }
            } else if name.eq_ignore_ascii_case("host") {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    host = Some(trimmed);
                }
            } else if name.eq_ignore_ascii_case("access-control-request-headers") {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    access_control_request_headers = Some(trimmed);
                }
            } else if name.eq_ignore_ascii_case("accept-encoding") {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    accept_encoding = Some(trimmed);
                }
            } else if name.eq_ignore_ascii_case("user-agent") {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    user_agent = Some(trimmed);
                }
            } else if name.eq_ignore_ascii_case("accept-language") {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    accept_language = Some(trimmed);
                }
            } else if name.eq_ignore_ascii_case("range") {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    range = Some(trimmed);
                }
            } else if name.eq_ignore_ascii_case("if-range") {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    if_range = Some(trimmed);
                }
            } else if name.eq_ignore_ascii_case("cf-connecting-ip") {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    cf_connecting_ip = Some(trimmed);
                }
            } else if name.eq_ignore_ascii_case("cf-connecting-ipv6") {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    cf_connecting_ipv6 = Some(trimmed);
                }
            } else if name.eq_ignore_ascii_case("cf-ipcountry") {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    cf_ipcountry = Some(trimmed);
                }
            } else if name.eq_ignore_ascii_case("x-forwarded-for") {
                x_forwarded_for = parse_x_forwarded_for(value);
                x_forwarded_for_last = parse_x_forwarded_for_last(value);
            } else if name.eq_ignore_ascii_case("x-real-ip") {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    x_real_ip = Some(trimmed);
                }
            } else if name.eq_ignore_ascii_case("webhook-id") {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    webhook_id = Some(trimmed);
                }
            } else if name.eq_ignore_ascii_case("webhook-timestamp") {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    webhook_timestamp = Some(trimmed);
                }
            } else if name.eq_ignore_ascii_case("webhook-signature") {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    webhook_signature = Some(trimmed);
                }
            } else if name.eq_ignore_ascii_case("stripe-signature") {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    stripe_signature = Some(trimmed);
                }
            } else if name.eq_ignore_ascii_case("connection")
                && connection_header_requests_close(value)
            {
                connection_close = true;
            }
        }
    }

    // Header values only; [`resolve_client_ip`] decides whether they are trusted.
    let client_ip = cf_connecting_ip
        .or(cf_connecting_ipv6)
        .or(x_forwarded_for)
        .or(x_real_ip);

    Some(ParsedRequest {
        method,
        target,
        head,
        cookie,
        authorization,
        origin,
        host,
        access_control_request_headers,
        accept_encoding,
        user_agent,
        accept_language,
        range,
        if_range,
        client_ip,
        x_forwarded_for_last,
        cf_ipcountry,
        webhook_id,
        webhook_timestamp,
        webhook_signature,
        stripe_signature,
        connection_close,
    })
}

/// Prefer brotli, then gzip, when advertised with a positive `q` value.
pub(crate) fn negotiate_content_encoding(accept_encoding: Option<&str>) -> ContentEncoding {
    let Some(header) = accept_encoding else {
        return ContentEncoding::Identity;
    };

    let mut brotli_q = 0.0f32;
    let mut gzip_q = 0.0f32;

    for part in header.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let mut params = part.split(';');
        let coding = params.next().unwrap_or("").trim();
        let mut q = 1.0f32;
        for param in params {
            let param = param.trim();
            let Some(value) = param
                .strip_prefix("q=")
                .or_else(|| param.strip_prefix("Q="))
            else {
                continue;
            };
            if let Ok(parsed) = value.trim().parse::<f32>() {
                q = parsed.clamp(0.0, 1.0);
            }
        }
        if q <= 0.0 {
            continue;
        }
        if coding.eq_ignore_ascii_case("br") {
            brotli_q = brotli_q.max(q);
        } else if coding.eq_ignore_ascii_case("gzip") || coding.eq_ignore_ascii_case("x-gzip") {
            gzip_q = gzip_q.max(q);
        }
    }

    if brotli_q > 0.0 && brotli_q >= gzip_q {
        ContentEncoding::Brotli
    } else if gzip_q > 0.0 {
        ContentEncoding::Gzip
    } else {
        ContentEncoding::Identity
    }
}

fn parse_x_forwarded_for(value: &str) -> Option<&str> {
    let first = value.split(',').next()?.trim();
    if first.is_empty() { None } else { Some(first) }
}

fn parse_x_forwarded_for_last(value: &str) -> Option<&str> {
    let last = value.rsplit(',').next()?.trim();
    if last.is_empty() { None } else { Some(last) }
}

/// Forwarded-for headers are trusted only when the TCP peer is a Cloudflare
/// proxy (published ranges in `ip_ranges`) or loopback (local/dev).
pub(crate) fn is_trusted_forwarding_proxy(ip: IpAddr) -> bool {
    let ip = crate::ip_ranges::canonical_ip(ip);
    ip.is_loopback() || is_cloudflare(ip)
}

fn is_cloudflare(ip: IpAddr) -> bool {
    crate::ip_ranges::operator_for(crate::ip_ranges::canonical_ip(ip)) == Some("cloudflare")
}

/// Whether this request's forwarded headers, `CF-Connecting-IP` and
/// `CF-IPCountry` first among them, were set by a proxy we trust rather than
/// by the client.
///
/// Without `trust_x_forwarded_for` that is any trusted forwarding peer. With
/// it, a loopback peer is the self-hoster's own reverse proxy, and nginx or
/// Caddy pass a client's headers through as is, so they are believed only when
/// the address that proxy appended to `X-Forwarded-For` is Cloudflare's, which
/// overwrites them. A Cloudflare peer is believed either way.
pub(crate) fn forwarded_headers_trusted(
    parsed: &ParsedRequest<'_>,
    connection_ip: Option<IpAddr>,
    trusted_proxy: bool,
    trust_x_forwarded_for: bool,
) -> bool {
    if !trust_x_forwarded_for {
        return trusted_proxy;
    }
    if connection_ip.is_some_and(is_cloudflare) {
        return true;
    }
    parsed
        .x_forwarded_for_last
        .and_then(|value| value.parse::<IpAddr>().ok())
        .is_some_and(is_cloudflare)
}

/// Visitor IP used for GeoIP, consent, `user_hash`, and exclude-user matching.
///
/// `trusted_proxy` is the once-per-connection Cloudflare/loopback check.
/// Forwarded headers are honored only then; any other peer uses the peer
/// address, so forged proxy headers cannot spoof geo or identity.
pub(crate) fn resolve_client_ip(
    header_ip: Option<&str>,
    connection_ip: Option<IpAddr>,
    trusted_proxy: bool,
) -> Option<IpAddr> {
    let peer = connection_ip?;
    if trusted_proxy {
        Some(
            header_ip
                .and_then(|value| value.parse::<IpAddr>().ok())
                .unwrap_or(peer),
        )
    } else {
        Some(peer)
    }
}

/// Visitor IP for `parsed`, arriving from `connection_ip`.
///
/// `headers_trusted` comes from [`forwarded_headers_trusted`]. Otherwise, with
/// `trust_x_forwarded_for`, the peer is taken to be a reverse proxy, and only
/// the last `X-Forwarded-For` value, the one that proxy appended, is used:
/// earlier values and the other headers came from the client and may be forged.
pub(crate) fn request_client_ip(
    parsed: &ParsedRequest<'_>,
    connection_ip: Option<IpAddr>,
    headers_trusted: bool,
    trust_x_forwarded_for: bool,
) -> Option<IpAddr> {
    if headers_trusted {
        resolve_client_ip(parsed.client_ip, connection_ip, true)
    } else if trust_x_forwarded_for {
        resolve_client_ip(parsed.x_forwarded_for_last, connection_ip, true)
    } else {
        connection_ip
    }
}

/// `CF-IPCountry` when [`forwarded_headers_trusted`] says the request's
/// forwarded headers are. Otherwise it is discarded so a client cannot spoof
/// consent or checkout routing.
pub(crate) fn trusted_cf_ipcountry(header: Option<&str>, headers_trusted: bool) -> Option<&str> {
    if headers_trusted { header } else { None }
}

/// Reads one HTTP request into memory.
///
/// `pending` holds bytes already read off the connection past the end of the
/// previous request (a pipelined client); they start this request and any
/// excess past its end is put back. Dropping them would make the next read
/// start mid-request, so a body could be parsed as a request.
///
/// `idle_timeout` bounds the whole wait for the first byte of the next request.
/// Leading blank lines don't extend it, or a peer could hold the connection
/// forever by trickling them. Once data arrives, `request_timeout` bounds how
/// long the rest of that request may take.
pub(crate) async fn read_request_with_limits<S: AsyncRead + Unpin>(
    stream: &mut S,
    pending: &mut Vec<u8>,
    max_size: usize,
    idle_timeout: Duration,
    request_timeout: Duration,
) -> Result<Vec<u8>, ReadRequestError> {
    let mut buffer = std::mem::take(pending);
    let mut chunk = [0_u8; 1024];
    let idle_deadline = tokio::time::Instant::now() + idle_timeout;

    // RFC 9112 §2.2: ignore empty lines before a request. Some clients send a
    // CRLF after a body, and it must not start the request timer on an idle
    // connection.
    loop {
        let blank_prefix = buffer
            .as_chunks::<2>()
            .0
            .iter()
            .take_while(|pair| *pair == b"\r\n")
            .count()
            * 2;
        buffer.drain(..blank_prefix);
        // A lone CR may be half of a blank line whose LF is still in flight.
        if !buffer.is_empty() && buffer != b"\r" {
            break;
        }
        let bytes_read = match timeout_at(idle_deadline, stream.read(&mut chunk)).await {
            Ok(Ok(0)) => return Err(ReadRequestError::Closed),
            Ok(Ok(n)) => n,
            Ok(Err(error)) => return Err(ReadRequestError::Io(error)),
            Err(_) => return Err(ReadRequestError::IdleTimeout),
        };
        buffer.extend_from_slice(&chunk[..bytes_read]);
    }

    let read_future = async {
        let mut request_end = None;
        loop {
            if request_end.is_none() {
                request_end = find_request_end(&buffer).map_err(ReadRequestError::BadFraming)?;
            }
            if request_end.unwrap_or(buffer.len()) > max_size {
                return Err(ReadRequestError::TooLarge);
            }
            if let Some(end) = request_end
                && buffer.len() >= end
            {
                return Ok(end);
            }

            let bytes_read = stream
                .read(&mut chunk)
                .await
                .map_err(ReadRequestError::Io)?;
            if bytes_read == 0 {
                return Err(ReadRequestError::Closed);
            }
            buffer.extend_from_slice(&chunk[..bytes_read]);
        }
    };

    match timeout(request_timeout, read_future).await {
        Ok(Ok(end)) => {
            *pending = buffer.split_off(end);
            Ok(buffer)
        }
        Ok(Err(error)) => Err(error),
        Err(_) => Err(ReadRequestError::Timeout),
    }
}

fn connection_header_requests_close(value: &str) -> bool {
    value
        .split(',')
        .any(|token| token.trim().eq_ignore_ascii_case("close"))
}

/// True when a response header block includes `Connection: close`.
fn headers_request_connection_close(headers: &[u8]) -> bool {
    let Ok(text) = str::from_utf8(headers) else {
        return false;
    };
    for line in text.split("\r\n") {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("connection") && connection_header_requests_close(value) {
            return true;
        }
    }
    false
}

// Extracts request body bytes from a full raw HTTP request.
pub(crate) fn request_body(request: &[u8]) -> Option<&[u8]> {
    let body_start = find_header_end(request)? + 4;
    let body_end = find_request_end(request).ok()??;
    request.get(body_start..body_end)
}

/// Offset just past the body of the request at the start of `buffer`, or
/// `None` while its head is still incomplete.
fn find_request_end(buffer: &[u8]) -> Result<Option<usize>, BadFraming> {
    let Some(head_end) = find_header_end(buffer) else {
        return Ok(None);
    };
    let length = body_length(&buffer[..head_end])?;
    // Checked because a wrapped sum would mark a huge body as already read.
    (head_end + 4)
        .checked_add(length)
        .map(Some)
        .ok_or(BadFraming::Malformed)
}

#[derive(Debug)]
pub(crate) struct CorsHeaders {
    pub(crate) origin: String,
    pub(crate) request_headers: Option<String>,
}

/// Sites allowed to call the API cross-origin with the session cookie, i.e.
/// where a separately deployed web app is served.
///
/// Read from `REVSCOPE_CORS_ORIGINS`: comma-separated hostnames (or origins,
/// whose scheme and port are ignored), where `*.example.com` matches every
/// subdomain but not `example.com` itself. Set it empty when the UI is only
/// served by this process, same-origin. Loopback origins are always allowed for
/// local development.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CorsOrigins {
    /// Lowercased hostnames, or `.example.com` suffixes for wildcards.
    patterns: Vec<String>,
}

impl CorsOrigins {
    pub(crate) fn from_env() -> Self {
        match std::env::var("REVSCOPE_CORS_ORIGINS") {
            Ok(value) => Self::parse(&value),
            Err(_) => Self::build_default(),
        }
    }

    /// A binary with the UI compiled in serves it itself and needs no
    /// cross-origin callers; otherwise the UI is the one on revscope.co.
    pub(crate) fn build_default() -> Self {
        if cfg!(feature = "self-hosted") {
            Self::parse("")
        } else {
            Self::parse("revscope.co,*.revscope.co")
        }
    }

    pub(crate) fn parse(value: &str) -> Self {
        let patterns = value
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .filter_map(|entry| {
                let authority = entry.split_once("://").map_or(entry, |(_, rest)| rest);
                let authority = authority.split(['/', '?', '#']).next().unwrap_or(authority);
                let (wildcard, authority) = match authority.strip_prefix("*.") {
                    Some(rest) => (true, rest),
                    None => (false, authority),
                };
                let host = host_header_hostname(authority)?.to_ascii_lowercase();
                Some(if wildcard { format!(".{host}") } else { host })
            })
            .collect();
        Self { patterns }
    }

    pub(crate) fn allows(&self, origin: &str) -> bool {
        let Some((_, host)) = split_origin(origin) else {
            return false;
        };
        if is_loopback_host(host) {
            return true;
        }
        let host = host.to_ascii_lowercase();
        self.patterns.iter().any(|pattern| {
            if pattern.starts_with('.') {
                host.ends_with(pattern.as_str())
            } else {
                host == *pattern
            }
        })
    }
}

pub(crate) fn cors_headers_for_request(
    parsed: &ParsedRequest<'_>,
    allowed: &CorsOrigins,
) -> Option<CorsHeaders> {
    let origin = parsed.origin?;
    if !allowed.allows(origin) {
        return None;
    }

    Some(CorsHeaders {
        origin: origin.to_string(),
        request_headers: parsed
            .access_control_request_headers
            .map(std::string::ToString::to_string),
    })
}

pub(crate) fn preflight_response(cors: Option<&CorsHeaders>) -> Response {
    let mut response = if cors.is_some() {
        Response::new(
            b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n".to_vec(),
            Vec::new(),
        )
    } else {
        Response::new(
            b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n".to_vec(),
            Vec::new(),
        )
    };
    with_cors_headers(&mut response, cors);
    response
}

/// Preflight for public ingest: always allowed, never credentialed.
pub(crate) fn ingest_preflight_response() -> Response {
    let mut response = Response::new(
        b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n".to_vec(),
        Vec::new(),
    );
    with_ingest_cors_headers(&mut response);
    response
}

/// Inject credentialed dashboard CORS headers into `response.headers` in-place.
/// The body buffer is untouched so callers do not pay for an extra copy.
pub(crate) fn with_cors_headers(response: &mut Response, cors: Option<&CorsHeaders>) {
    let Some(cors) = cors else { return };
    debug_assert!(
        response.headers.ends_with(b"\r\n\r\n"),
        "response headers must end with the header terminator before adding CORS headers"
    );
    let len = response.headers.len();
    // Trim the trailing \r\n so the final header line still ends with \r\n but
    // the block is no longer terminated; we will re-terminate at the end.
    response.headers.truncate(len - 2);
    let request_headers = cors
        .request_headers
        .as_deref()
        .unwrap_or(CORS_DEFAULT_ALLOWED_HEADERS);
    use std::io::Write as _;
    let _ = write!(
        response.headers,
        "Access-Control-Allow-Origin: {}\r\nAccess-Control-Allow-Credentials: true\r\nAccess-Control-Allow-Methods: {CORS_ALLOWED_METHODS}\r\nAccess-Control-Allow-Headers: {request_headers}\r\nAccess-Control-Max-Age: 86400\r\nVary: Origin\r\n",
        cors.origin,
    );
    response.headers.extend_from_slice(b"\r\n");
}

/// Inject public ingest CORS headers (`Access-Control-Allow-Origin: *`, no
/// credentials). Safe for any customer origin behind a CNAME or path proxy.
pub(crate) fn with_ingest_cors_headers(response: &mut Response) {
    debug_assert!(
        response.headers.ends_with(b"\r\n\r\n"),
        "response headers must end with the header terminator before adding CORS headers"
    );
    let len = response.headers.len();
    response.headers.truncate(len - 2);
    use std::io::Write as _;
    let _ = write!(
        response.headers,
        "Access-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: {INGEST_CORS_ALLOWED_METHODS}\r\nAccess-Control-Allow-Headers: {INGEST_CORS_ALLOWED_HEADERS}\r\nAccess-Control-Max-Age: 86400\r\n",
    );
    response.headers.extend_from_slice(b"\r\n");
}

/// Value of the first cookie called `name` in a `Cookie` header, or `None`
/// when that value is empty. A later cookie with the same name is ignored, so
/// an empty first entry cannot be bypassed by appending another.
pub(crate) fn cookie_value<'a>(cookie_header: &'a str, name: &str) -> Option<&'a str> {
    cookie_header
        .split(';')
        .filter_map(|entry| entry.split_once('='))
        .find(|(entry_name, _)| entry_name.trim() == name)
        .map(|(_, value)| value.trim())
        .filter(|value| !value.is_empty())
}

/// Raw value of `name` in a request target's query string. A repeated
/// parameter resolves to its last non-empty value, as the report parsers that
/// loop over every pair do; earlier values are ignored, even malformed ones.
pub(crate) fn query_param<'a>(target: &'a str, name: &str) -> Option<&'a str> {
    let (_path, query) = target.split_once('?')?;
    query
        .rsplit('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, value)| *key == name && !value.is_empty())
        .map(|(_, value)| value)
}

/// Development serves the API over plain http on loopback, and a `Secure`
/// cookie set there is discarded by the browser. Production terminates TLS in
/// front of the process, so any non-loopback `Host` keeps the attribute.
/// Missing `Host` is treated as production.
pub(crate) fn host_supports_secure_cookies(host: Option<&str>) -> bool {
    !host
        .and_then(host_header_hostname)
        .is_some_and(is_loopback_host)
}

/// The `Domain` for a cookie the API sets for its web UI to read: the parent the
/// API's host and the request's origin share, as `revscope.co` for `api.revscope.co`
/// and `revscope.co`. `None` when they're the same host, which needs no `Domain`,
/// or when the API is addressed by IP or shares no parent, where the UI can't read
/// the cookie either way. A shared public suffix such as `co.uk` comes out too, and
/// the browser refuses the cookie, which only costs that cookie.
pub(crate) fn ui_cookie_domain<'a>(host: Option<&'a str>, origin: Option<&str>) -> Option<&'a str> {
    let hostname = host_header_hostname(host?)?;
    let (_, origin_host) = split_origin(origin?)?;
    if hostname.eq_ignore_ascii_case(origin_host) || hostname.parse::<IpAddr>().is_ok() {
        return None;
    }
    let shared: Vec<&str> = hostname
        .rsplit('.')
        .zip(origin_host.rsplit('.'))
        .take_while(|(ours, theirs)| ours.eq_ignore_ascii_case(theirs))
        .map(|(ours, _)| ours)
        .collect();
    if shared.len() < 2 {
        return None;
    }
    let shared_len = shared.iter().map(|label| label.len()).sum::<usize>() + shared.len() - 1;
    Some(&hostname[hostname.len() - shared_len..])
}

/// Hostname from a `Host` header value such as `localhost:4320` or `[::1]:4320`.
fn host_header_hostname(host: &str) -> Option<&str> {
    let host = host.trim();
    if host.is_empty() {
        return None;
    }
    let hostname = if let Some(stripped) = host.strip_prefix('[') {
        stripped.split(']').next().unwrap_or("")
    } else {
        host.split(':').next().unwrap_or("")
    };
    (!hostname.is_empty()).then_some(hostname)
}

/// Splits an origin into its scheme and host, e.g. `https://a.b:443` into
/// `("https", "a.b")`. `None` for anything that is not a usable origin.
fn split_origin(origin: &str) -> Option<(&str, &str)> {
    let origin = origin.trim();
    if origin.eq_ignore_ascii_case("null") {
        return None;
    }
    let (scheme, without_scheme) = origin.split_once("://")?;
    let authority = without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(without_scheme)
        .trim();
    let host = if let Some(stripped) = authority.strip_prefix('[') {
        stripped.split(']').next().unwrap_or("")
    } else {
        authority.split(':').next().unwrap_or("")
    };
    (!scheme.is_empty() && !host.is_empty()).then_some((scheme, host))
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// Access log written after the response has been fully sent.
///
/// Ingest traffic stays at `debug` so tracker hits do not flood production logs.
/// 500 details still go to `error!` (→ Sentry) via `Response::internal_error` in
/// the request handler.
pub(crate) fn log_completed_request(
    method: &str,
    target: &str,
    response: &Response,
    duration: Duration,
    is_ingest: bool,
) {
    if response.skip_access_log {
        return;
    }
    let Some((status_code, _)) = parse_status_line(&response.headers) else {
        return;
    };
    let body_bytes = response
        .file_body
        .as_ref()
        .map(|file_body| file_body.length)
        .unwrap_or(response.body.len() as u64);
    let size_kb = body_bytes as f64 / 1024.0;
    let duration_ms = duration.as_millis();
    if is_ingest {
        debug!("{method} {target} {status_code} {size_kb:.1}kb {duration_ms}ms");
    } else {
        info!("{method} {target} {status_code} {size_kb:.1}kb {duration_ms}ms");
    }
}

/// Query parameters that carry a credential and so must never be logged.
const REDACTED_QUERY_PARAMS: [&str; 2] = ["token", "session_id"];

/// Rewrites the values of [`REDACTED_QUERY_PARAMS`] so a request target can be
/// logged. Some clients cannot send an `Authorization` header (a browser
/// downloading a file by navigating to it, say) and pass a credential in the
/// query string instead.
pub(crate) fn redact_sensitive_query(target: &str) -> String {
    let Some((path, query)) = target.split_once('?') else {
        return target.to_string();
    };
    let mut redacted = String::with_capacity(target.len());
    redacted.push_str(path);
    redacted.push('?');
    for (index, pair) in query.split('&').enumerate() {
        if index > 0 {
            redacted.push('&');
        }
        let name = pair.split_once('=').map_or(pair, |(name, _value)| name);
        if REDACTED_QUERY_PARAMS.contains(&name) {
            redacted.push_str(name);
            redacted.push_str("=redacted");
        } else {
            redacted.push_str(pair);
        }
    }
    redacted
}

pub(crate) fn parse_status_line(response: &[u8]) -> Option<(u16, &str)> {
    let line_end = response.windows(2).position(|window| window == b"\r\n")?;
    let line = str::from_utf8(&response[..line_end]).ok()?;
    let mut parts = line.splitn(3, ' ');
    let _http_version = parts.next()?;
    let code = parts.next()?.parse().ok()?;
    let text = parts.next().unwrap_or("");
    Some((code, text))
}

/// Length of the body that follows `headers` (the request head without its
/// final blank line); no `Content-Length` means no body.
///
/// Works on bytes, not UTF-8, so a non-UTF-8 byte elsewhere in the head can't
/// hide the framing headers.
fn body_length(headers: &[u8]) -> Result<usize, BadFraming> {
    // A bare CR or LF is a line break to some parsers and not to others, and
    // the request-head parser only splits on CRLF.
    let bare_line_break = headers.iter().enumerate().any(|(index, &byte)| match byte {
        b'\r' => headers.get(index + 1) != Some(&b'\n'),
        b'\n' => index == 0 || headers[index - 1] != b'\r',
        _ => false,
    });
    if bare_line_break {
        return Err(BadFraming::Malformed);
    }
    let mut length = None;
    for line in headers.split(|&byte| byte == b'\n').skip(1) {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        // Obsolete line folding: an unfolding proxy would join this line onto
        // the previous header's value, which may be a framing header.
        if line
            .first()
            .is_some_and(|&byte| byte == b' ' || byte == b'\t')
        {
            return Err(BadFraming::Malformed);
        }
        let Some(colon) = line.iter().position(|&byte| byte == b':') else {
            continue;
        };
        let (name, value) = (&line[..colon], &line[colon + 1..]);
        if name.is_empty() || !name.iter().copied().all(is_token_byte) {
            return Err(BadFraming::Malformed);
        }
        if name.eq_ignore_ascii_case(b"transfer-encoding") {
            return Err(BadFraming::TransferEncoding);
        }
        if name.eq_ignore_ascii_case(b"content-length") {
            let value = value.trim_ascii();
            if value.is_empty() || !value.iter().all(u8::is_ascii_digit) {
                return Err(BadFraming::Malformed);
            }
            let parsed: usize = str::from_utf8(value)
                .ok()
                .and_then(|digits| digits.parse().ok())
                .ok_or(BadFraming::Malformed)?;
            if length.is_some_and(|previous| previous != parsed) {
                return Err(BadFraming::Malformed);
            }
            length = Some(parsed);
        }
    }
    Ok(length.unwrap_or(0))
}

/// RFC 9110 `tchar`, the bytes allowed in a header field name.
fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
}

fn find_header_end(request: &[u8]) -> Option<usize> {
    request.windows(4).position(|window| window == b"\r\n\r\n")
}

#[cfg(test)]
mod tests {
    #[test]
    fn query_param_takes_the_last_non_empty_value() {
        use super::query_param;
        assert_eq!(query_param("/a?x=1&y=2&x=3&x=", "x"), Some("3"));
        assert_eq!(query_param("/a?xx=1", "x"), None);
        assert_eq!(query_param("/a", "x"), None);
    }

    use super::*;

    #[test]
    fn negotiate_content_encoding_prefers_brotli_then_gzip() {
        assert_eq!(negotiate_content_encoding(None), ContentEncoding::Identity);
        assert_eq!(
            negotiate_content_encoding(Some("gzip, deflate")),
            ContentEncoding::Gzip
        );
        assert_eq!(
            negotiate_content_encoding(Some("gzip, deflate, br, zstd")),
            ContentEncoding::Brotli
        );
        assert_eq!(
            negotiate_content_encoding(Some("gzip;q=1.0, br;q=0.5")),
            ContentEncoding::Gzip
        );
        assert_eq!(
            negotiate_content_encoding(Some("x-gzip")),
            ContentEncoding::Gzip
        );
        assert_eq!(
            negotiate_content_encoding(Some("br;q=0, gzip;q=0")),
            ContentEncoding::Identity
        );
    }

    #[test]
    fn parses_accept_encoding_header() {
        let head =
            b"GET /script.js HTTP/1.1\r\nHost: x\r\nAccept-Encoding: gzip, deflate, br\r\n\r\n";
        let parsed = parse_request_head(head).expect("parse");
        assert_eq!(parsed.accept_encoding, Some("gzip, deflate, br"));
    }

    #[test]
    fn parses_user_agent_header() {
        let head =
            b"POST /demo HTTP/1.1\r\nHost: x\r\nuser-agent: Mozilla/5.0 (Windows NT 10.0)\r\n\r\n";
        let parsed = parse_request_head(head).expect("parse");
        assert_eq!(parsed.user_agent, Some("Mozilla/5.0 (Windows NT 10.0)"));

        let missing = parse_request_head(b"POST /demo HTTP/1.1\r\nHost: x\r\n\r\n").expect("parse");
        assert_eq!(missing.user_agent, None);
    }

    #[test]
    fn parses_accept_language_header() {
        let head =
            b"POST /demo HTTP/1.1\r\nHost: x\r\nAccept-Language: fr-CH, fr;q=0.9, en;q=0.8\r\n\r\n";
        let parsed = parse_request_head(head).expect("parse");
        assert_eq!(parsed.accept_language, Some("fr-CH, fr;q=0.9, en;q=0.8"));

        let missing = parse_request_head(b"POST /demo HTTP/1.1\r\nHost: x\r\n\r\n").expect("parse");
        assert_eq!(missing.accept_language, None);
    }

    #[test]
    fn parses_connection_close_header() {
        let close = parse_request_head(b"GET /x HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
            .expect("parse");
        assert!(close.connection_close);

        let keep_alive =
            parse_request_head(b"GET /x HTTP/1.1\r\nHost: x\r\nConnection: keep-alive\r\n\r\n")
                .expect("parse");
        assert!(!keep_alive.connection_close);

        let mixed = parse_request_head(
            b"GET /x HTTP/1.1\r\nHost: x\r\nConnection: keep-alive, close\r\n\r\n",
        )
        .expect("parse");
        assert!(mixed.connection_close);

        let omitted = parse_request_head(b"GET /x HTTP/1.1\r\nHost: x\r\n\r\n").expect("parse");
        assert!(!omitted.connection_close);
    }

    #[test]
    fn javascript_response_includes_encoding_and_vary() {
        let response = Response::javascript(b"console.log(1)", Some("br"));
        let text = String::from_utf8(response.headers.clone()).expect("utf8");
        assert!(text.contains("Cache-Control: public, max-age=14400\r\n"));
        assert!(text.contains("Content-Encoding: br\r\n"));
        assert!(text.contains("Vary: Accept-Encoding\r\n"));
        assert!(text.contains("Content-Length: 14\r\n"));
        assert!(
            !text.contains("Connection: close"),
            "keep-alive responses must omit Connection: close"
        );
        assert!(!response.closes_connection());
        assert_eq!(response.body, b"console.log(1)");
    }

    #[test]
    fn event_stream_omits_content_length_and_disables_buffering() {
        let (_sender, receiver) = mpsc::channel(1);
        let response = Response::event_stream(receiver);
        let text = String::from_utf8(response.headers.clone()).expect("utf8");
        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(text.contains("Content-Type: text/event-stream; charset=utf-8\r\n"));
        assert!(text.contains("Cache-Control: no-store\r\n"));
        assert!(text.contains("X-Accel-Buffering: no\r\n"));
        // The body length is unknown, so the connection close delimits it.
        assert!(!text.contains("Content-Length"));
        assert!(text.contains("Connection: close\r\n"));
        assert!(text.ends_with("\r\n\r\n"));
        assert!(response.stream_body.is_some());
    }

    #[test]
    fn event_stream_accepts_cors_headers() {
        let (_sender, receiver) = mpsc::channel(1);
        let mut response = Response::event_stream(receiver);
        with_cors_headers(
            &mut response,
            Some(&CorsHeaders {
                origin: "https://revscope.co".to_string(),
                request_headers: None,
            }),
        );
        let text = String::from_utf8(response.headers.clone()).expect("utf8");
        assert!(text.contains("Access-Control-Allow-Origin: https://revscope.co\r\n"));
        assert!(text.contains("Content-Type: text/event-stream; charset=utf-8\r\n"));
        assert!(text.ends_with("\r\n\r\n"));
    }

    #[test]
    fn route_not_found_serves_gzipped_html() {
        let response = Response::route_not_found();
        let text = String::from_utf8(response.headers.clone()).expect("utf8");
        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(text.contains("Content-Type: text/html\r\n"));
        assert!(text.contains("Content-Encoding: gzip\r\n"));
        assert!(text.contains("Cache-Control: no-store, no-cache, must-revalidate\r\n"));
        assert!(text.contains("Pragma: no-cache\r\n"));
        assert!(text.contains(&format!(
            "Content-Length: {}\r\n",
            ROUTE_NOT_FOUND_BODY_GZ.len()
        )));
        assert_eq!(response.body, ROUTE_NOT_FOUND_BODY_GZ);
        assert_eq!(&response.body[..2], &[0x1f, 0x8b]);
        assert!(response.skip_access_log);
    }

    #[test]
    fn parses_content_length_case_insensitive() {
        let headers = b"POST / HTTP/1.1\r\nHost: ingest.example.com\r\nContent-Length: 5";
        assert_eq!(body_length(headers), Ok(5));
        let headers_lower = b"POST / HTTP/1.1\r\ncontent-length: 9";
        assert_eq!(body_length(headers_lower), Ok(9));
    }

    #[test]
    fn body_length_is_zero_without_content_length_and_allows_identical_repeats() {
        assert_eq!(body_length(b"GET / HTTP/1.1\r\nHost: x"), Ok(0));
        assert_eq!(
            body_length(b"POST / HTTP/1.1\r\nContent-Length: 3\r\nContent-Length: 3"),
            Ok(3)
        );
    }

    #[test]
    fn body_length_rejects_any_transfer_encoding() {
        for head in [
            &b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked"[..],
            b"POST / HTTP/1.1\r\ntransfer-encoding: gzip, chunked",
            b"POST / HTTP/1.1\r\nContent-Length: 5\r\nTransfer-Encoding: chunked",
            b"POST / HTTP/1.1\r\nTransfer-Encoding: identity",
            b"POST / HTTP/1.1\r\nX-Bin: \xff\r\nTransfer-Encoding: chunked",
        ] {
            assert_eq!(
                body_length(head),
                Err(BadFraming::TransferEncoding),
                "{}",
                String::from_utf8_lossy(head)
            );
        }
    }

    #[test]
    fn body_length_rejects_ambiguous_content_length() {
        for head in [
            &b"POST / HTTP/1.1\r\nContent-Length: abc"[..],
            b"POST / HTTP/1.1\r\nContent-Length: +5",
            b"POST / HTTP/1.1\r\nContent-Length: -1",
            b"POST / HTTP/1.1\r\nContent-Length: 5, 5",
            b"POST / HTTP/1.1\r\nContent-Length:",
            b"POST / HTTP/1.1\r\nContent-Length: 99999999999999999999999",
            b"POST / HTTP/1.1\r\nContent-Length: 3\r\nContent-Length: 4",
            b"POST / HTTP/1.1\r\nContent-Length : 5",
            b"POST / HTTP/1.1\r\nX-A: 1\r\n Transfer-Encoding: chunked",
            b"POST / HTTP/1.1\r\nX-A: 1\nContent-Length: 5",
            b"POST / HTTP/1.1\r\nX-A: a\rTransfer-Encoding: chunked",
            b"POST / HTTP/1.1\r\n\nContent-Length: 5",
            b"POST / HTTP/1.1\r\nX-A: a\r",
            b"POST / HTTP/1.1\r\nContent-Length: 1\r\n 0",
            b"POST / HTTP/1.1\r\nContent-Length: 1\r\n\t0",
            b"POST / HTTP/1.1\r\nContent-Length: 5\r\nTransfer-Encoding\x0b: chunked",
            b"POST / HTTP/1.1\r\nTransfer-Encoding\0: chunked",
            b"POST / HTTP/1.1\r\n: empty-name",
        ] {
            assert_eq!(
                body_length(head),
                Err(BadFraming::Malformed),
                "{}",
                String::from_utf8_lossy(head)
            );
        }
    }

    use tokio::io::AsyncWriteExt;

    async fn read(
        stream: &mut (impl AsyncRead + Unpin),
        pending: &mut Vec<u8>,
    ) -> Result<Vec<u8>, ReadRequestError> {
        read_request_with_limits(
            stream,
            pending,
            1024 * 1024,
            Duration::from_secs(5),
            Duration::from_secs(5),
        )
        .await
    }

    #[tokio::test]
    async fn reader_rejects_transfer_encoding_that_arrives_in_a_later_read() {
        // `Chain` returns each slice from its own read, so the head is split.
        let mut stream = (&b"POST /x HTTP/1.1\r\nHost: x\r\n"[..])
            .chain(&b"Transfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n"[..]);
        let result = read(&mut stream, &mut Vec::new()).await;
        assert!(matches!(
            result,
            Err(ReadRequestError::BadFraming(BadFraming::TransferEncoding))
        ));
    }

    #[tokio::test]
    async fn reader_keeps_pipelined_requests_for_the_next_read() {
        let first =
            "POST /a HTTP/1.1\r\nContent-Length: 1500\r\n\r\n".to_string() + &"x".repeat(1500);
        let second = "GET /b HTTP/1.1\r\nHost: x\r\n\r\n";
        let third = "GET /c HTTP/1.1\r\nHost: x\r\n\r\n";
        let wire = format!("{first}{second}{third}");
        let mut stream = wire.as_bytes();
        let mut pending = Vec::new();
        for expected in [first.as_str(), second, third] {
            let request = read(&mut stream, &mut pending).await.expect("request");
            assert_eq!(String::from_utf8(request).unwrap(), expected);
        }
        assert!(pending.is_empty());
        assert!(matches!(
            read(&mut stream, &mut pending).await,
            Err(ReadRequestError::Closed)
        ));
    }

    #[tokio::test]
    async fn reader_skips_blank_lines_between_requests_under_the_idle_timeout() {
        let (mut client, mut server) = tokio::io::duplex(1024);
        client
            .write_all(b"POST /a HTTP/1.1\r\nContent-Length: 2\r\n\r\n{}\r\n")
            .await
            .unwrap();
        let mut pending = Vec::new();
        let first = read(&mut server, &mut pending).await.expect("first");
        assert!(first.ends_with(b"{}"));
        assert_eq!(pending, b"\r\n");

        let reader = tokio::spawn(async move {
            let result = read_request_with_limits(
                &mut server,
                &mut pending,
                1024,
                Duration::from_secs(5),
                Duration::from_millis(10),
            )
            .await;
            (result, server)
        });
        // Longer than the request timeout, so only the idle timeout may apply.
        tokio::time::sleep(Duration::from_millis(50)).await;
        client
            .write_all(b"\r\nGET /b HTTP/1.1\r\nHost: x\r\n\r\n")
            .await
            .unwrap();
        let (result, _server) = reader.await.unwrap();
        assert_eq!(
            result.expect("second"),
            b"GET /b HTTP/1.1\r\nHost: x\r\n\r\n"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn blank_lines_do_not_extend_the_idle_timeout() {
        let (mut client, mut server) = tokio::io::duplex(1024);
        let reader = tokio::spawn(async move {
            read_request_with_limits(
                &mut server,
                &mut Vec::new(),
                1024,
                Duration::from_secs(10),
                Duration::from_secs(2),
            )
            .await
        });
        let started = tokio::time::Instant::now();
        // Bounded so that a timer the blank lines reset fails the test, not hangs it.
        let trickle = tokio::spawn(async move {
            for _ in 0..30 {
                tokio::time::sleep(Duration::from_secs(1)).await;
                if client.write_all(b"\r\n").await.is_err() {
                    break;
                }
            }
        });

        let result = reader.await.unwrap();
        trickle.abort();
        assert!(matches!(result, Err(ReadRequestError::IdleTimeout)));
        assert_eq!(started.elapsed(), Duration::from_secs(10));
    }

    #[tokio::test]
    async fn reader_waits_out_a_blank_line_split_between_its_cr_and_lf() {
        let (mut client, mut server) = tokio::io::duplex(1024);
        let mut pending = b"\r".to_vec();
        let reader = tokio::spawn(async move {
            read_request_with_limits(
                &mut server,
                &mut pending,
                1024,
                Duration::from_secs(5),
                Duration::from_millis(10),
            )
            .await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        client
            .write_all(b"\nGET /a?x=1:2 HTTP/1.1\r\nHost: x\r\n\r\n")
            .await
            .unwrap();
        assert_eq!(
            reader.await.unwrap().expect("request"),
            b"GET /a?x=1:2 HTTP/1.1\r\nHost: x\r\n\r\n"
        );
    }

    #[tokio::test]
    async fn reader_treats_a_body_cut_short_by_close_as_closed() {
        let mut stream = &b"POST /x HTTP/1.1\r\nContent-Length: 100\r\n\r\n{\"a\":"[..];
        assert!(matches!(
            read(&mut stream, &mut Vec::new()).await,
            Err(ReadRequestError::Closed)
        ));
    }

    #[tokio::test]
    async fn reader_rejects_a_declared_length_over_the_limit_before_reading_it() {
        let mut stream = &b"POST /x HTTP/1.1\r\nContent-Length: 2000000\r\n\r\n"[..];
        assert!(matches!(
            read(&mut stream, &mut Vec::new()).await,
            Err(ReadRequestError::TooLarge)
        ));
    }

    #[test]
    fn content_length_overflowing_the_request_end_is_malformed() {
        let request = format!(
            "POST / HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            usize::MAX - 2
        );
        assert_eq!(
            find_request_end(request.as_bytes()),
            Err(BadFraming::Malformed)
        );
        assert_eq!(request_body(request.as_bytes()), None);
    }

    #[test]
    fn request_body_without_content_length_is_empty() {
        let request = b"POST / HTTP/1.1\r\nHost: x\r\n\r\nstray";
        assert_eq!(request_body(request), Some(&b""[..]));
    }

    #[test]
    fn extracts_body_from_request_with_content_length() {
        let request =
            b"POST / HTTP/1.1\r\nHost: ingest.example.com\r\nContent-Length: 7\r\n\r\npayload";
        assert_eq!(request_body(request), Some(&b"payload"[..]));
    }

    #[test]
    fn parses_client_ip_from_cf_connecting_ip() {
        let head = b"GET /i HTTP/1.1\r\nHost: x\r\nCF-Connecting-IP: 203.0.113.15\r\n\r\n";
        let parsed = parse_request_head(head).expect("parse");
        assert_eq!(parsed.client_ip, Some("203.0.113.15"));
    }

    #[test]
    fn parses_cf_ipcountry() {
        let head = b"GET /i HTTP/1.1\r\nHost: x\r\ncf-ipcountry: de\r\n\r\n";
        let parsed = parse_request_head(head).expect("parse");
        assert_eq!(parsed.cf_ipcountry, Some("de"));
    }

    #[test]
    fn trusts_cf_ipcountry_only_from_a_trusted_proxy() {
        assert_eq!(trusted_cf_ipcountry(Some("DE"), true), Some("DE"));
        assert_eq!(trusted_cf_ipcountry(Some("DE"), false), None);
        assert_eq!(trusted_cf_ipcountry(None, true), None);
    }

    #[test]
    fn client_ip_prefers_cf_connecting_ip_over_other_headers() {
        let head = b"GET /i HTTP/1.1\r\nHost: x\r\nX-Real-IP: 198.51.100.7\r\nX-Forwarded-For: 198.51.100.8, 10.0.0.1\r\nCF-Connecting-IP: 203.0.113.15\r\n\r\n";
        let parsed = parse_request_head(head).expect("parse");
        assert_eq!(parsed.client_ip, Some("203.0.113.15"));
    }

    #[test]
    fn client_ip_falls_back_to_cf_connecting_ipv6() {
        let head =
            b"GET /i HTTP/1.1\r\nHost: x\r\nX-Real-IP: 198.51.100.7\r\nCF-Connecting-IPv6: 2001:db8::1\r\n\r\n";
        let parsed = parse_request_head(head).expect("parse");
        assert_eq!(parsed.client_ip, Some("2001:db8::1"));
    }

    #[test]
    fn client_ip_falls_back_to_forwarded_for_then_real_ip() {
        let forwarded =
            b"GET /i HTTP/1.1\r\nHost: x\r\nX-Real-IP: 198.51.100.7\r\nX-Forwarded-For: 198.51.100.8, 10.0.0.1\r\n\r\n";
        let parsed = parse_request_head(forwarded).expect("parse");
        assert_eq!(parsed.client_ip, Some("198.51.100.8"));

        let real_ip = b"GET /i HTTP/1.1\r\nHost: x\r\nX-Real-IP: 198.51.100.7\r\n\r\n";
        let parsed = parse_request_head(real_ip).expect("parse");
        assert_eq!(parsed.client_ip, Some("198.51.100.7"));
    }

    #[test]
    fn parses_origin_and_preflight_headers() {
        let head = b"OPTIONS /v2/public/auth/login HTTP/1.1\r\nHost: api.revscope.dev\r\nCookie: rr_session=abc\r\nOrigin: http://localhost:4321\r\nAccess-Control-Request-Headers: Content-Type, X-Client-Version\r\n\r\n";
        let parsed = parse_request_head(head).expect("parse");
        assert_eq!(parsed.cookie, Some("rr_session=abc"));
        assert_eq!(parsed.origin, Some("http://localhost:4321"));
        assert_eq!(parsed.host, Some("api.revscope.dev"));
        assert_eq!(
            parsed.access_control_request_headers,
            Some("Content-Type, X-Client-Version")
        );
    }

    #[test]
    fn parses_last_x_forwarded_for_value() {
        let head = b"GET /i HTTP/1.1\r\nHost: x\r\nX-Forwarded-For: 203.0.113.1\r\nX-Forwarded-For: 198.51.100.12, 10.1.2.3\r\nCF-Connecting-IP: 203.0.113.15\r\n\r\n";
        let parsed = parse_request_head(head).expect("parse");
        assert_eq!(parsed.x_forwarded_for_last, Some("10.1.2.3"));

        let none = b"GET /i HTTP/1.1\r\nHost: x\r\nX-Real-IP: 198.51.100.7\r\n\r\n";
        let parsed = parse_request_head(none).expect("parse");
        assert_eq!(parsed.x_forwarded_for_last, None);
    }

    /// The visitor IP and country as the server resolves them for a request
    /// from `peer`.
    fn resolved_visitor<'a>(
        parsed: &ParsedRequest<'a>,
        peer: IpAddr,
        trust_x_forwarded_for: bool,
    ) -> (Option<IpAddr>, Option<&'a str>) {
        let headers_trusted = forwarded_headers_trusted(
            parsed,
            Some(peer),
            is_trusted_forwarding_proxy(peer),
            trust_x_forwarded_for,
        );
        (
            request_client_ip(parsed, Some(peer), headers_trusted, trust_x_forwarded_for),
            trusted_cf_ipcountry(parsed.cf_ipcountry, headers_trusted),
        )
    }

    #[test]
    fn trust_x_forwarded_for_uses_only_the_last_value_unless_cloudflare_vouches() {
        let head = b"GET /i HTTP/1.1\r\nHost: x\r\nX-Forwarded-For: 203.0.113.1, 198.51.100.12\r\nCF-Connecting-IP: 203.0.113.15\r\nCF-IPCountry: KP\r\n\r\n";
        let parsed = parse_request_head(head).expect("parse");
        let proxy: IpAddr = "172.18.0.2".parse().unwrap();
        let loopback: IpAddr = "127.0.0.1".parse().unwrap();
        let cloudflare: IpAddr = "173.245.48.1".parse().unwrap();
        let appended: IpAddr = "198.51.100.12".parse().unwrap();
        let forwarded: IpAddr = "203.0.113.15".parse().unwrap();

        assert_eq!(
            resolved_visitor(&parsed, proxy, true),
            (Some(appended), None)
        );
        assert_eq!(resolved_visitor(&parsed, proxy, false), (Some(proxy), None));
        // A proxy on the same host passes the client's own headers through.
        assert_eq!(
            resolved_visitor(&parsed, loopback, true),
            (Some(appended), None)
        );
        // Without the flag, loopback keeps its header precedence for local use.
        assert_eq!(
            resolved_visitor(&parsed, loopback, false),
            (Some(forwarded), Some("KP"))
        );
        // Cloudflare overwrites its headers, so they are believed from it.
        assert_eq!(
            resolved_visitor(&parsed, cloudflare, true),
            (Some(forwarded), Some("KP"))
        );

        let no_header = parse_request_head(b"GET /i HTTP/1.1\r\nHost: x\r\n\r\n").expect("parse");
        assert_eq!(
            resolved_visitor(&no_header, proxy, true),
            (Some(proxy), None)
        );
    }

    #[test]
    fn a_reverse_proxy_behind_cloudflare_takes_cloudflares_headers() {
        let head = b"GET /i HTTP/1.1\r\nHost: x\r\nX-Forwarded-For: 203.0.113.1, 173.245.48.1\r\nCF-Connecting-IP: 203.0.113.15\r\nCF-IPCountry: DE\r\n\r\n";
        let parsed = parse_request_head(head).expect("parse");
        let forwarded: IpAddr = "203.0.113.15".parse().unwrap();
        for peer in ["127.0.0.1", "172.18.0.2"] {
            assert_eq!(
                resolved_visitor(&parsed, peer.parse().unwrap(), true),
                (Some(forwarded), Some("DE")),
                "{peer}"
            );
        }
    }

    #[test]
    fn falls_back_to_x_forwarded_for_first_ip() {
        let head =
            b"GET /i HTTP/1.1\r\nHost: x\r\nX-Forwarded-For: 198.51.100.12, 10.1.2.3\r\n\r\n";
        let parsed = parse_request_head(head).expect("parse");
        assert_eq!(parsed.client_ip, Some("198.51.100.12"));
    }

    #[test]
    fn trusts_forwarded_ip_only_from_cloudflare_or_loopback() {
        let header = Some("203.0.113.15");
        let expected: IpAddr = "203.0.113.15".parse().unwrap();
        let cloudflare: IpAddr = "173.245.48.1".parse().unwrap();
        let internet: IpAddr = "198.51.100.1".parse().unwrap();
        let loopback: IpAddr = "127.0.0.1".parse().unwrap();

        assert_eq!(
            resolve_client_ip(header, Some(cloudflare), true),
            Some(expected)
        );
        assert_eq!(
            resolve_client_ip(header, Some(internet), false),
            Some(internet)
        );
        assert_eq!(
            resolve_client_ip(header, Some(loopback), true),
            Some(expected)
        );
        assert_eq!(resolve_client_ip(header, None, false), None);
    }

    #[test]
    fn cloudflare_ipv6_peer_trusts_headers() {
        let header = Some("2001:db8::1");
        let cloudflare: IpAddr = "2400:cb00::1".parse().unwrap();
        let expected: IpAddr = "2001:db8::1".parse().unwrap();
        assert_eq!(
            resolve_client_ip(header, Some(cloudflare), true),
            Some(expected)
        );
    }

    #[test]
    fn ipv4_mapped_cloudflare_peer_is_trusted() {
        let header = Some("203.0.113.15");
        let mapped: IpAddr = "::ffff:173.245.48.1".parse().unwrap();
        let expected: IpAddr = "203.0.113.15".parse().unwrap();
        assert_eq!(
            resolve_client_ip(header, Some(mapped), true),
            Some(expected)
        );
    }

    #[test]
    fn unparseable_header_falls_back_to_trusted_peer() {
        let cloudflare: IpAddr = "173.245.48.1".parse().unwrap();
        assert_eq!(
            resolve_client_ip(Some("not-an-ip"), Some(cloudflare), true),
            Some(cloudflare)
        );
    }

    #[test]
    fn cloudflare_cidr_boundaries() {
        assert!(is_trusted_forwarding_proxy("173.245.48.0".parse().unwrap()));
        assert!(is_trusted_forwarding_proxy(
            "173.245.63.255".parse().unwrap()
        ));
        assert!(!is_trusted_forwarding_proxy(
            "173.245.47.255".parse().unwrap()
        ));
        assert!(!is_trusted_forwarding_proxy(
            "173.245.64.0".parse().unwrap()
        ));
        assert!(is_trusted_forwarding_proxy("2400:cb00::1".parse().unwrap()));
        assert!(!is_trusted_forwarding_proxy(
            "2400:cb01::1".parse().unwrap()
        ));
        assert!(is_trusted_forwarding_proxy("::1".parse().unwrap()));
        assert!(!is_trusted_forwarding_proxy("10.0.0.1".parse().unwrap()));
    }

    #[test]
    fn allows_expected_cors_origins() {
        let allowed = CorsOrigins::parse("revscope.co,*.revscope.co");
        assert!(allowed.allows("http://localhost:4321"));
        assert!(allowed.allows("http://127.0.0.1:4321"));
        assert!(allowed.allows("http://[::1]:4321"));
        assert!(allowed.allows("https://revscope.co"));
        assert!(allowed.allows("https://dash.revscope.co"));
        assert!(allowed.allows("https://Dash.RevScope.co:443"));
    }

    #[test]
    fn parses_configured_cors_origins() {
        let allowed = CorsOrigins::parse(
            " https://app.example.com:8443/ , *.Example.org,http://*.corp.test, analytics.test:4322",
        );
        assert!(allowed.allows("https://app.example.com"));
        assert!(!allowed.allows("https://example.com"));
        assert!(!allowed.allows("https://other.example.com"));
        assert!(allowed.allows("https://a.example.org"));
        assert!(allowed.allows("https://a.b.example.org"));
        assert!(!allowed.allows("https://example.org"));
        assert!(!allowed.allows("https://badexample.org"));
        assert!(allowed.allows("https://x.corp.test"));
        assert!(allowed.allows("http://analytics.test:9999"));
    }

    #[test]
    fn empty_cors_origins_allow_only_loopback() {
        let allowed = CorsOrigins::parse("");
        assert!(allowed.allows("http://localhost:4322"));
        assert!(!allowed.allows("https://revscope.co"));
        assert!(!allowed.allows("https://app.example.com"));
    }

    #[test]
    fn default_cors_origins_depend_on_whether_the_ui_is_bundled() {
        let expected = if cfg!(feature = "self-hosted") {
            CorsOrigins::parse("")
        } else {
            CorsOrigins::parse("revscope.co,*.revscope.co")
        };
        assert_eq!(CorsOrigins::build_default(), expected);
    }

    #[test]
    fn cookie_value_reads_the_first_cookie_with_the_name() {
        assert_eq!(cookie_value("a=1; rsdemo=abc; b=2", "rsdemo"), Some("abc"));
        assert_eq!(cookie_value(" rsdemo = abc ", "rsdemo"), Some("abc"));
        assert_eq!(
            cookie_value("rsdemo=abc; rsdemo=def", "rsdemo"),
            Some("abc")
        );
        assert_eq!(cookie_value("rsdemo=; rsdemo=abc", "rsdemo"), None);
        assert_eq!(cookie_value("xrsdemo=abc", "rsdemo"), None);
        assert_eq!(cookie_value("rsdemo", "rsdemo"), None);
    }

    #[test]
    fn secure_cookies_require_a_non_loopback_host() {
        assert!(host_supports_secure_cookies(Some("api.revscope.co")));
        assert!(host_supports_secure_cookies(Some("dash.revscope.co")));
        assert!(host_supports_secure_cookies(None));
        assert!(!host_supports_secure_cookies(Some("localhost")));
        assert!(!host_supports_secure_cookies(Some("localhost:4320")));
        assert!(!host_supports_secure_cookies(Some("127.0.0.1:4320")));
        assert!(!host_supports_secure_cookies(Some("[::1]:4320")));
    }

    #[test]
    fn ui_cookie_domain_is_the_parent_of_the_api_host_and_the_origin() {
        let domain = |host, origin| ui_cookie_domain(Some(host), Some(origin));
        let revscope = Some("revscope.co");
        assert_eq!(domain("api.revscope.co", "https://revscope.co"), revscope);
        assert_eq!(
            domain("API.revscope.co:443", "https://revscope.co"),
            revscope
        );
        assert_eq!(
            domain("api.revscope.co", "https://www.revscope.co"),
            revscope
        );
        assert_eq!(domain("revscope.co", "https://www.revscope.co"), revscope);
        assert_eq!(
            domain("revscope-api.example.com", "https://revscope.example.com"),
            Some("example.com")
        );
        assert_eq!(
            domain("api.a.example.com", "https://ui.a.example.com"),
            Some("a.example.com")
        );
    }

    #[test]
    fn ui_cookie_domain_is_none_without_a_shared_parent() {
        let domain = |host, origin| ui_cookie_domain(Some(host), Some(origin));
        assert_eq!(domain("api.revscope.co", "https://notrevscope.co"), None);
        assert_eq!(
            domain("api.revscope.co", "https://revscope.co.evil.com"),
            None
        );
        assert_eq!(domain("api.revscope.co", "null"), None);
        assert_eq!(domain("api.example.com", "https://api.example.com"), None);
        assert_eq!(domain("localhost:4320", "http://localhost:4322"), None);
        assert_eq!(domain("11.0.0.1:4320", "http://10.0.0.1"), None);
        assert_eq!(domain("[::1]:4320", "http://localhost:4322"), None);
        assert_eq!(ui_cookie_domain(Some("api.revscope.co"), None), None);
        assert_eq!(ui_cookie_domain(None, Some("https://revscope.co")), None);
    }

    #[test]
    fn rejects_unexpected_cors_origins() {
        let allowed = CorsOrigins::parse("revscope.co,*.revscope.co");
        assert!(!allowed.allows("https://revscope.dev"));
        assert!(!allowed.allows("https://app.revscope.dev"));
        assert!(!allowed.allows("https://evil.example.com"));
        assert!(!allowed.allows("https://revscope.co.evil.com"));
        assert!(!allowed.allows("null"));
    }

    #[test]
    fn preflight_uses_no_content_for_allowed_origin() {
        let response = preflight_response(Some(&CorsHeaders {
            origin: "http://localhost:3000".to_string(),
            request_headers: Some("Content-Type, X-Test".to_string()),
        }));
        let text = String::from_utf8(response.headers.clone()).expect("utf8");
        assert!(text.starts_with("HTTP/1.1 204 No Content"));
        assert!(text.contains("Access-Control-Allow-Origin: http://localhost:3000"));
        assert!(text.contains("Access-Control-Allow-Headers: Content-Type, X-Test"));
        assert!(text.ends_with("\r\n\r\n"));
        assert!(response.body.is_empty());
    }

    #[test]
    fn preflight_rejects_disallowed_origin() {
        let response = preflight_response(None);
        let text = String::from_utf8(response.headers.clone()).expect("utf8");
        assert!(text.starts_with("HTTP/1.1 403 Forbidden"));
        assert!(!text.contains("Access-Control-Allow-Origin"));
    }

    #[test]
    fn forbidden_closes_the_connection() {
        let response = Response::forbidden();
        let text = String::from_utf8(response.headers.clone()).expect("utf8");
        assert!(text.starts_with("HTTP/1.1 403 Forbidden"));
        assert!(text.contains("Connection: close\r\n"));
        assert!(response.closes_connection());
        assert!(response.body.is_empty());
    }

    #[test]
    fn service_unavailable_asks_the_client_to_retry_in_five_seconds() {
        let response = Response::service_unavailable();
        let text = String::from_utf8(response.headers.clone()).expect("utf8");
        assert!(text.starts_with("HTTP/1.1 503 Service Unavailable"));
        assert!(text.contains("Retry-After: 5\r\n"));
        assert!(!response.closes_connection());
        assert!(response.body.is_empty());
    }

    #[test]
    fn redacts_only_credential_query_parameters() {
        assert_eq!(
            redact_sensitive_query("/v2/download/projects/7?token=abc-def"),
            "/v2/download/projects/7?token=redacted"
        );
        assert_eq!(
            redact_sensitive_query("/v2/download/projects/7?session_id=abc&start=1"),
            "/v2/download/projects/7?session_id=redacted&start=1"
        );
        assert_eq!(
            redact_sensitive_query("/v2/read/projects/1/overview?start=1&end=2"),
            "/v2/read/projects/1/overview?start=1&end=2"
        );
        assert_eq!(redact_sensitive_query("/v2/session/me"), "/v2/session/me");
    }

    #[test]
    fn parses_status_line_with_reason_text() {
        let parsed =
            parse_status_line(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n");
        assert_eq!(parsed, Some((500, "Internal Server Error")));
    }

    #[test]
    fn with_cors_headers_preserves_body_and_appends_headers() {
        let mut response = Response::json("200 OK", br#"{"ok":true}"#.to_vec());
        let body_before = response.body.clone();
        with_cors_headers(
            &mut response,
            Some(&CorsHeaders {
                origin: "http://localhost:4321".to_string(),
                request_headers: None,
            }),
        );
        assert_eq!(response.body, body_before, "body must not be reallocated");
        assert!(response.headers.ends_with(b"\r\n\r\n"));
        let text = String::from_utf8(response.headers.clone()).expect("utf8");
        assert!(text.contains("Access-Control-Allow-Origin: http://localhost:4321"));
        assert!(text.contains("Access-Control-Allow-Headers: Content-Type, Authorization"));
    }

    #[test]
    fn ingest_cors_uses_wildcard_without_credentials() {
        let mut response = Response::json("200 OK", br#"{"ok":true}"#.to_vec());
        let body_before = response.body.clone();
        with_ingest_cors_headers(&mut response);
        assert_eq!(response.body, body_before, "body must not be reallocated");
        assert!(response.headers.ends_with(b"\r\n\r\n"));
        let text = String::from_utf8(response.headers.clone()).expect("utf8");
        assert!(text.contains("Access-Control-Allow-Origin: *"));
        assert!(text.contains("Access-Control-Allow-Methods: GET, POST, OPTIONS, HEAD"));
        assert!(text.contains("Access-Control-Allow-Headers: Content-Type"));
        assert!(!text.contains("Access-Control-Allow-Credentials"));
        assert!(!text.contains("Vary: Origin"));
    }

    #[test]
    fn ingest_preflight_always_allows() {
        let response = ingest_preflight_response();
        let text = String::from_utf8(response.headers.clone()).expect("utf8");
        assert!(text.starts_with("HTTP/1.1 204 No Content"));
        assert!(text.contains("Access-Control-Allow-Origin: *"));
        assert!(!text.contains("Access-Control-Allow-Credentials"));
        assert!(text.ends_with("\r\n\r\n"));
        assert!(response.body.is_empty());
    }
}
