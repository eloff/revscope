use std::sync::Arc;
use std::time::Duration;

use fxhash::{FxHashMap, FxHashSet};
use log::{info, warn};
use reqwest::StatusCode;
use reqwest::header::{HeaderMap, RETRY_AFTER};
use serde::Deserialize;
use serde_json::json;

use super::schedule::{self, SYNC_BATCH_SIZE, SYNC_THROTTLE_SECS, SyncOutcome};
use super::sync::{SyncCounts, SyncError, SyncKind, SyncResult, sleep_or_shutdown};
use crate::crawlers::{
    CloudflareUpsert, cloudflare_patterns, upsert_cloudflare_crawler_view, upsert_crawler_path,
    upsert_useragent,
};
use crate::crypto;
use crate::db::{
    ProjectConnection, open_main_database_read, open_project_database_read_for,
    open_project_database_write_for,
};
use crate::server_context::ServerContext;
use crate::util::{rfc3339_to_unix, unix_seconds, unix_to_rfc3339};

const CLOUDFLARE_FULL_SYNC_INTERVAL_SECS: i64 = 8 * 60 * 60;
const CLOUDFLARE_REQUEST_MAX_ATTEMPTS: usize = 5;
const CLOUDFLARE_REQUEST_BASE_BACKOFF: Duration = Duration::from_secs(2);
const CLOUDFLARE_REQUEST_MAX_BACKOFF: Duration = Duration::from_secs(60);
const CLOUDFLARE_LOOKBACK_HOURS: i64 = 23;
const CLOUDFLARE_PAGE_LIMIT: i64 = 5000;
const HOUR_SECONDS: i64 = 3600;
const CLOUDFLARE_GRAPHQL_URL: &str = "https://api.cloudflare.com/client/v4/graphql";
const CLOUDFLARE_ZONES_URL: &str = "https://api.cloudflare.com/client/v4/zones";

pub(crate) struct CloudflareIntegration {
    id: i32,
    project_id: String,
    encrypted_api_token: Option<Vec<u8>>,
}

#[derive(Debug)]
struct CloudflareZone {
    id: String,
    name: String,
}

pub(crate) async fn load_due_integrations(
    server_context: Arc<ServerContext>,
) -> SyncResult<Vec<CloudflareIntegration>> {
    let conn = open_main_database_read(server_context).await?;
    let rows = conn
        .query(
            &format!(
                "SELECT
                    id,
                    project_id::text AS project_id,
                    encrypted_api_key
                 FROM integrations
                 WHERE disabled_at IS NULL
                    AND integration_type = 'cloudflare'::integration_type
                    AND encrypted_api_key IS NOT NULL
                    AND (last_sync_completed_at IS NULL
                         OR last_sync_completed_at < NOW() - INTERVAL '{SYNC_THROTTLE_SECS} seconds')
                    AND (last_full_sync_success_at IS NULL
                         OR last_full_sync_success_at < NOW() - INTERVAL '{CLOUDFLARE_FULL_SYNC_INTERVAL_SECS} seconds')
                 ORDER BY last_full_sync_success_at ASC NULLS FIRST
                 LIMIT {SYNC_BATCH_SIZE}"
            ),
            &[],
        )
        .await?;

    Ok(rows
        .iter()
        .map(|row| CloudflareIntegration {
            id: row.get(0),
            project_id: row.get(1),
            encrypted_api_token: row.get(2),
        })
        .collect())
}

pub(crate) async fn run_sync_with_retries(
    server_context: Arc<ServerContext>,
    integration: CloudflareIntegration,
) {
    schedule::run_sync_with_retries(
        server_context.clone(),
        "cloudflare",
        integration.id,
        SyncKind::Full,
        || async { sync_integration(server_context.clone(), &integration).await },
    )
    .await;
}

async fn sync_integration(
    server_context: Arc<ServerContext>,
    integration: &CloudflareIntegration,
) -> SyncResult<SyncOutcome> {
    let project =
        super::sync::ensure_project_cached(server_context.clone(), &integration.project_id).await?;
    let api_token = decrypt_credential(integration.encrypted_api_token.as_deref(), "API token")?;
    let apex_domains = {
        let conn = open_project_database_read_for(&server_context, &project)?;
        apex_domains_from_paths(&conn)?
    };
    if apex_domains.is_empty() {
        return Err(SyncError::Cloudflare(
            "no apex domains in paths to match Cloudflare zones".into(),
        ));
    }

    let client = &server_context.http;
    let zones = fetch_matching_zones(
        &server_context,
        client,
        &api_token,
        &apex_domains,
        integration.id,
    )
    .await?;
    if zones.is_empty() {
        return Err(SyncError::Cloudflare(format!(
            "no Cloudflare zones match apex domains {}",
            apex_domains.join(", ")
        )));
    }

    let (window_start, window_end) = complete_hours_window(unix_seconds());
    let format_hour = |unix| {
        unix_to_rfc3339(unix)
            .ok_or_else(|| SyncError::Cloudflare(format!("invalid sync window hour {unix}")))
    };
    let datetime_start = format_hour(window_start)?;
    let datetime_end = format_hour(window_end)?;

    let mut groups: FxHashMap<(i64, String, String, String), i64> = FxHashMap::default();
    for zone in &zones {
        for token in cloudflare_patterns() {
            if server_context.shutdown_requested() {
                return Err(SyncError::Cloudflare(
                    "abandoned: shutdown requested during GraphQL walk".into(),
                ));
            }
            let rows = fetch_token_groups(
                &server_context,
                client,
                CLOUDFLARE_GRAPHQL_URL,
                &api_token,
                &zone.id,
                token,
                &datetime_start,
                &datetime_end,
            )
            .await?;
            if rows.len() as i64 >= CLOUDFLARE_PAGE_LIMIT {
                warn!(
                    "Cloudflare integration {} zone {} token {token} returned {CLOUDFLARE_PAGE_LIMIT} rows (truncated)",
                    integration.id, zone.name
                );
            }
            for row in rows {
                let Some(key) = row.bucket_key() else {
                    continue;
                };
                let estimated = row.estimated_count();
                if estimated <= 0 {
                    continue;
                }
                let entry = groups.entry(key).or_insert(0);
                *entry = (*entry).max(estimated);
            }
        }
    }

    let conn = open_project_database_write_for(&server_context, &project)?;
    conn.execute("BEGIN IMMEDIATE", [])?;
    let counts = match write_groups(&conn, groups) {
        Ok(counts) => {
            conn.execute("COMMIT", [])?;
            counts
        }
        Err(error) => {
            let _ = conn.execute("ROLLBACK", []);
            return Err(error);
        }
    };

    let zone_names = zones
        .iter()
        .map(|zone| zone.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    info!(
        "synced Cloudflare integration {} zones [{zone_names}]: inserted={}, updated={}",
        integration.id, counts.records_inserted, counts.records_updated
    );
    Ok(SyncOutcome {
        counts,
        sync_kind: SyncKind::Full,
    })
}

fn write_groups(
    conn: &ProjectConnection,
    groups: FxHashMap<(i64, String, String, String), i64>,
) -> SyncResult<SyncCounts> {
    let mut counts = SyncCounts::default();
    for ((hour, useragent, hostname, path), estimated_count) in groups {
        let useragent_id = upsert_useragent(conn, &useragent, None)?;
        let path_id = upsert_crawler_path(conn, &path, &hostname)?;
        match upsert_cloudflare_crawler_view(conn, hour, useragent_id, path_id, estimated_count)? {
            CloudflareUpsert::Inserted => counts.records_inserted += 1,
            CloudflareUpsert::Updated => counts.records_updated += 1,
            CloudflareUpsert::KeptHigher { stored, fetched } => {
                warn!(
                    "Cloudflare crawler view hour={hour} useragent_id={useragent_id} \
                     fetched {fetched} < stored {stored}; keeping stored count"
                );
            }
        }
    }
    Ok(counts)
}

fn apex_domains_from_paths(conn: &ProjectConnection) -> SyncResult<Vec<String>> {
    let mut statement = conn.prepare(
        "SELECT DISTINCT hostname FROM paths
         WHERE deleted_at IS NULL AND hostname != ''",
    )?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    let mut apexes = FxHashSet::default();
    for hostname in rows {
        if let Some(apex) = apex_domain(&hostname?) {
            apexes.insert(apex);
        }
    }
    let mut list: Vec<String> = apexes.into_iter().collect();
    list.sort();
    Ok(list)
}

/// Registrable (eTLD+1) domain for a hostname stored on `paths`.
fn apex_domain(hostname: &str) -> Option<String> {
    let host = hostname.trim().trim_end_matches('.').to_ascii_lowercase();
    let host = strip_host_port(&host)?;
    if host.is_empty() || host.parse::<std::net::IpAddr>().is_ok() {
        return None;
    }
    psl::domain_str(host).map(|domain| domain.trim_end_matches('.').to_ascii_lowercase())
}

fn strip_host_port(host: &str) -> Option<&str> {
    if let Some(stripped) = host.strip_prefix('[') {
        return stripped.split(']').next().filter(|value| !value.is_empty());
    }
    if let Some((name, port)) = host.rsplit_once(':')
        && !name.is_empty()
        && !port.is_empty()
        && port.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Some(name);
    }
    Some(host)
}

async fn fetch_matching_zones(
    server_context: &ServerContext,
    client: &reqwest::Client,
    api_token: &str,
    apex_domains: &[String],
    integration_id: i32,
) -> SyncResult<Vec<CloudflareZone>> {
    let mut zones: FxHashMap<String, String> = FxHashMap::default();
    for apex in apex_domains {
        if server_context.shutdown_requested() {
            return Err(SyncError::Cloudflare(
                "abandoned: shutdown requested while listing zones".into(),
            ));
        }
        let label = format!("cloudflare zones {apex}");
        let text = send_with_retries(server_context, &label, || {
            client
                .get(CLOUDFLARE_ZONES_URL)
                .bearer_auth(api_token)
                .query(&[("name", apex.as_str()), ("status", "active")])
        })
        .await?;
        let found = parse_zones_response(&text)?;
        if found.is_empty() {
            warn!("Cloudflare integration {integration_id} has no active zone named {apex}");
        }
        for zone in found {
            zones.insert(zone.id, zone.name);
        }
    }
    let mut list: Vec<CloudflareZone> = zones
        .into_iter()
        .map(|(id, name)| CloudflareZone { id, name })
        .collect();
    list.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(list)
}

fn parse_zones_response(text: &str) -> SyncResult<Vec<CloudflareZone>> {
    let parsed: ZonesResponse = serde_json::from_str(text)?;
    if parsed.success == Some(false) {
        let message = parsed
            .errors
            .as_ref()
            .map(|errors| {
                errors
                    .iter()
                    .map(|error| error.message.as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
            })
            .filter(|message| !message.is_empty())
            .unwrap_or_else(|| text.to_string());
        return Err(SyncError::Cloudflare(format!(
            "zones list failed: {message}"
        )));
    }
    Ok(parsed
        .result
        .unwrap_or_default()
        .into_iter()
        .filter(|zone| !zone.id.is_empty() && !zone.name.is_empty())
        .map(|zone| CloudflareZone {
            id: zone.id,
            name: zone.name,
        })
        .collect())
}

#[allow(clippy::too_many_arguments)]
async fn fetch_token_groups(
    server_context: &ServerContext,
    client: &reqwest::Client,
    api_url: &str,
    api_token: &str,
    zone_tag: &str,
    token: &str,
    datetime_start: &str,
    datetime_end: &str,
) -> SyncResult<Vec<AdaptiveGroup>> {
    let body = graphql_body(zone_tag, token, datetime_start, datetime_end);
    let label = format!("cloudflare graphql {token}");
    let text = send_with_retries(server_context, &label, || {
        client
            .post(api_url)
            .timeout(super::REPORT_QUERY_TIMEOUT)
            .bearer_auth(api_token)
            .header("Content-Type", "application/json")
            .json(&body)
    })
    .await?;
    parse_graphql_groups(&text)
}

fn graphql_body(
    zone_tag: &str,
    token: &str,
    datetime_start: &str,
    datetime_end: &str,
) -> serde_json::Value {
    // Cloudflare's LIKE is case-sensitive; `token` must be the UA's real casing.
    let pattern = format!("%{token}%");
    json!({
        "query": r#"
query CrawlerHits($zoneTag: string, $datetimeStart: Time, $datetimeEnd: Time, $userAgent: string) {
  viewer {
    zones(filter: {zoneTag: $zoneTag}) {
      crawlerHits: httpRequestsAdaptiveGroups(
        limit: 5000
        orderBy: [datetimeHour_DESC]
        filter: {
          AND: [
            { datetime_geq: $datetimeStart, datetime_lt: $datetimeEnd }
            { requestSource: "eyeball" }
            { edgeResponseStatus_geq: 200, edgeResponseStatus_lt: 300 }
            { userAgent_like: $userAgent }
          ]
        }
      ) {
        count
        avg { sampleInterval }
        dimensions {
          datetimeHour
          userAgent
          clientRequestHTTPHost
          clientRequestPath
        }
      }
    }
  }
}
"#,
        "variables": {
            "zoneTag": zone_tag,
            "datetimeStart": datetime_start,
            "datetimeEnd": datetime_end,
            "userAgent": pattern,
        }
    })
}

fn parse_graphql_groups(text: &str) -> SyncResult<Vec<AdaptiveGroup>> {
    let parsed: GraphQlResponse = serde_json::from_str(text)?;
    if let Some(errors) = parsed.errors
        && !errors.is_empty()
    {
        let message = errors
            .iter()
            .map(|error| error.message.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(SyncError::Cloudflare(format!("GraphQL errors: {message}")));
    }
    let Some(data) = parsed.data else {
        return Ok(Vec::new());
    };
    Ok(data
        .viewer
        .zones
        .into_iter()
        .flat_map(|zone| zone.crawler_hits)
        .collect())
}

fn decrypt_credential(encrypted: Option<&[u8]>, label: &'static str) -> SyncResult<String> {
    let encrypted = encrypted.ok_or(SyncError::MissingCloudflareCredential(label))?;
    let decrypted =
        crypto::decrypt(encrypted).ok_or(SyncError::InvalidCloudflareCredential(label))?;
    String::from_utf8(decrypted).map_err(|_| SyncError::InvalidCloudflareCredential(label))
}

async fn send_with_retries(
    server_context: &ServerContext,
    label: &str,
    build_request: impl Fn() -> reqwest::RequestBuilder,
) -> SyncResult<String> {
    let mut attempt = 1;
    loop {
        let can_retry = attempt < CLOUDFLARE_REQUEST_MAX_ATTEMPTS;
        // The body is read inside the attempt so a timeout while it downloads
        // is retried like one before the headers arrive.
        let attempt_result = async {
            let response = build_request().send().await?;
            let status = response.status();
            let retry_after = retry_after_delay(response.headers());
            let text = response.text().await?;
            Ok::<_, reqwest::Error>((status, retry_after, text))
        }
        .await;
        match attempt_result {
            Err(error) => {
                if !can_retry || !is_retryable_transport_error(&error) {
                    return Err(error.into());
                }
                let delay = retry_backoff(attempt);
                warn!(
                    "{label} could not reach Cloudflare ({error}), retrying in {delay:?} (attempt {attempt}/{CLOUDFLARE_REQUEST_MAX_ATTEMPTS})"
                );
                sleep_before_retry(server_context, label, delay).await?;
            }
            Ok((status, retry_after, text)) => {
                if status.is_success() {
                    return Ok(text);
                }
                if !can_retry || !is_retryable_response(status) {
                    return Err(SyncError::Cloudflare(format!(
                        "{label} failed with {status}: {text}"
                    )));
                }
                let delay = retry_after.unwrap_or_else(|| retry_backoff(attempt));
                warn!(
                    "{label} failed with {status}, retrying in {delay:?} (attempt {attempt}/{CLOUDFLARE_REQUEST_MAX_ATTEMPTS})"
                );
                sleep_before_retry(server_context, label, delay).await?;
            }
        }
        attempt += 1;
    }
}

async fn sleep_before_retry(
    server_context: &ServerContext,
    label: &str,
    delay: Duration,
) -> SyncResult<()> {
    if sleep_or_shutdown(server_context, delay).await {
        return Err(SyncError::Cloudflare(format!(
            "{label} abandoned: shutdown requested while backing off"
        )));
    }
    Ok(())
}

fn is_retryable_transport_error(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect()
}

fn is_retryable_response(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

fn retry_after_delay(headers: &HeaderMap) -> Option<Duration> {
    let seconds: u64 = headers
        .get(RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()?;
    Some(Duration::from_secs(seconds).min(CLOUDFLARE_REQUEST_MAX_BACKOFF))
}

fn retry_backoff(attempt: usize) -> Duration {
    let exponent = u32::try_from(attempt.saturating_sub(1)).unwrap_or(u32::MAX);
    CLOUDFLARE_REQUEST_BASE_BACKOFF
        .checked_mul(2_u32.saturating_pow(exponent))
        .unwrap_or(CLOUDFLARE_REQUEST_MAX_BACKOFF)
        .min(CLOUDFLARE_REQUEST_MAX_BACKOFF)
}

/// Closed hours fully inside a 24h lookback: `[floor(now) - 23h, floor(now))`.
fn complete_hours_window(now_unix: i64) -> (i64, i64) {
    let floor = now_unix - now_unix.rem_euclid(HOUR_SECONDS);
    (floor - CLOUDFLARE_LOOKBACK_HOURS * HOUR_SECONDS, floor)
}

#[derive(Debug, Deserialize)]
struct ZonesResponse {
    success: Option<bool>,
    errors: Option<Vec<CloudflareApiError>>,
    result: Option<Vec<ZoneResult>>,
}

#[derive(Debug, Deserialize)]
struct CloudflareApiError {
    message: String,
}

#[derive(Debug, Deserialize)]
struct ZoneResult {
    id: String,
    name: String,
}

#[derive(Debug, Deserialize)]
struct GraphQlResponse {
    data: Option<GraphQlData>,
    errors: Option<Vec<GraphQlError>>,
}

#[derive(Debug, Deserialize)]
struct GraphQlError {
    message: String,
}

#[derive(Debug, Deserialize)]
struct GraphQlData {
    viewer: GraphQlViewer,
}

#[derive(Debug, Deserialize)]
struct GraphQlViewer {
    zones: Vec<GraphQlZone>,
}

#[derive(Debug, Deserialize)]
struct GraphQlZone {
    #[serde(rename = "crawlerHits", default)]
    crawler_hits: Vec<AdaptiveGroup>,
}

#[derive(Debug, Deserialize)]
struct AdaptiveGroup {
    count: Option<f64>,
    avg: Option<AdaptiveAvg>,
    dimensions: Option<AdaptiveDimensions>,
}

#[derive(Debug, Deserialize)]
struct AdaptiveAvg {
    #[serde(rename = "sampleInterval")]
    sample_interval: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct AdaptiveDimensions {
    #[serde(rename = "datetimeHour")]
    datetime_hour: Option<String>,
    #[serde(rename = "userAgent")]
    user_agent: Option<String>,
    #[serde(rename = "clientRequestHTTPHost")]
    host: Option<String>,
    #[serde(rename = "clientRequestPath")]
    path: Option<String>,
}

impl AdaptiveGroup {
    fn estimated_count(&self) -> i64 {
        let count = self.count.unwrap_or(0.0);
        let interval = self
            .avg
            .as_ref()
            .and_then(|avg| avg.sample_interval)
            .unwrap_or(1.0)
            .max(1.0);
        (count * interval).round() as i64
    }

    fn bucket_key(&self) -> Option<(i64, String, String, String)> {
        let dimensions = self.dimensions.as_ref()?;
        let hour = rfc3339_to_unix(dimensions.datetime_hour.as_deref()?).ok()?;
        let user_agent = dimensions.user_agent.as_deref()?.trim();
        if user_agent.is_empty() {
            return None;
        }
        let host = dimensions.host.as_deref()?.trim();
        let path = dimensions.path.as_deref()?.trim();
        if host.is_empty() || path.is_empty() {
            return None;
        }
        Some((
            hour,
            user_agent.to_ascii_lowercase(),
            host.to_string(),
            path.to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::open_migrated_db;

    #[test]
    fn window_is_23_complete_hours_ending_at_hour_floor() {
        // 2026-09-07 15:37:00 UTC
        let now = 1_788_000_000 + 15 * HOUR_SECONDS + 37 * 60;
        let floor = now - now.rem_euclid(HOUR_SECONDS);
        let (start, end) = complete_hours_window(now);
        assert_eq!(end, floor);
        assert_eq!(end - start, 23 * HOUR_SECONDS);
        assert_eq!(start % HOUR_SECONDS, 0);
        assert_eq!(end % HOUR_SECONDS, 0);
        // The current (partial) hour is excluded.
        assert!(end <= now);
        // The oldest hour starts at or after now-24h, so it is fully inside lookback.
        assert!(start >= now - 24 * HOUR_SECONDS);
    }

    #[test]
    fn window_at_exact_hour_boundary_excludes_current_hour() {
        // 2026-09-07 16:00:00 UTC — already on an hour boundary.
        let now = 1_788_001_200;
        let (start, end) = complete_hours_window(now);
        assert_eq!(end, now);
        assert_eq!(end - start, 23 * HOUR_SECONDS);
    }

    #[test]
    fn estimated_count_multiplies_sample_interval() {
        let group = AdaptiveGroup {
            count: Some(3.0),
            avg: Some(AdaptiveAvg {
                sample_interval: Some(10.0),
            }),
            dimensions: None,
        };
        assert_eq!(group.estimated_count(), 30);
        let exact = AdaptiveGroup {
            count: Some(2.0),
            avg: Some(AdaptiveAvg {
                sample_interval: Some(1.0),
            }),
            dimensions: None,
        };
        assert_eq!(exact.estimated_count(), 2);
    }

    #[test]
    fn parses_graphql_groups() {
        let json = r#"{
          "data": {
            "viewer": {
              "zones": [{
                "crawlerHits": [{
                  "count": 1,
                  "avg": { "sampleInterval": 1 },
                  "dimensions": {
                    "datetimeHour": "2026-09-07T09:00:00Z",
                    "userAgent": "Mozilla/5.0 compatible; GPTBot/1.4",
                    "clientRequestHTTPHost": "example.com",
                    "clientRequestPath": "/blog"
                  }
                }]
              }]
            }
          }
        }"#;
        let rows = parse_graphql_groups(json).expect("parse");
        assert_eq!(rows.len(), 1);
        let key = rows[0].bucket_key().expect("key");
        assert_eq!(key.1, "mozilla/5.0 compatible; gptbot/1.4");
        assert_eq!(key.2, "example.com");
        assert_eq!(key.3, "/blog");
        assert_eq!(rows[0].estimated_count(), 1);
    }

    #[test]
    fn graphql_errors_are_rejected() {
        let json = r#"{ "errors": [{ "message": "cannot request data older than 86400s" }] }"#;
        let error = parse_graphql_groups(json).expect_err("errors");
        assert!(error.to_string().contains("86400"));
    }

    #[test]
    fn never_decreases_estimated_count() {
        let conn = open_migrated_db();
        conn.execute(
            "INSERT INTO useragents (useragent) VALUES ('gptbot/1.4')",
            [],
        )
        .unwrap();
        let useragent_id: i64 = conn
            .query_row("SELECT id FROM useragents", [], |row| row.get(0))
            .unwrap();
        let hour = 1_788_000_000;
        let path_id = upsert_crawler_path(&conn, "/blog", "example.com").unwrap();
        assert_eq!(
            upsert_cloudflare_crawler_view(&conn, hour, useragent_id, path_id, 10).unwrap(),
            CloudflareUpsert::Inserted
        );
        assert_eq!(
            upsert_cloudflare_crawler_view(&conn, hour, useragent_id, path_id, 4).unwrap(),
            CloudflareUpsert::KeptHigher {
                stored: 10,
                fetched: 4
            }
        );
        let stored: i64 = conn
            .query_row(
                "SELECT estimated_count FROM crawler_views_by_hour",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, 10);
        assert_eq!(
            upsert_cloudflare_crawler_view(&conn, hour, useragent_id, path_id, 12).unwrap(),
            CloudflareUpsert::Updated
        );
        let stored: i64 = conn
            .query_row(
                "SELECT estimated_count FROM crawler_views_by_hour",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, 12);
    }

    #[test]
    fn write_groups_upserts_crawler_paths_without_touching_paths() {
        let conn = open_migrated_db();
        let hour = 1_788_000_000;
        let mut groups: FxHashMap<(i64, String, String, String), i64> = FxHashMap::default();
        groups.insert(
            (
                hour,
                "gptbot/1.4".into(),
                "example.com".into(),
                "/blog".into(),
            ),
            4,
        );
        groups.insert(
            (
                hour,
                "gptbot/1.4".into(),
                "example.com".into(),
                "/robots.txt".into(),
            ),
            2,
        );
        write_groups(&conn, groups).expect("write");
        let path_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM paths", [], |row| row.get(0))
            .unwrap();
        assert_eq!(path_count, 0);
        let crawler_path_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM crawler_paths", [], |row| row.get(0))
            .unwrap();
        assert_eq!(crawler_path_count, 2);
        let view_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM crawler_views_by_hour", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(view_count, 2);
    }

    #[test]
    fn overlapping_token_groups_keep_the_max() {
        let mut groups: FxHashMap<(i64, String, String, String), i64> = FxHashMap::default();
        let key = (
            1_788_000_000_i64,
            "mozilla/5.0 compatible; applebot-extended/0.1".into(),
            "example.com".into(),
            "/".into(),
        );
        *groups.entry(key.clone()).or_insert(0) = (*groups.get(&key).unwrap_or(&0)).max(3);
        *groups.entry(key.clone()).or_insert(0) = (*groups.get(&key).unwrap_or(&0)).max(5);
        assert_eq!(groups.get(&key).copied(), Some(5));
    }

    #[test]
    fn apex_domain_uses_registrable_etld_plus_one() {
        assert_eq!(
            apex_domain("www.example.com").as_deref(),
            Some("example.com")
        );
        assert_eq!(
            apex_domain("app.shop.example.co.uk").as_deref(),
            Some("example.co.uk")
        );
        assert_eq!(apex_domain("EXAMPLE.COM.").as_deref(), Some("example.com"));
        assert_eq!(
            apex_domain("example.com:443").as_deref(),
            Some("example.com")
        );
        assert_eq!(apex_domain("192.168.1.1"), None);
        assert_eq!(apex_domain("localhost"), None);
        assert_eq!(apex_domain(""), None);
    }

    #[test]
    fn apex_domains_from_paths_are_unique_and_sorted() {
        let conn = open_migrated_db();
        conn.execute_batch(
            "INSERT INTO paths (path, hostname) VALUES
                ('/', 'www.example.com'),
                ('/blog', 'blog.example.com'),
                ('/', 'shop.example.co.uk'),
                ('/x', '192.168.0.1');",
        )
        .unwrap();
        assert_eq!(
            apex_domains_from_paths(&conn).unwrap(),
            vec!["example.co.uk".to_string(), "example.com".to_string()]
        );
    }

    #[test]
    fn parses_matching_zones_and_rejects_api_errors() {
        let json = r#"{
          "success": true,
          "result": [
            {"id": "zone-1", "name": "example.com"},
            {"id": "zone-2", "name": "example.co.uk"}
          ]
        }"#;
        let zones = parse_zones_response(json).expect("parse");
        assert_eq!(zones.len(), 2);
        assert_eq!(zones[0].id, "zone-1");
        assert_eq!(zones[0].name, "example.com");

        let error = parse_zones_response(
            r#"{ "success": false, "errors": [{ "message": "Authentication error" }] }"#,
        )
        .expect_err("auth");
        assert!(error.to_string().contains("Authentication error"));
    }
}
