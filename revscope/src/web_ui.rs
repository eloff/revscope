//! The product UI, embedded in the binary for self-hosting.
//!
//! `pnpm build:bundle` in `web/` writes a static build to `web/app/dist-ui`,
//! and the `self-hosted` cargo feature compiles it in. Without the feature nothing
//! here answers, and the UI is deployed separately (as on revscope.co).
//!
//! build.rs stores text files only gzipped (`{name}.gz`), so they are always
//! sent with `Content-Encoding: gzip` whatever `Accept-Encoding` says; every
//! browser the app supports accepts it.
//!
//! Everything lives under `/ui` because ingest owns the root: any single path
//! segment that is a valid project name, `/dashboard` included, is a tracker
//! endpoint. Project names are at least 3 characters, so `ui` is never one.

use include_dir::Dir;

use crate::http::{ParsedRequest, Response};

#[cfg(feature = "self-hosted")]
static DIST: Dir<'_> = include_dir::include_dir!("$OUT_DIR/web-ui");

const PREFIX: &str = "/ui";
const ENTRY_PATH: &str = "/ui/dashboard";
/// Astro puts every content-hashed build asset here, so they never change.
const IMMUTABLE_DIR: &str = "_astro/";

/// A page or asset of the bundled UI, or `None` when this request is not for
/// one. Only GET and HEAD are answered; other methods stay on the router.
#[cfg(feature = "self-hosted")]
pub(crate) fn response(parsed: &ParsedRequest<'_>) -> Option<Response> {
    response_from(&DIST, parsed)
}

#[cfg(not(feature = "self-hosted"))]
pub(crate) fn response(_parsed: &ParsedRequest<'_>) -> Option<Response> {
    None
}

#[cfg_attr(not(any(feature = "self-hosted", test)), allow(dead_code))]
fn response_from(dist: &Dir<'static>, parsed: &ParsedRequest<'_>) -> Option<Response> {
    if parsed.method != "GET" && parsed.method != "HEAD" {
        return None;
    }
    let path = parsed.target.split('?').next().unwrap_or(parsed.target);
    if path == "/" || path == PREFIX || path == "/ui/" {
        return Some(redirect(ENTRY_PATH));
    }
    let rest = path.strip_prefix("/ui/")?;
    let Some(file) = find(dist, rest) else {
        return Some(Response::not_found());
    };
    let cache_control = if file.path.starts_with(IMMUTABLE_DIR) {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    let mut headers = vec![("Cache-Control", cache_control.to_string())];
    if file.gzipped {
        headers.push(("Content-Encoding", "gzip".to_string()));
    }
    let mut response =
        Response::with_content_type("200 OK", content_type(file.path), file.contents, &headers);
    // HEAD keeps the length header and omits the body.
    if parsed.method == "HEAD" {
        response.body.clear();
    }
    Some(response)
}

struct BundleFile {
    /// Path as requested, without the `.gz` it may be stored under.
    path: &'static str,
    contents: &'static [u8],
    gzipped: bool,
}

/// Resolves a path below `/ui/` the way Astro lays out a static build: a file
/// as named, otherwise the page directory's `index.html`.
fn find(dist: &Dir<'static>, rest: &str) -> Option<BundleFile> {
    let rest = rest.trim_end_matches('/');
    // `.gz` names are build.rs's storage, not files Astro emitted.
    if rest.ends_with(".gz")
        || rest
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return None;
    }
    [rest.to_string(), format!("{rest}/index.html")]
        .iter()
        .find_map(|candidate| stored_file(dist, candidate))
}

fn stored_file(dist: &Dir<'static>, path: &str) -> Option<BundleFile> {
    if let Some(file) = dist.get_file(format!("{path}.gz")) {
        let stored = file.path().to_str()?;
        return Some(BundleFile {
            path: stored.strip_suffix(".gz")?,
            contents: file.contents(),
            gzipped: true,
        });
    }
    let file = dist.get_file(path)?;
    Some(BundleFile {
        path: file.path().to_str()?,
        contents: file.contents(),
        gzipped: false,
    })
}

fn redirect(location: &str) -> Response {
    Response::new(
        format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\n\r\n")
            .into_bytes(),
        Vec::new(),
    )
}

fn content_type(path: &str) -> &'static str {
    let extension = path.rsplit_once('.').map_or("", |(_, extension)| extension);
    match extension {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use include_dir::{DirEntry, File};

    use super::*;
    use crate::http::parse_request_head;

    /// Laid out like build.rs's output; the "gzipped" bytes are stand-ins.
    static TEST_DIST: Dir<'static> = Dir::new(
        "",
        &[
            DirEntry::File(File::new("favicon.ico.gz", b"icon")),
            DirEntry::Dir(Dir::new(
                "dashboard",
                &[DirEntry::File(File::new(
                    "dashboard/index.html.gz",
                    b"<html>dashboard</html>",
                ))],
            )),
            DirEntry::Dir(Dir::new(
                "_astro",
                &[
                    DirEntry::File(File::new("_astro/client.abc123.js.gz", b"js")),
                    DirEntry::File(File::new("_astro/logo.abc123.webp", b"webp")),
                ],
            )),
        ],
    );

    fn respond(head: &str) -> Option<Response> {
        let raw = head.as_bytes().to_vec();
        let parsed = parse_request_head(&raw).expect("parse");
        response_from(&TEST_DIST, &parsed)
    }

    fn headers(response: &Response) -> String {
        String::from_utf8(response.headers.clone()).expect("utf8")
    }

    #[test]
    fn serves_page_directories_as_their_index() {
        for target in ["/ui/dashboard", "/ui/dashboard/", "/ui/dashboard?tab=pages"] {
            let response = respond(&format!("GET {target} HTTP/1.1\r\nHost: a.example\r\n\r\n"))
                .unwrap_or_else(|| panic!("{target} should be served"));
            let headers = headers(&response);
            assert!(headers.starts_with("HTTP/1.1 200 OK"), "{target}");
            assert!(headers.contains("Content-Type: text/html; charset=utf-8"));
            assert!(headers.contains("Content-Encoding: gzip\r\n"));
            assert!(headers.contains("Cache-Control: no-cache"));
            assert_eq!(response.body, b"<html>dashboard</html>");
        }
    }

    #[test]
    fn hashed_assets_are_cached_forever() {
        let response = respond("GET /ui/_astro/client.abc123.js HTTP/1.1\r\n\r\n").expect("asset");
        let headers = headers(&response);
        assert!(headers.contains("Content-Type: text/javascript; charset=utf-8"));
        assert!(headers.contains("Content-Encoding: gzip\r\n"));
        assert!(headers.contains("Cache-Control: public, max-age=31536000, immutable"));
        assert_eq!(response.body, b"js");
    }

    #[test]
    fn compressed_formats_are_sent_as_stored() {
        let response = respond("GET /ui/_astro/logo.abc123.webp HTTP/1.1\r\n\r\n").expect("image");
        let headers = headers(&response);
        assert!(headers.contains("Content-Type: image/webp"));
        assert!(!headers.contains("Content-Encoding"));
        assert_eq!(response.body, b"webp");
    }

    #[test]
    fn stored_gz_names_are_not_requestable() {
        let response = respond("GET /ui/favicon.ico.gz HTTP/1.1\r\n\r\n").expect("404");
        assert!(headers(&response).starts_with("HTTP/1.1 404"));
    }

    #[test]
    fn root_and_prefix_redirect_to_the_dashboard() {
        for target in ["/", "/ui", "/ui/"] {
            let response = respond(&format!("GET {target} HTTP/1.1\r\n\r\n")).expect("redirect");
            let headers = headers(&response);
            assert!(headers.starts_with("HTTP/1.1 302 Found"), "{target}");
            assert!(headers.contains("Location: /ui/dashboard\r\n"), "{target}");
        }
    }

    #[test]
    fn unknown_paths_under_the_prefix_are_not_found() {
        for target in ["/ui/missing", "/ui/../favicon.ico", "/ui//favicon.ico"] {
            let response = respond(&format!("GET {target} HTTP/1.1\r\n\r\n")).expect("404");
            assert!(headers(&response).starts_with("HTTP/1.1 404"), "{target}");
        }
    }

    #[test]
    fn head_keeps_the_length_and_drops_the_body() {
        let response = respond("HEAD /ui/favicon.ico HTTP/1.1\r\n\r\n").expect("favicon");
        assert!(headers(&response).contains("Content-Length: 4\r\n"));
        assert!(response.body.is_empty());
    }

    #[test]
    fn other_requests_are_left_alone() {
        assert!(respond("POST /ui/dashboard HTTP/1.1\r\n\r\n").is_none());
        assert!(respond("POST / HTTP/1.1\r\n\r\n").is_none());
        assert!(respond("GET /dashboard HTTP/1.1\r\n\r\n").is_none());
        assert!(respond("GET /uiproject HTTP/1.1\r\n\r\n").is_none());
        assert!(respond("GET /v2/session/me HTTP/1.1\r\n\r\n").is_none());
    }
}
