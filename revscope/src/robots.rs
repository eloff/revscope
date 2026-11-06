//! `robots.txt` for this process.
//!
//! The same listener answers `api.revscope.co`, `in.revscope.co`,
//! `eu1.revscope.co`, and customer CNAMEs. None of those hosts are pages, so
//! every one of them gets the same crawl policy. The file is not access
//! control: an MCP client opens `/v2/mcp` directly and never reads it.

use crate::http::{ParsedRequest, Response};

const PATH: &str = "/robots.txt";
const BODY: &str = "User-agent: *\nDisallow: /\n";

/// The crawl policy, or `None` when this request is not for it. Other methods
/// stay on the normal router so a POST here is not mistaken for a document.
pub(crate) fn response(parsed: &ParsedRequest<'_>) -> Option<Response> {
    let path = parsed.target.split('?').next().unwrap_or(parsed.target);
    if path != PATH || (parsed.method != "GET" && parsed.method != "HEAD") {
        return None;
    }
    let mut response =
        Response::with_content_type("200 OK", "text/plain; charset=utf-8", BODY.as_bytes(), &[]);
    // HEAD keeps the length header and omits the body.
    if parsed.method == "HEAD" {
        response.body.clear();
    }
    Some(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::parse_request_head;

    fn respond(head: &str) -> Option<Response> {
        let raw = head.as_bytes().to_vec();
        let parsed = parse_request_head(&raw).expect("parse");
        response(&parsed)
    }

    #[test]
    fn get_disallows_the_whole_host() {
        for host in ["api.revscope.co", "in.revscope.co", "eu1.revscope.co"] {
            let response = respond(&format!("GET /robots.txt HTTP/1.1\r\nHost: {host}\r\n\r\n"))
                .unwrap_or_else(|| panic!("{host} should serve robots.txt"));
            let headers = String::from_utf8(response.headers).expect("utf8");
            assert!(headers.starts_with("HTTP/1.1 200 OK"), "{host}");
            assert!(
                headers.contains("Content-Type: text/plain; charset=utf-8"),
                "{host}"
            );
            assert_eq!(response.body, b"User-agent: *\nDisallow: /\n");
        }
    }

    #[test]
    fn head_keeps_the_length_and_drops_the_body() {
        let response =
            respond("HEAD /robots.txt?from=crawler HTTP/1.1\r\nHost: eu1.revscope.co\r\n\r\n")
                .expect("robots.txt");
        let headers = String::from_utf8(response.headers).expect("utf8");
        assert!(headers.contains(&format!("Content-Length: {}\r\n", BODY.len())));
        assert!(response.body.is_empty());
    }

    #[test]
    fn other_requests_are_left_alone() {
        assert!(respond("POST /robots.txt HTTP/1.1\r\nHost: api.revscope.co\r\n\r\n").is_none());
        assert!(respond("GET /robots.txt/ HTTP/1.1\r\nHost: api.revscope.co\r\n\r\n").is_none());
        assert!(respond("GET /v2/mcp HTTP/1.1\r\nHost: api.revscope.co\r\n\r\n").is_none());
    }
}
