//! Installation and data-pipeline checks for a project.
//!
//! `GET` returns a status summary (default site URL, recent ingest volume,
//! integrations). `POST .../check` fetches a URL server-side (CORS) and
//! inspects the HTML for the tracker stub. `GET .../ingest` looks for a real tracker
//! hit after the user visits their site: in the project's in-memory ingest batch,
//! then in its database, since a flush can land between two polls.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use regex::Regex;
pub(crate) use revscope_api::troubleshooting::TroubleshootingCheckRequest;
use revscope_api::troubleshooting::{
    FetchResult, InstallFinding, InstallResult, IntegrationStatus, Severity, SyncStatus,
    TroubleshootingCheckResponse, TroubleshootingIngestQuery, TroubleshootingIngestResponse,
    TroubleshootingStatusResponse,
};
use tokio::net::lookup_host;
use url::{Host, Url};

use crate::auth::AuthError;
use crate::authorize::OwnedProject;
use crate::db::{open_main_database_read, open_project_database_read};
use crate::projects;
use crate::server_context::ServerContext;
use crate::settings::{self, IntegrationView};
use crate::util::{block_on, nanoseconds_since_epoch};

const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;
const MONTH_NANOS: i64 = 30 * 24 * 60 * 60 * 1_000_000_000;
const NANOS_PER_MILLI: i64 = 1_000_000;

/// Parses `/v2/session/projects/{id}/troubleshooting/ingest?since_ms=...`.
pub(crate) fn parse_ingest_query(target: &str) -> Result<TroubleshootingIngestQuery, &'static str> {
    let since_ms: i64 = crate::http::query_param(target, "since_ms")
        .and_then(|value| value.parse().ok())
        .ok_or("since_ms is required")?;
    if since_ms < 0 {
        return Err("since_ms must be non-negative");
    }
    Ok(TroubleshootingIngestQuery {
        since_ms,
        hostname: crate::http::query_param(target, "hostname").map(|value| value.to_string()),
    })
}

pub(crate) fn get_status(
    server_context: Arc<ServerContext>,
    project: &OwnedProject,
) -> Result<TroubleshootingStatusResponse, AuthError> {
    let project_key = project.key();

    let project_row = projects::fetch_project_blocking(server_context.clone(), project_key)
        .map_err(|error| AuthError::Db(error.to_string()))?
        .ok_or(AuthError::InvalidRequest("project not found"))?;

    let settings = settings::get_settings(server_context.clone(), project)?;
    let integrations = settings
        .integrations
        .into_iter()
        .filter(IntegrationView::is_connected)
        .map(integration_status)
        .collect();

    let events_queued = queued_events(&server_context, project_key);
    let (default_url, events_last_month) = block_on(async move {
        let conn = open_main_database_read(server_context).await?;
        let account_name: String = conn
            .query_one(
                "SELECT a.name
                 FROM accounts a
                 INNER JOIN projects p ON p.account_id = a.id
                 WHERE p.id = $1",
                &[&project_key],
            )
            .await
            .map_err(AuthError::from)?
            .get(0);
        let since = nanoseconds_since_epoch() as i64 - MONTH_NANOS;
        let events_last_month: i64 = conn
            .query_one(
                "SELECT COALESCE(SUM(events_inserted), 0)::bigint
                 FROM projects_last_ingest
                 WHERE project_id = $1 AND started_at >= $2",
                &[&project_key, &since],
            )
            .await
            .map_err(AuthError::from)?
            .get(0);
        Ok::<_, AuthError>((default_site_url(&account_name), events_last_month))
    })?;

    Ok(TroubleshootingStatusResponse {
        project_name: project_row.name().to_string(),
        default_url,
        events_last_month,
        events_queued,
        integrations,
    })
}

pub(crate) fn run_check(
    server_context: Arc<ServerContext>,
    project: &OwnedProject,
    request: TroubleshootingCheckRequest,
) -> Result<TroubleshootingCheckResponse, AuthError> {
    let project_row = projects::fetch_project_blocking(server_context.clone(), project.key())
        .map_err(|error| AuthError::Db(error.to_string()))?
        .ok_or(AuthError::InvalidRequest("project not found"))?;
    let project_name = project_row.name().to_string();

    let url = normalize_check_url(&request.url)?;
    let fetch = block_on(fetch_page_html(&url))?;
    let install = match &fetch.html {
        Some(html) => analyze_tracker_html(html, &project_name),
        None => InstallResult {
            ok: false,
            findings: vec![InstallFinding {
                severity: Severity::Error,
                message: "Could not download the page, so the tracker install was not checked."
                    .to_string(),
            }],
        },
    };

    Ok(TroubleshootingCheckResponse {
        url,
        fetch: FetchResult {
            ok: fetch.ok,
            status_code: fetch.status_code,
            error: fetch.error,
            note: fetch.note.map(str::to_string),
            final_url: Some(fetch.final_url),
        },
        install,
        wait_started_at_ms: (nanoseconds_since_epoch() / 1_000_000) as i64,
    })
}

/// Pageviews + events still in the project's in-memory batch.
fn queued_events(server_context: &ServerContext, project_key: i32) -> i64 {
    server_context
        .ingest_map
        .get_existing(project_key)
        .map_or(0, |guard| {
            (guard.pageviews.len() + guard.queued_event_count()) as i64
        })
}

/// Look for a tracker pageview since `since_ms`, on `hostname` if the query names one.
pub(crate) fn poll_ingest(
    server_context: Arc<ServerContext>,
    project: &OwnedProject,
    query: TroubleshootingIngestQuery,
) -> Result<TroubleshootingIngestResponse, AuthError> {
    let since_nanos = query.since_ms.saturating_mul(NANOS_PER_MILLI);
    let wanted = query.hostname.as_deref();

    let queued = server_context
        .ingest_map
        .get_existing(project.key())
        .and_then(|guard| {
            guard
                .pageviews
                .iter()
                .filter(|pageview| pageview.timestamp_id >= since_nanos)
                .filter(|pageview| wanted.is_none_or(|host| same_site(&pageview.hostname, host)))
                .max_by_key(|pageview| pageview.timestamp_id)
                .map(|pageview| {
                    (
                        pageview.timestamp_id,
                        pageview.path.to_string(),
                        pageview.hostname.to_string(),
                    )
                })
        });
    let hit = match queued {
        Some(hit) => Some(hit),
        None => stored_pageview(&server_context, project, since_nanos, wanted)?,
    };

    Ok(match hit {
        Some((timestamp_id, path, hostname)) => TroubleshootingIngestResponse {
            detected: true,
            detected_at_ms: Some(timestamp_id / NANOS_PER_MILLI),
            path: Some(path),
            hostname: Some(hostname),
        },
        None => TroubleshootingIngestResponse {
            detected: false,
            detected_at_ms: None,
            path: None,
            hostname: None,
        },
    })
}

/// The newest hit already flushed out of the batch.
fn stored_pageview(
    server_context: &ServerContext,
    project: &OwnedProject,
    since_nanos: i64,
    wanted: Option<&str>,
) -> Result<Option<(i64, String, String)>, AuthError> {
    let (bare, www) = match wanted {
        Some(host) => {
            let bare = bare_host(host);
            let www = format!("www.{bare}");
            (Some(bare), Some(www))
        }
        None => (None, None),
    };
    let conn = open_project_database_read(server_context, project.id())?;
    let mut statement = conn
        .prepare_cached(
            "SELECT pv.timestamp_id, p.path, p.hostname
             FROM pageviews pv
             JOIN paths p ON p.id = pv.path_id
             WHERE pv.timestamp_id >= ?1
             AND (?2 IS NULL OR lower(p.hostname) IN (?2, ?3))
             ORDER BY pv.timestamp_id DESC
             LIMIT 1",
        )
        .map_err(|error| AuthError::Query(error.to_string()))?;
    let mut rows = statement
        .query_map((since_nanos, &bare, &www), |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .map_err(|error| AuthError::Query(error.to_string()))?;
    rows.next()
        .transpose()
        .map_err(|error| AuthError::Query(error.to_string()))
}

/// Whether two hostnames are the same site: a check of `example.com` that redirects
/// to `www.example.com` records its visits there.
fn same_site(first: &str, second: &str) -> bool {
    bare_host(first) == bare_host(second)
}

fn bare_host(host: &str) -> String {
    let host = host.trim().to_ascii_lowercase();
    host.strip_prefix("www.")
        .map(str::to_string)
        .unwrap_or(host)
}

fn integration_status(view: IntegrationView) -> IntegrationStatus {
    let status = if view.last_sync_error.is_some() {
        SyncStatus::Error
    } else if view.last_synced_at.is_some() {
        SyncStatus::Ok
    } else {
        SyncStatus::NeverRun
    };
    IntegrationStatus {
        integration_type: view.integration_type,
        last_synced_at: view.last_synced_at,
        last_sync_error: view.last_sync_error,
        status,
    }
}

fn default_site_url(account_name: &str) -> String {
    let host = account_name.trim().trim_end_matches('/');
    if host.is_empty() {
        return String::new();
    }
    format!("https://{host}/")
}

fn normalize_check_url(raw: &str) -> Result<String, AuthError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AuthError::InvalidRequest("url is required"));
    }
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };
    let parsed = Url::parse(&with_scheme).map_err(|_| AuthError::InvalidRequest("invalid url"))?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(AuthError::InvalidRequest("url must be http or https"));
    }
    if parsed.host_str().is_none() {
        return Err(AuthError::InvalidRequest("url must include a host"));
    }
    Ok(parsed.to_string())
}

struct FetchedPage {
    ok: bool,
    status_code: Option<u16>,
    error: Option<String>,
    note: Option<&'static str>,
    html: Option<String>,
    /// Where any redirects ended.
    final_url: String,
}

async fn fetch_page_html(url: &str) -> Result<FetchedPage, AuthError> {
    let mut current = Url::parse(url).map_err(|_| AuthError::InvalidRequest("invalid url"))?;
    for _ in 0..=MAX_REDIRECTS {
        let addrs = match ensure_public_url(&current).await {
            Ok(addrs) => addrs,
            Err(message) => {
                return Ok(FetchedPage {
                    ok: false,
                    status_code: None,
                    error: Some(message.to_string()),
                    note: None,
                    html: None,
                    final_url: current.to_string(),
                });
            }
        };
        // Pinned to the addresses just checked: left to resolve the host
        // itself, reqwest would look it up again, and a short-TTL record can
        // answer that second lookup with a private address. No proxy, since a
        // proxy would resolve the host on its own.
        let mut builder = reqwest::Client::builder()
            .timeout(FETCH_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("RevScope-Troubleshooting/1.0")
            .no_proxy();
        if let Some(domain) = current.domain() {
            builder = builder.resolve_to_addrs(domain, &addrs);
        }
        let client = builder
            .build()
            .map_err(|error| AuthError::Db(format!("http client: {error}")))?;
        let response = match client.get(current.clone()).send().await {
            Ok(response) => response,
            Err(error) => {
                return Ok(FetchedPage {
                    ok: false,
                    status_code: None,
                    error: Some(format!("Request failed: {error}")),
                    note: None,
                    html: None,
                    final_url: current.to_string(),
                });
            }
        };
        let status = response.status();
        if status.is_redirection() {
            let Some(location) = response.headers().get(reqwest::header::LOCATION) else {
                return Ok(FetchedPage {
                    ok: false,
                    status_code: Some(status.as_u16()),
                    error: Some("Redirect missing Location header.".to_string()),
                    note: None,
                    html: None,
                    final_url: current.to_string(),
                });
            };
            let location = location
                .to_str()
                .map_err(|_| AuthError::InvalidRequest("invalid redirect location"))?;
            current = current
                .join(location)
                .map_err(|_| AuthError::InvalidRequest("invalid redirect location"))?;
            continue;
        }
        if !status.is_success() {
            return Ok(FetchedPage {
                ok: false,
                status_code: Some(status.as_u16()),
                error: Some(format!("HTTP {}", status.as_u16())),
                note: None,
                html: None,
                final_url: current.to_string(),
            });
        }

        let bytes = match read_body_capped(response, MAX_BODY_BYTES).await {
            Ok(Some(bytes)) => bytes,
            Ok(None) => {
                return Ok(FetchedPage {
                    ok: false,
                    status_code: Some(status.as_u16()),
                    error: Some(format!(
                        "Response larger than {} bytes; download aborted.",
                        MAX_BODY_BYTES
                    )),
                    note: None,
                    html: None,
                    final_url: current.to_string(),
                });
            }
            Err(error) => {
                return Ok(FetchedPage {
                    ok: false,
                    status_code: Some(status.as_u16()),
                    error: Some(format!("Failed to read body: {error}")),
                    note: None,
                    html: None,
                    final_url: current.to_string(),
                });
            }
        };
        let html = String::from_utf8_lossy(&bytes).into_owned();
        let note = if !html.to_ascii_lowercase().contains("<script") {
            Some(
                "Page HTML has no script tags. Client-rendered SPAs may inject the tracker after load, which this check cannot see.",
            )
        } else {
            None
        };
        return Ok(FetchedPage {
            ok: true,
            status_code: Some(status.as_u16()),
            error: None,
            note,
            html: Some(html),
            final_url: current.to_string(),
        });
    }

    Ok(FetchedPage {
        ok: false,
        status_code: None,
        error: Some("Too many redirects.".to_string()),
        note: None,
        html: None,
        final_url: current.to_string(),
    })
}

/// The body, or `None` as soon as it is known to exceed `limit`. The page is
/// untrusted, so reading stops at the first chunk that would pass `limit`,
/// whatever its `Content-Length` claims.
async fn read_body_capped(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Option<Vec<u8>>, reqwest::Error> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Ok(None);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len() + chunk.len() > limit {
            return Ok(None);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(Some(body))
}

/// The addresses `url`'s host resolves to, all of them public. The fetch must
/// connect only to these.
async fn ensure_public_url(url: &Url) -> Result<Vec<SocketAddr>, &'static str> {
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err("url must be http or https");
    }
    let Some(host) = url.host_str() else {
        return Err("url must include a host");
    };
    if host.eq_ignore_ascii_case("localhost") || host.ends_with(".localhost") {
        return Err("url host is not allowed");
    }
    let port = url.port_or_known_default().unwrap_or(80);
    // `host_str` keeps an IPv6 literal's brackets, which `lookup_host` cannot
    // parse, so literals are taken as they are.
    let addrs: Vec<SocketAddr> = match url.host() {
        Some(Host::Ipv4(ip)) => vec![SocketAddr::new(IpAddr::V4(ip), port)],
        Some(Host::Ipv6(ip)) => vec![SocketAddr::new(IpAddr::V6(ip), port)],
        _ => lookup_host((host, port))
            .await
            .map_err(|_| "could not resolve url host")?
            .collect(),
    };
    if addrs.is_empty() {
        return Err("could not resolve url host");
    }
    if addrs.iter().any(|addr| !is_public_ip(addr.ip())) {
        return Err("url host resolves to a private address");
    }
    Ok(addrs)
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || v4.is_multicast()
                || v4.octets()[0] >= 240
                || v4.octets()[0] == 0
                || (v4.octets()[0] == 169 && v4.octets()[1] == 254)
                || (v4.octets()[0] == 100 && (v4.octets()[1] & 0b1100_0000) == 0b0100_0000)
                // IETF protocol assignments, and benchmarking.
                || v4.octets()[..3] == [192, 0, 0]
                || (v4.octets()[0] == 198 && (v4.octets()[1] & 0xfe) == 18))
        }
        IpAddr::V6(v6) => {
            // A v4 address written as v6 reaches the same host, so it gets the
            // v4 rules: mapped `[::ffff:127.0.0.1]`, compatible `[::127.0.0.1]`,
            // NAT64's `[64:ff9b::7f00:1]` and 6to4's `[2002:7f00:1::]`.
            // `to_ipv4` also turns `::1` and `::` into 0.0.0.x, which is refused.
            if let Some(v4) = v6.to_ipv4() {
                return is_public_ip(IpAddr::V4(v4));
            }
            let segments = v6.segments();
            if segments[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
                let [.., a, b, c, d] = v6.octets();
                return is_public_ip(IpAddr::V4(Ipv4Addr::new(a, b, c, d)));
            }
            if segments[0] == 0x2002 {
                let [_, _, a, b, c, d, ..] = v6.octets();
                return is_public_ip(IpAddr::V4(Ipv4Addr::new(a, b, c, d)));
            }
            // Local-use NAT64 (RFC 8215) translates to whatever the operator
            // routes it to, and Teredo hides its v4 server, so neither can be
            // checked.
            if segments[..3] == [0x64, 0xff9b, 1] || segments[..2] == [0x2001, 0] {
                return false;
            }
            !(v6.is_loopback()
                || v6.is_multicast()
                || v6.is_unspecified()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
                || (v6.segments()[0] & 0xffc0) == 0xfe80
                || (v6.segments()[0] & 0xfe00) == 0xfc00)
        }
    }
}

fn analyze_tracker_html(html: &str, project_name: &str) -> InstallResult {
    let mut findings = Vec::new();
    let lower = html.to_ascii_lowercase();
    let has_placeholder = html.contains("REPLACE_WITH_PROJECT_NAME");
    let has_dispatch = dispatch_name(project_name).is_match(html);
    let has_project_js = tracker_script_url(project_name).is_match(html);
    let has_default_host = lower.contains("in.revscope.co");
    let stub_names = stub_project_names(html);
    let stub_matches_project = stub_names.iter().any(|name| name == project_name);

    if has_placeholder {
        findings.push(InstallFinding {
            severity: Severity::Error,
            message: "Install snippet still contains REPLACE_WITH_PROJECT_NAME.".to_string(),
        });
    }

    let wrong_projects: Vec<String> = stub_names
        .iter()
        .filter(|name| {
            name.as_str() != project_name && name.as_str() != "REPLACE_WITH_PROJECT_NAME"
        })
        .cloned()
        .collect();
    let found_project = stub_matches_project || has_dispatch || has_project_js;
    if found_project {
        findings.push(InstallFinding {
            severity: Severity::Ok,
            message: format!("Found RevScope tracker references for `{project_name}`."),
        });
        if has_default_host {
            findings.push(InstallFinding {
                severity: Severity::Ok,
                message: "Default ingest host in.revscope.co is present.".to_string(),
            });
        } else {
            findings.push(InstallFinding {
                severity: Severity::Warning,
                message: format!(
                    "Default ingest host in.revscope.co was not found. If you proxy the tracker, confirm the first-party URL still loads `{project_name}.js`."
                ),
            });
        }
    } else if !wrong_projects.is_empty() {
        // Only when this project's tracker is absent: other vendors' snippets end the
        // same way, `})(window, "name")`, and sit beside it on plenty of pages.
        findings.push(InstallFinding {
            severity: Severity::Error,
            message: format!(
                "No tracker for `{project_name}` found. A snippet on the page names {}: if it's RevScope's, it has the wrong project, and if not, the tracker still needs adding.",
                wrong_projects
                    .iter()
                    .map(|name| format!("`{name}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        });
    } else {
        if !stub_names.is_empty()
            || html.contains("dispatch_")
            || lower.contains("revscope")
            || (lower.contains(".js") && html.contains("(function(w,n,u,c)"))
        {
            findings.push(InstallFinding {
                severity: Severity::Error,
                message: format!(
                    "Page mentions a RevScope-style stub, but not project `{project_name}`."
                ),
            });
        } else {
            findings.push(InstallFinding {
                severity: Severity::Error,
                message: format!(
                    "No RevScope tracker stub or `{project_name}.js` script found in the HTML. If the site is a client-rendered SPA, confirm the script is in the initial document or a proxied layout."
                ),
            });
        }
    }

    let ok = findings
        .iter()
        .all(|finding| finding.severity != Severity::Error);
    InstallResult { ok, findings }
}

// Matches the install IIFE's project-name argument: (window, "name"...
// URL is optional (defaults inside the stub to https://in.revscope.co/).
static STUB_PROJECT_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\)\s*\(\s*window\s*,\s*["']([a-z0-9_]+)["']"#).expect("stub regex")
});

/// A whole word, or the project `shop` would pass on `dispatch_shopify_order`.
fn dispatch_name(project_name: &str) -> Regex {
    Regex::new(&format!(r"\bdispatch_{}\b", regex::escape(project_name)))
        .expect("an escaped name makes a valid pattern")
}

/// The tracker script at the root of a host, where it's served. Anywhere deeper is
/// probably the site's own `/assets/shop.js`, not the tracker for a project `shop`.
fn tracker_script_url(project_name: &str) -> Regex {
    let name = regex::escape(project_name);
    Regex::new(&format!(
        r#"(?i)(?:https?:)?//[^/"'\s]+/{name}\.js\b|["']/{name}\.js\b"#
    ))
    .expect("an escaped name makes a valid pattern")
}

fn stub_project_names(html: &str) -> Vec<String> {
    STUB_PROJECT_NAME
        .captures_iter(html)
        .filter_map(|capture| capture.get(1).map(|m| m.as_str().to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyze_finds_default_stub() {
        let html = r#"<html><head><script>(function(w,n,u,c){if(u&&typeof u==="object"){c=u;u=null}c=c||{};u=u||"https://in.revscope.co/";c.url=u;u+=n+".js";c.project=n;n=typeof c.name==="string"&&c.name||"dispatch";})(window, "acme");</script></head></html>"#;
        let result = analyze_tracker_html(html, "acme");
        assert!(result.ok);
        assert!(result.findings.iter().any(|f| f.severity == Severity::Ok));
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.message.contains("in.revscope.co"))
        );
    }

    #[test]
    fn analyze_finds_stub_with_explicit_url() {
        let html = r#"<html><head><script>(function(w,n,u,c){c=c||{};c.url=u;u+=n+".js";c.project=n;n=typeof c.name==="string"&&c.name||"dispatch";})(window, "acme", "https://in.revscope.co/");</script></head></html>"#;
        let result = analyze_tracker_html(html, "acme");
        assert!(result.ok);
        assert!(result.findings.iter().any(|f| f.severity == Severity::Ok));
    }

    #[test]
    fn analyze_detects_wrong_project() {
        let html = r#"<script>(function(w,n,u,c){})(window, "other", "https://in.revscope.co/");</script>"#;
        let result = analyze_tracker_html(html, "acme");
        assert!(!result.ok);
        let errors: Vec<_> = result
            .findings
            .iter()
            .filter(|finding| finding.severity == Severity::Error)
            .collect();
        assert_eq!(errors.len(), 1, "{:?}", result.findings);
        assert!(errors[0].message.contains("`other`"));
        assert!(!errors[0].message.contains("Page mentions"));
    }

    #[test]
    fn analyze_detects_placeholder() {
        let html = r#"<script>(function(w,n,u,c){})(window, "REPLACE_WITH_PROJECT_NAME", "https://in.revscope.co/");</script>"#;
        let result = analyze_tracker_html(html, "acme");
        assert!(!result.ok);
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.message.contains("REPLACE_WITH_PROJECT_NAME"))
        );
    }

    #[test]
    fn default_url_from_account_name() {
        assert_eq!(default_site_url("example.com"), "https://example.com/");
    }

    #[test]
    fn is_public_ip_admits_only_globally_routable_addresses() {
        for ip in [
            "93.184.216.34",
            "8.8.8.8",
            "100.63.255.255",
            "100.128.0.0",
            "2606:4700::1111",
            "::ffff:93.184.216.34",
            "::93.184.216.34",
            "64:ff9b::808:808",
            "2002:5db8:d822::1",
            "198.17.255.255",
            "198.20.0.0",
            "192.0.1.0",
        ] {
            assert!(is_public_ip(ip.parse().unwrap()), "{ip} is public");
        }
    }

    #[test]
    fn is_public_ip_refuses_private_and_reserved_addresses() {
        for ip in [
            // v4: private, loopback, link-local (cloud metadata), CGNAT,
            // this-network, documentation, multicast, reserved, broadcast.
            "10.0.0.1",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "127.0.0.1",
            "127.255.255.254",
            "169.254.169.254",
            "100.64.0.1",
            "100.127.255.255",
            "0.0.0.0",
            "0.1.2.3",
            "192.0.2.1",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "239.255.255.250",
            "240.0.0.1",
            "255.255.255.255",
            // v6: loopback, unspecified, unique local, link-local, multicast.
            "::1",
            "::",
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
            "febf::1",
            "ff02::1",
            // v4 addresses spelled as v6 reach the same hosts.
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "::ffff:169.254.169.254",
            "64:ff9b::7f00:1",
            "64:ff9b::a9fe:a9fe",
            "::127.0.0.1",
            "::7f00:1",
            "2002:7f00:1::",
            "2002:a9fe:a9fe::1",
            // Local-use NAT64 and Teredo can't be checked.
            "64:ff9b:1::a9fe:a9fe",
            "2001:0:4136:e378::1",
            // v4 IETF protocol assignments and benchmarking.
            "192.0.0.1",
            "198.18.0.1",
            "198.19.255.255",
        ] {
            assert!(!is_public_ip(ip.parse().unwrap()), "{ip} is not public");
        }
    }

    fn response_with_body(body: impl Into<reqwest::Body>) -> reqwest::Response {
        reqwest::Response::from(http::Response::new(body))
    }

    #[tokio::test]
    async fn a_body_within_the_limit_is_read_whole() {
        let body = vec![b'a'; 64];
        let read = read_body_capped(response_with_body(body.clone()), 64)
            .await
            .expect("body reads");
        assert_eq!(read, Some(body));
    }

    /// Claims `length` bytes and panics if any are read.
    struct UnreadBody {
        length: u64,
    }

    impl http_body::Body for UnreadBody {
        type Data = bytes::Bytes;
        type Error = std::io::Error;

        fn poll_frame(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
            panic!("a body declared over the limit must not be read");
        }

        fn size_hint(&self) -> http_body::SizeHint {
            http_body::SizeHint::with_exact(self.length)
        }
    }

    #[tokio::test]
    async fn a_declared_length_over_the_limit_is_refused_unread() {
        let body = reqwest::Body::wrap(UnreadBody { length: 65 });
        let read = read_body_capped(response_with_body(body), 64)
            .await
            .expect("body reads");
        assert_eq!(read, None);
    }

    #[tokio::test]
    async fn a_streamed_body_over_the_limit_is_refused() {
        let chunks = futures::stream::iter([&[b'a'; 40][..], &[b'a'; 25][..]].map(|chunk| {
            Ok::<_, std::io::Error>(http_body::Frame::data(bytes::Bytes::from_static(chunk)))
        }));
        let body = reqwest::Body::wrap(http_body_util::StreamBody::new(chunks));
        let read = read_body_capped(response_with_body(body), 64)
            .await
            .expect("body reads");
        assert_eq!(read, None);
    }

    #[tokio::test]
    async fn an_endless_body_without_a_length_stops_at_the_limit() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let chunks_sent = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&chunks_sent);
        let endless = futures::stream::repeat_with(move || {
            counter.fetch_add(1, Ordering::Relaxed);
            Ok::<_, std::io::Error>(http_body::Frame::data(bytes::Bytes::from_static(
                &[b'a'; 16],
            )))
        });
        let body = reqwest::Body::wrap(http_body_util::StreamBody::new(endless));
        let read = read_body_capped(response_with_body(body), 64)
            .await
            .expect("body reads");
        assert_eq!(read, None);
        assert_eq!(chunks_sent.load(Ordering::Relaxed), 5);
    }

    async fn check_url(url: &str) -> Result<Vec<SocketAddr>, &'static str> {
        ensure_public_url(&Url::parse(url).expect("valid url")).await
    }

    #[tokio::test]
    async fn a_public_address_is_fetched_on_the_port_the_url_names() {
        let addrs = check_url("http://93.184.216.34:8080/")
            .await
            .expect("public");
        assert_eq!(addrs, ["93.184.216.34:8080".parse::<SocketAddr>().unwrap()]);
        let addrs = check_url("https://93.184.216.34/").await.expect("public");
        assert_eq!(addrs[0].port(), 443);
        let addrs = check_url("http://[2606:4700::1111]/")
            .await
            .expect("public");
        assert_eq!(
            addrs,
            ["[2606:4700::1111]:80".parse::<SocketAddr>().unwrap()]
        );
    }

    #[tokio::test]
    async fn private_hosts_are_refused_however_the_url_spells_them() {
        for url in [
            "http://127.0.0.1/",
            "http://169.254.169.254/latest/meta-data/",
            "http://[::1]/",
            "http://[::ffff:10.0.0.1]/",
            // The url parser reads these as 127.0.0.1.
            "http://2130706433/",
            "http://0x7f.1/",
        ] {
            assert_eq!(
                check_url(url).await,
                Err("url host resolves to a private address"),
                "{url}"
            );
        }
    }

    #[tokio::test]
    async fn localhost_names_and_other_schemes_are_refused_before_resolving() {
        for url in [
            "http://localhost/",
            "http://LOCALHOST:3000/",
            "http://app.localhost/",
        ] {
            assert_eq!(
                check_url(url).await,
                Err("url host is not allowed"),
                "{url}"
            );
        }
        for url in ["ftp://93.184.216.34/", "file:///etc/passwd"] {
            assert_eq!(
                check_url(url).await,
                Err("url must be http or https"),
                "{url}"
            );
        }
    }

    #[test]
    fn a_host_with_or_without_www_is_the_same_site() {
        assert!(same_site("www.example.com", "example.com"));
        assert!(same_site("Example.com", "www.EXAMPLE.com"));
        assert!(!same_site("staging.example.com", "example.com"));
        assert!(!same_site("example.com.evil.test", "example.com"));
    }

    #[test]
    fn another_vendors_snippet_does_not_hide_the_tracker() {
        let html = r#"<script>(function(w,n,u,c){c=c||{};u=u||"https://in.revscope.co/";})(window, "shop");</script>
            <script>(function(w,r){w[r]=w[r]||function(){};})(window,'rewardful');</script>"#;
        let result = analyze_tracker_html(html, "shop");
        assert!(result.ok, "{:?}", result.findings);
    }

    #[test]
    fn a_stub_for_another_project_alone_is_still_an_error() {
        let html = r#"<script>(function(w,n,u,c){})(window, "other");</script>"#;
        let result = analyze_tracker_html(html, "shop");
        assert!(!result.ok);
        assert!(result.findings[0].message.contains("`other`"));
        assert!(!analyze_tracker_html("dispatch_shopify_order", "shop").ok);
    }

    #[test]
    fn the_sites_own_file_with_the_projects_name_is_not_the_tracker() {
        for html in [
            r#"<script src="/assets/shop.js"></script>"#,
            r#"<script src="/myshop.js"></script>"#,
            r#"<link href="https://cdn.example.com/shop.json">"#,
        ] {
            assert!(!analyze_tracker_html(html, "shop").ok, "{html}");
        }
        for html in [
            r#"<script src="https://in.revscope.co/shop.js"></script>"#,
            r#"<script src="https://stats.example.com/shop.js"></script>"#,
            r#"<script src="/shop.js"></script>"#,
        ] {
            assert!(analyze_tracker_html(html, "shop").ok, "{html}");
        }
    }
}
