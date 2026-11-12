//! Headers that limit what a browser lets other sites do with our responses:
//! sniff them into scripts, frame them, read our URLs from `Referer`, or reach
//! us over plain HTTP.
//!
//! Ingest responses get none of them. Customers serve the tracker and its
//! endpoints through their own domain with a CNAME or a path proxy, and a proxy
//! that passes our headers along would set them on the customer's site.

use std::net::IpAddr;

use crate::http::{Response, host_header_hostname, split_origin};

/// The bundled UI's embeddable pages, which only our own origin frames.
const EMBED_PATH_PREFIX: &str = "/ui/embed";
const HSTS_VALUE: &str = "max-age=31536000";

/// Hostnames this instance is publicly reached on over HTTPS, the only ones
/// it sends `Strict-Transport-Security` for. HSTS covers every port of a host
/// and can't be undone before it expires, so it is never sent for customers'
/// domains, loopback, or a host only `Host` vouches for.
#[derive(Debug, Default)]
pub(crate) struct HstsHosts(Vec<String>);

impl HstsHosts {
    /// The hosts of `REVSCOPE_WEB_BASE_URL` and `REVSCOPE_API_BASE_URL` when
    /// they are `https`, and `REVSCOPE_TLS_DOMAINS` when `REVSCOPE_LISTEN_TLS`
    /// has this process terminate TLS itself.
    pub(crate) fn from_env() -> Self {
        let serves_tls =
            std::env::var("REVSCOPE_LISTEN_TLS").is_ok_and(|value| !value.trim().is_empty());
        let tls_domains = if serves_tls {
            crate::acme::platform_domains_from(
                &std::env::var("REVSCOPE_TLS_DOMAINS").unwrap_or_default(),
            )
        } else {
            Vec::new()
        };
        Self::from_parts(
            &[
                Some(crate::util::web_base_url()),
                crate::mcp::configured_api_base_url(),
            ],
            tls_domains,
        )
    }

    pub(crate) fn from_parts(base_urls: &[Option<String>], tls_domains: Vec<String>) -> Self {
        let mut hosts = Vec::new();
        let https_hosts = base_urls
            .iter()
            .flatten()
            .filter_map(|url| split_origin(url))
            .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("https"))
            .map(|(_, host)| host.to_string());
        for host in https_hosts.chain(tls_domains) {
            let host = normalize_host(&host);
            // Browsers ignore HSTS for IP addresses (RFC 6797 section 8.1.1), and on
            // localhost it would break every other local server, whatever its port.
            let named = host != "localhost" && host.parse::<IpAddr>().is_err();
            if named && !hosts.contains(&host) {
                hosts.push(host);
            }
        }
        Self(hosts)
    }

    fn covers(&self, host: Option<&str>) -> bool {
        host.and_then(host_header_hostname)
            .map(normalize_host)
            .is_some_and(|host| self.0.contains(&host))
    }
}

fn normalize_host(host: &str) -> String {
    host.trim().trim_end_matches('.').to_ascii_lowercase()
}

/// Appends the security headers to a non-ingest response for `target`, as
/// requested on `host`.
pub(crate) fn with_security_headers(
    response: &mut Response,
    target: &str,
    host: Option<&str>,
    hsts_hosts: &HstsHosts,
) {
    debug_assert!(
        response.headers.ends_with(b"\r\n\r\n"),
        "response headers must end with the header terminator before adding security headers"
    );
    let path = target.split(['?', '#']).next().unwrap_or(target);
    let embeddable = path
        .strip_prefix(EMBED_PATH_PREFIX)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'));
    // Both framing headers, because browsers that predate frame-ancestors
    // still honour X-Frame-Options.
    let (frame_ancestors, frame_options) = if embeddable {
        ("'self'", "SAMEORIGIN")
    } else {
        ("'none'", "DENY")
    };
    let len = response.headers.len();
    response.headers.truncate(len - 2);
    use std::io::Write as _;
    let _ = write!(
        response.headers,
        "X-Content-Type-Options: nosniff\r\nContent-Security-Policy: frame-ancestors {frame_ancestors}\r\nX-Frame-Options: {frame_options}\r\nReferrer-Policy: strict-origin-when-cross-origin\r\n",
    );
    if hsts_hosts.covers(host) {
        let _ = write!(
            response.headers,
            "Strict-Transport-Security: {HSTS_VALUE}\r\n"
        );
    }
    response.headers.extend_from_slice(b"\r\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_for(target: &str, host: Option<&str>, hsts_hosts: &HstsHosts) -> String {
        let mut response = Response::json("200 OK", b"{}".to_vec());
        with_security_headers(&mut response, target, host, hsts_hosts);
        assert!(response.headers.ends_with(b"\r\n\r\n"));
        assert_eq!(response.body, b"{}");
        String::from_utf8(response.headers).expect("utf8")
    }

    fn hosts(base_urls: &[&str], tls_domains: &[&str]) -> HstsHosts {
        HstsHosts::from_parts(
            &base_urls
                .iter()
                .map(|url| Some(url.to_string()))
                .collect::<Vec<_>>(),
            tls_domains
                .iter()
                .map(|domain| domain.to_string())
                .collect(),
        )
    }

    #[test]
    fn every_response_gets_nosniff_referrer_policy_and_no_framing() {
        let headers = headers_for("/v2/me?x=1", Some("api.example.com"), &HstsHosts::default());
        assert!(headers.contains("\r\nX-Content-Type-Options: nosniff\r\n"));
        assert!(headers.contains("\r\nContent-Security-Policy: frame-ancestors 'none'\r\n"));
        assert!(headers.contains("\r\nX-Frame-Options: DENY\r\n"));
        assert!(headers.contains("\r\nReferrer-Policy: strict-origin-when-cross-origin\r\n"));
        assert!(!headers.contains("Strict-Transport-Security"));
    }

    #[test]
    fn embed_pages_may_be_framed_by_their_own_origin() {
        for target in ["/ui/embed", "/ui/embed/", "/ui/embed/runway?x=1"] {
            let headers = headers_for(target, None, &HstsHosts::default());
            assert!(
                headers.contains("Content-Security-Policy: frame-ancestors 'self'\r\n"),
                "{target}"
            );
            assert!(
                headers.contains("X-Frame-Options: SAMEORIGIN\r\n"),
                "{target}"
            );
        }
        let headers = headers_for("/ui/embedded", None, &HstsHosts::default());
        assert!(headers.contains("frame-ancestors 'none'"));
    }

    #[test]
    fn hsts_is_sent_only_for_configured_https_hosts() {
        let hsts_hosts = hosts(
            &[
                "https://Analytics.Example.com/ui",
                "http://plain.example.com",
                "https://localhost:4320",
                "https://127.0.0.1",
                "https://[::1]:4320/ui",
            ],
            &["api.example.net."],
        );
        for host in [
            "analytics.example.com",
            "ANALYTICS.example.com:443",
            "api.example.net",
        ] {
            let headers = headers_for("/ui/dashboard", Some(host), &hsts_hosts);
            assert!(
                headers.contains("\r\nStrict-Transport-Security: max-age=31536000\r\n"),
                "{host}"
            );
        }
        for host in [
            Some("plain.example.com"),
            Some("localhost:4320"),
            Some("127.0.0.1"),
            Some("[::1]:4320"),
            Some("stats.customer.example"),
            Some("sub.analytics.example.com"),
            None,
        ] {
            let headers = headers_for("/ui/dashboard", host, &hsts_hosts);
            assert!(!headers.contains("Strict-Transport-Security"), "{host:?}");
        }
    }
}
