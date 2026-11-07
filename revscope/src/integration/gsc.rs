use std::sync::Arc;

use std::time::Duration;

use fxhash::FxHashMap;
use log::{info, warn};
use reqwest::StatusCode;
use reqwest::header::{HeaderMap, RETRY_AFTER};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use time::{Date, Month, OffsetDateTime, UtcOffset};
use url::Url;

use super::google_auth::{self, GOOGLE_TOKEN_URL, GoogleToken, GoogleTokenError};
use super::schedule::{self, SYNC_BATCH_SIZE, SYNC_THROTTLE_SECS, SyncOutcome};
use super::sync::{SyncCounts, SyncError, SyncKind, SyncResult, sleep_or_shutdown};
use crate::crypto;
use crate::db::{
    ProjectCache, ProjectConnection, open_main_database_read, open_main_database_write,
};
use crate::server_context::ServerContext;
use crate::util::unix_seconds;

const GSC_FULL_SYNC_INTERVAL_SECS: i64 = 24 * 60 * 60;
const GSC_ROW_LIMIT: i32 = 25_000;
/// Search Console publishes a day's data a couple of days late; don't ask for anything newer.
const GSC_DATA_DELAY_DAYS: i64 = 3;
/// Search Console retains 16 months of history.
const GSC_BACKFILL_DAYS: i64 = 16 * 31;
/// Trailing days re-fetched on every sync once the backfill has caught up.
const GSC_INCREMENTAL_DAYS: i64 = 14;
/// Search Console keeps revising a day's numbers for about two weeks; `final` is
/// only stable past that, so recent days are requested as `all` instead.
const GSC_FINAL_DATA_DAYS: i64 = 14;
const GSC_DATA_STATE_FINAL: &str = "final";
const GSC_DATA_STATE_ALL: &str = "all";
/// Search Console reports calendar dates in US Pacific time. A fixed offset is
/// close enough: it only shifts the cutover by an hour during daylight saving,
/// and [`GSC_DATA_DELAY_DAYS`] already keeps us well clear of the boundary.
const GSC_REPORT_UTC_OFFSET_HOURS: i8 = -8;
/// Per-request attempts for rate limits and transient server errors.
const GSC_REQUEST_MAX_ATTEMPTS: usize = 5;
const GSC_REQUEST_BASE_BACKOFF: Duration = Duration::from_secs(2);
const GSC_REQUEST_MAX_BACKOFF: Duration = Duration::from_secs(60);
/// Renew the access token this far ahead of expiry; a full backfill outlives one token.
const GSC_TOKEN_REFRESH_MARGIN_SECS: i64 = 300;
/// Average position is a ratio, so compare with a tolerance rather than bit-exactly.
const GSC_POSITION_EPSILON: f64 = 1e-6;
const GSC_API_BASE_URL: &str = "https://www.googleapis.com/webmasters/v3";
const GSC_SCOPE: &str = "https://www.googleapis.com/auth/webmasters.readonly";

/// Loads Google Search Console integrations that are due for their daily full sync.
///
/// `last_full_sync_success_at` only advances once a sync has walked its window all
/// the way to the newest available day, so an interrupted backfill falls back to
/// the [`SYNC_THROTTLE_SECS`] retry cadence instead of waiting a full day.
pub(crate) async fn load_due_integrations(
    server_context: Arc<ServerContext>,
) -> SyncResult<Vec<GscIntegration>> {
    let conn = open_main_database_read(server_context).await?;
    let rows = conn
        .query(
            &format!(
                "SELECT
                    id,
                    project_id::text AS project_id,
                    external_account_id,
                    encrypted_api_key,
                    encrypted_secret,
                    synced_through_date::text,
                    credentials_version
                 FROM integrations
                 WHERE disabled_at IS NULL
                    AND integration_type = 'google_search_console'::integration_type
                    AND external_account_id IS NOT NULL
                    AND encrypted_api_key IS NOT NULL
                    AND encrypted_secret IS NOT NULL
                    AND (last_sync_completed_at IS NULL
                         OR last_sync_completed_at < NOW() - INTERVAL '{SYNC_THROTTLE_SECS} seconds')
                    AND (last_full_sync_success_at IS NULL
                         OR last_full_sync_success_at < NOW() - INTERVAL '{GSC_FULL_SYNC_INTERVAL_SECS} seconds')
                 ORDER BY last_full_sync_success_at ASC NULLS FIRST
                 LIMIT {SYNC_BATCH_SIZE}"
            ),
            &[],
        )
        .await?;

    Ok(rows
        .iter()
        .map(|row| GscIntegration {
            id: row.get(0),
            project_id: row.get(1),
            site_url: row.get(2),
            encrypted_client_email: row.get(3),
            encrypted_private_key: row.get(4),
            synced_through: row.get::<_, Option<&str>>(5).and_then(parse_iso_date),
            credentials_version: row.get(6),
        })
        .collect())
}

pub(crate) async fn run_sync_with_retries(
    server_context: Arc<ServerContext>,
    integration: GscIntegration,
) {
    schedule::run_sync_with_retries(
        server_context.clone(),
        "gsc",
        integration.id,
        integration.credentials_version,
        SyncKind::Full,
        || async { sync_integration(server_context.clone(), &integration).await },
    )
    .await;
}

pub(crate) struct GscIntegration {
    id: i32,
    project_id: String,
    site_url: String,
    encrypted_client_email: Option<Vec<u8>>,
    encrypted_private_key: Option<Vec<u8>>,
    /// Newest report date already written, used to resume an interrupted backfill.
    synced_through: Option<Date>,
    credentials_version: i64,
}

/// Walks the integration's pending report days oldest-first, committing and
/// recording each day on its own so an interrupted run resumes where it stopped.
///
/// The sync scheduler runs integrations one at a time, so this is already the only
/// Search Console sync in flight for the process.
async fn sync_integration(
    server_context: Arc<ServerContext>,
    integration: &GscIntegration,
) -> SyncResult<SyncOutcome> {
    let project =
        super::sync::ensure_project_cached(server_context.clone(), &integration.project_id).await?;
    let client_email = decrypt_credential(
        integration.encrypted_client_email.as_deref(),
        "service account email",
    )?;
    let private_key = decrypt_credential(
        integration.encrypted_private_key.as_deref(),
        "service account private key",
    )?;
    let client = &server_context.http;
    let mut token = GscAccessToken::default();

    let today = gsc_report_today()?;
    let newest = today - time::Duration::days(GSC_DATA_DELAY_DAYS);
    let oldest = gsc_sync_start_date(newest, integration.synced_through);

    let mut counts = SyncCounts::default();
    let mut synced_through = integration.synced_through;
    let mut days = 0;
    let mut day = oldest;
    let mut caught_up = true;
    while day <= newest {
        if server_context.shutdown_requested() {
            caught_up = false;
            break;
        }
        // A backfill takes one request per day of history, and the new site's sync
        // can't start until this one ends.
        if !super::sync::credentials_are_current(
            server_context.clone(),
            integration.id,
            integration.credentials_version,
        )
        .await
        {
            return Err(SyncError::CredentialsChanged);
        }
        let access_token = token.get(client, &client_email, &private_key).await?;
        let snapshot = fetch_day_snapshot(
            &server_context,
            client,
            GSC_API_BASE_URL,
            &integration.site_url,
            &access_token,
            day,
            gsc_data_state(today, day),
        )
        .await?;
        add_counts(
            &mut counts,
            write_day_snapshot(&server_context, &project, day, snapshot)?,
        );
        if synced_through.is_none_or(|cursor| day > cursor) {
            record_synced_through(
                server_context.clone(),
                integration.id,
                integration.credentials_version,
                day,
            )
            .await?;
            synced_through = Some(day);
        }
        days += 1;
        day = day.next_day().ok_or_else(|| {
            SyncError::Gsc("GSC sync ran past the last representable date".into())
        })?;
    }

    info!(
        "synced GSC integration {} ({days} day(s) {}..={}, caught_up={caught_up}): inserted={}, updated={}, deleted={}",
        integration.id,
        format_date(oldest),
        format_date(newest),
        counts.records_inserted,
        counts.records_updated,
        counts.records_deleted
    );
    // A sync counts as full only once its window reached the newest available day.
    Ok(SyncOutcome {
        counts,
        sync_kind: if caught_up {
            SyncKind::Full
        } else {
            SyncKind::Incremental
        },
    })
}

/// The oldest report day still to fetch.
///
/// A first sync walks the whole retained history. Once the cursor sits inside the
/// trailing window we only re-fetch those days, whose numbers Search Console is
/// still revising. A cursor behind that window means an earlier backfill was cut
/// short, so it picks up from the day after the last one written.
fn gsc_sync_start_date(newest: Date, synced_through: Option<Date>) -> Date {
    let oldest = newest - time::Duration::days(GSC_BACKFILL_DAYS);
    let incremental_start = newest - time::Duration::days(GSC_INCREMENTAL_DAYS - 1);
    match synced_through {
        Some(cursor) if cursor >= incremental_start => incremental_start,
        Some(cursor) if cursor >= oldest => cursor.next_day().unwrap_or(cursor),
        _ => oldest,
    }
}

/// `final` is only published for settled days; ask for `all` while a day is still
/// being revised so fresh partial data isn't reported as zero.
fn gsc_data_state(today: Date, date: Date) -> &'static str {
    if (today - date).whole_days() > GSC_FINAL_DATA_DAYS {
        GSC_DATA_STATE_FINAL
    } else {
        GSC_DATA_STATE_ALL
    }
}

fn decrypt_credential(encrypted: Option<&[u8]>, label: &'static str) -> SyncResult<String> {
    let encrypted = encrypted.ok_or(SyncError::MissingGscCredential(label))?;
    let decrypted = crypto::decrypt(encrypted).ok_or(SyncError::InvalidGscCredential(label))?;
    String::from_utf8(decrypted).map_err(|_| SyncError::InvalidGscCredential(label))
}

/// Caches a service account access token and renews it before it expires, since a
/// full backfill runs longer than a token's lifetime.
#[derive(Default)]
struct GscAccessToken {
    token: String,
    expires_at: i64,
}

impl GscAccessToken {
    async fn get(
        &mut self,
        client: &reqwest::Client,
        client_email: &str,
        private_key: &str,
    ) -> SyncResult<String> {
        if unix_seconds() + GSC_TOKEN_REFRESH_MARGIN_SECS >= self.expires_at {
            let fetched = fetch_access_token(client, client_email, private_key).await?;
            self.expires_at = unix_seconds() + fetched.expires_in;
            self.token = fetched.access_token;
        }
        Ok(self.token.clone())
    }
}

async fn fetch_access_token(
    client: &reqwest::Client,
    client_email: &str,
    private_key: &str,
) -> SyncResult<GoogleToken> {
    google_auth::service_account_token(
        client,
        GOOGLE_TOKEN_URL,
        client_email,
        private_key,
        GSC_SCOPE,
    )
    .await
    .map_err(|error| match error {
        GoogleTokenError::Http(error) => error.into(),
        GoogleTokenError::Unavailable(message)
        | GoogleTokenError::ClientRejected(message)
        | GoogleTokenError::Rejected(message)
        | GoogleTokenError::InvalidCredential(message) => SyncError::Gsc(format!("GSC {message}")),
    })
}

/// Aggregates one report day across the query, device and page breakdowns.
///
/// Each request covers a single day, so the `date` dimension is left out and the
/// day is known from the caller instead.
async fn fetch_day_snapshot(
    server_context: &ServerContext,
    client: &reqwest::Client,
    api_base_url: &str,
    site_url: &str,
    access_token: &str,
    date: Date,
    data_state: &'static str,
) -> SyncResult<GscDaySnapshot> {
    let request = GscDayRequest {
        server_context,
        client,
        url: search_analytics_url(api_base_url, site_url),
        access_token,
        date: format_date(date),
        data_state,
    };
    let mut daily: FxHashMap<String, DailyAggregate> = FxHashMap::default();
    let mut paths: FxHashMap<(String, String, String), i64> = FxHashMap::default();

    for row in request.rows(&["query"]).await? {
        let Some(query) = row.keys.first().and_then(|key| normalize_query(key)) else {
            continue;
        };
        let entry = daily.entry(query).or_default();
        entry.clicks += metric_to_i64(row.clicks);
        entry.impressions += metric_to_i64(row.impressions);
        entry.weighted_position += row.position.max(0.0) * row.impressions.max(0.0);
    }

    for row in request.rows(&["query", "device"]).await? {
        if row.keys.len() != 2 {
            continue;
        }
        let Some(query) = normalize_query(&row.keys[0]) else {
            continue;
        };
        let Some(is_mobile) = gsc_device_is_mobile(&row.keys[1]) else {
            continue;
        };
        let clicks = metric_to_i64(row.clicks);
        if clicks <= 0 {
            continue;
        }
        let entry = daily.entry(query).or_default();
        if is_mobile == 1 {
            entry.mobile_clicks += clicks;
        } else {
            entry.desktop_clicks += clicks;
        }
    }

    for row in request.rows(&["query", "page"]).await? {
        if row.keys.len() != 2 {
            continue;
        }
        let Some(query) = normalize_query(&row.keys[0]) else {
            continue;
        };
        let Some(page) = gsc_page_path(&row.keys[1], site_url) else {
            continue;
        };
        let clicks = metric_to_i64(row.clicks);
        if clicks <= 0 {
            continue;
        }
        *paths.entry((query, page.hostname, page.path)).or_insert(0) += clicks;
    }

    Ok(GscDaySnapshot { daily, paths })
}

/// Everything a search analytics request for one report day needs apart from the
/// dimensions being broken out.
struct GscDayRequest<'a> {
    server_context: &'a ServerContext,
    client: &'a reqwest::Client,
    url: String,
    access_token: &'a str,
    date: String,
    data_state: &'static str,
}

impl GscDayRequest<'_> {
    async fn rows(&self, dimensions: &[&'static str]) -> SyncResult<Vec<GscSearchAnalyticsRow>> {
        let mut start_row = 0;
        let mut rows = Vec::new();
        loop {
            let body = GscSearchAnalyticsRequest {
                start_date: &self.date,
                end_date: &self.date,
                dimensions,
                row_limit: GSC_ROW_LIMIT,
                start_row,
                data_state: self.data_state,
            };
            let text = send_with_retries(self.server_context, "GSC search analytics query", || {
                self.client
                    .post(&self.url)
                    .timeout(super::REPORT_QUERY_TIMEOUT)
                    .bearer_auth(self.access_token)
                    .json(&body)
            })
            .await?;

            let response: GscSearchAnalyticsResponse = serde_json::from_str(&text)?;
            let mut page_rows = response.rows.unwrap_or_default();
            let page_len = page_rows.len();
            rows.append(&mut page_rows);
            if page_len < GSC_ROW_LIMIT as usize {
                break;
            }
            start_row += GSC_ROW_LIMIT;
        }
        Ok(rows)
    }
}

/// Sends a request, retrying rate limits, transient server errors and dropped
/// connections. A `Retry-After` from Search Console wins over the exponential
/// backoff. A backfill makes thousands of requests, so a single blip shouldn't
/// cost the whole run.
async fn send_with_retries(
    server_context: &ServerContext,
    label: &str,
    build_request: impl Fn() -> reqwest::RequestBuilder,
) -> SyncResult<String> {
    let mut attempt = 1;
    loop {
        let can_retry = attempt < GSC_REQUEST_MAX_ATTEMPTS;
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
                    "{label} could not reach Search Console ({error}), retrying in {delay:?} (attempt {attempt}/{GSC_REQUEST_MAX_ATTEMPTS})"
                );
                sleep_before_retry(server_context, label, delay).await?;
            }
            Ok((status, retry_after, text)) => {
                if status.is_success() {
                    return Ok(text);
                }
                if !can_retry || !is_retryable_response(status, &text) {
                    return Err(SyncError::Gsc(format!(
                        "{label} failed with {status}: {text}"
                    )));
                }
                let delay = retry_after.unwrap_or_else(|| retry_backoff(attempt));
                warn!(
                    "{label} failed with {status}, retrying in {delay:?} (attempt {attempt}/{GSC_REQUEST_MAX_ATTEMPTS})"
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
        return Err(SyncError::Gsc(format!(
            "{label} abandoned: shutdown requested while backing off"
        )));
    }
    Ok(())
}

fn is_retryable_transport_error(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect()
}

fn is_retryable_response(status: StatusCode, body: &str) -> bool {
    if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
        return true;
    }
    // Google reports exhausted per-minute quota as a 403 rather than a 429.
    status == StatusCode::FORBIDDEN
        && (body.contains("rateLimitExceeded") || body.contains("userRateLimitExceeded"))
}

/// Only the delay-seconds form is honored. Google sends seconds; the HTTP-date
/// form falls back to the exponential backoff.
///
/// Capped like the backoff. The scheduler syncs integrations one at a time, so
/// sitting out a long quota window here would stall every other integration; the
/// run is picked up again on the throttle interval instead.
fn retry_after_delay(headers: &HeaderMap) -> Option<Duration> {
    let seconds: u64 = headers
        .get(RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()?;
    Some(Duration::from_secs(seconds).min(GSC_REQUEST_MAX_BACKOFF))
}

fn retry_backoff(attempt: usize) -> Duration {
    let exponent = u32::try_from(attempt.saturating_sub(1)).unwrap_or(u32::MAX);
    GSC_REQUEST_BASE_BACKOFF
        .checked_mul(2_u32.saturating_pow(exponent))
        .unwrap_or(GSC_REQUEST_MAX_BACKOFF)
        .min(GSC_REQUEST_MAX_BACKOFF)
}

async fn record_synced_through(
    server_context: Arc<ServerContext>,
    integration_id: i32,
    credentials_version: i64,
    date: Date,
) -> SyncResult<()> {
    let conn = open_main_database_write(server_context).await?;
    // GREATEST ignores NULLs, so the cursor only ever moves forward even when the
    // trailing window re-syncs days that are already covered. The version check
    // keeps a sync of the old site from setting the cursor a new site's backfill
    // starts from, which would skip that site's history for good.
    conn.execute(
        "UPDATE integrations
         SET synced_through_date = GREATEST(synced_through_date, $2::text::date)
         WHERE id = $1 AND credentials_version = $3",
        &[&integration_id, &format_date(date), &credentials_version],
    )
    .await?;
    Ok(())
}

/// Commits one report day on its own so a failure part-way through a backfill
/// keeps the days already written.
fn write_day_snapshot(
    server_context: &ServerContext,
    project: &Arc<ProjectCache>,
    date: Date,
    snapshot: GscDaySnapshot,
) -> SyncResult<SyncCounts> {
    let conn = crate::db::open_project_database_write_for(server_context, project)?;
    let mut counts = SyncCounts::default();

    conn.execute("BEGIN IMMEDIATE", [])?;
    match write_day_in_transaction(&conn, date, snapshot, &mut counts) {
        Ok(()) => {
            conn.execute("COMMIT", [])?;
            Ok(counts)
        }
        Err(error) => {
            if let Err(rollback_error) = conn.execute("ROLLBACK", []) {
                warn!("failed to roll back GSC search data for a day: {rollback_error}");
            }
            Err(error)
        }
    }
}

fn write_day_in_transaction(
    conn: &ProjectConnection,
    date: Date,
    snapshot: GscDaySnapshot,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    let day = date_to_unix_utc_midnight(date);
    let mut query_ids = FxHashMap::default();
    write_daily_rows(conn, day, snapshot.daily, &mut query_ids, counts)?;
    write_path_rows(conn, day, snapshot.paths, &mut query_ids, counts)?;
    Ok(())
}

/// Reconciles `search_queries_by_date` for one day against the fetched totals:
/// rows that vanished are deleted, rows whose numbers moved are updated, and rows
/// that match are left untouched so unchanged days cost no writes.
fn write_daily_rows(
    conn: &ProjectConnection,
    day: i64,
    mut fetched: FxHashMap<String, DailyAggregate>,
    query_ids: &mut FxHashMap<String, i64>,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    let stored = conn.with_query(
        "SELECT q.query, d.query_id, d.position, d.impressions, d.clicks,
                d.mobile_clicks, d.desktop_clicks
         FROM search_queries_by_date d
         JOIN search_queries q ON q.id = d.query_id
         WHERE d.date = ?1",
        params![day],
        |mut rows| {
            let mut stored = Vec::new();
            while let Some(row) = rows.next()? {
                stored.push((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    DailyRow {
                        position: row.get(2)?,
                        impressions: row.get(3)?,
                        clicks: row.get(4)?,
                        mobile_clicks: row.get(5)?,
                        desktop_clicks: row.get(6)?,
                    },
                ));
            }
            Ok(stored)
        },
    )?;

    for (query, query_id, current) in stored {
        query_ids.insert(query.clone(), query_id);
        match fetched.remove(&query) {
            Some(aggregate) => {
                let wanted = DailyRow::from(&aggregate);
                if !wanted.matches(&current) {
                    conn.execute(
                        "UPDATE search_queries_by_date
                         SET position = ?3,
                             impressions = ?4,
                             clicks = ?5,
                             mobile_clicks = ?6,
                             desktop_clicks = ?7
                         WHERE date = ?1 AND query_id = ?2",
                        params![
                            day,
                            query_id,
                            wanted.position,
                            wanted.impressions,
                            wanted.clicks,
                            wanted.mobile_clicks,
                            wanted.desktop_clicks
                        ],
                    )?;
                    counts.records_updated = counts.records_updated.saturating_add(1);
                }
            }
            None => {
                conn.execute(
                    "DELETE FROM search_queries_by_date WHERE date = ?1 AND query_id = ?2",
                    params![day, query_id],
                )?;
                counts.records_deleted = counts.records_deleted.saturating_add(1);
            }
        }
    }

    for (query, aggregate) in fetched {
        let query_id = search_query_id(conn, query_ids, &query)?;
        let row = DailyRow::from(&aggregate);
        conn.execute(
            "INSERT INTO search_queries_by_date
                (date, query_id, position, impressions, clicks, mobile_clicks, desktop_clicks)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                day,
                query_id,
                row.position,
                row.impressions,
                row.clicks,
                row.mobile_clicks,
                row.desktop_clicks
            ],
        )?;
        counts.records_inserted = counts.records_inserted.saturating_add(1);
    }

    Ok(())
}

/// Reconciles `search_query_paths_by_date` for one day, keyed by query and page.
fn write_path_rows(
    conn: &ProjectConnection,
    day: i64,
    mut fetched: FxHashMap<(String, String, String), i64>,
    query_ids: &mut FxHashMap<String, i64>,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    let stored = conn.with_query(
        "SELECT q.query, p.hostname, p.path, sp.query_id, sp.path_id, sp.clicks
         FROM search_query_paths_by_date sp
         JOIN search_queries q ON q.id = sp.query_id
         JOIN paths p ON p.id = sp.path_id
         WHERE sp.date = ?1",
        params![day],
        |mut rows| {
            let mut stored = Vec::new();
            while let Some(row) = rows.next()? {
                stored.push((
                    (
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ),
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                ));
            }
            Ok(stored)
        },
    )?;

    let mut path_ids = FxHashMap::default();
    for (key, query_id, path_id, current_clicks) in stored {
        query_ids.insert(key.0.clone(), query_id);
        path_ids.insert((key.1.clone(), key.2.clone()), path_id);
        match fetched.remove(&key) {
            Some(clicks) => {
                if clicks != current_clicks {
                    conn.execute(
                        "UPDATE search_query_paths_by_date
                         SET clicks = ?4
                         WHERE date = ?1 AND query_id = ?2 AND path_id = ?3",
                        params![day, query_id, path_id, clicks],
                    )?;
                    counts.records_updated = counts.records_updated.saturating_add(1);
                }
            }
            None => {
                conn.execute(
                    "DELETE FROM search_query_paths_by_date
                     WHERE date = ?1 AND query_id = ?2 AND path_id = ?3",
                    params![day, query_id, path_id],
                )?;
                counts.records_deleted = counts.records_deleted.saturating_add(1);
            }
        }
    }

    for ((query, hostname, path), clicks) in fetched {
        let query_id = search_query_id(conn, query_ids, &query)?;
        let path_id = path_id(conn, &mut path_ids, &hostname, &path)?;
        conn.execute(
            "INSERT INTO search_query_paths_by_date (date, query_id, path_id, clicks)
             VALUES (?1, ?2, ?3, ?4)",
            params![day, query_id, path_id, clicks],
        )?;
        counts.records_inserted = counts.records_inserted.saturating_add(1);
    }

    Ok(())
}

fn search_query_id(
    conn: &ProjectConnection,
    cache: &mut FxHashMap<String, i64>,
    query: &str,
) -> SyncResult<i64> {
    if let Some(id) = cache.get(query) {
        return Ok(*id);
    }
    conn.execute(
        "INSERT INTO search_queries (query)
         VALUES (?1)
         ON CONFLICT(query) DO NOTHING",
        params![query],
    )?;
    let id = conn.query_row(
        "SELECT id FROM search_queries WHERE query = ?1",
        params![query],
        |row| row.get::<_, i64>(0),
    )?;
    cache.insert(query.to_string(), id);
    Ok(id)
}

fn path_id(
    conn: &ProjectConnection,
    cache: &mut FxHashMap<(String, String), i64>,
    hostname: &str,
    path: &str,
) -> SyncResult<i64> {
    let key = (hostname.to_string(), path.to_string());
    if let Some(id) = cache.get(&key) {
        return Ok(*id);
    }
    conn.execute(
        "INSERT INTO paths (path, hostname)
         VALUES (?1, ?2)
         ON CONFLICT(path, hostname) DO NOTHING",
        params![path, hostname],
    )?;
    let id = conn.query_row(
        "SELECT id FROM paths WHERE hostname = ?1 AND path = ?2",
        params![hostname, path],
        |row| row.get::<_, i64>(0),
    )?;
    cache.insert(key, id);
    Ok(id)
}

fn gsc_report_today() -> SyncResult<Date> {
    gsc_report_date(unix_seconds())
}

fn gsc_report_date(unix_seconds: i64) -> SyncResult<Date> {
    let offset = UtcOffset::from_hms(GSC_REPORT_UTC_OFFSET_HOURS, 0, 0)
        .map_err(|error| SyncError::Gsc(error.to_string()))?;
    Ok(OffsetDateTime::from_unix_timestamp(unix_seconds)
        .map_err(|error| SyncError::Gsc(error.to_string()))?
        .to_offset(offset)
        .date())
}

fn parse_iso_date(value: &str) -> Option<Date> {
    let mut parts = value.split('-');
    let year = parts.next()?.parse::<i32>().ok()?;
    let month = Month::try_from(parts.next()?.parse::<u8>().ok()?).ok()?;
    let day = parts.next()?.parse::<u8>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Date::from_calendar_date(year, month, day).ok()
}

/// Report dates are stored as the UTC midnight of the calendar day, matching the
/// `date` columns the dashboard queries.
fn date_to_unix_utc_midnight(date: Date) -> i64 {
    date.midnight().assume_utc().unix_timestamp()
}

fn format_date(date: Date) -> String {
    format!(
        "{:04}-{:02}-{:02}",
        date.year(),
        u8::from(date.month()),
        date.day()
    )
}

fn search_analytics_url(api_base_url: &str, site_url: &str) -> String {
    format!(
        "{}/sites/{}/searchAnalytics/query",
        api_base_url.trim_end_matches('/'),
        percent_encode_path_segment(site_url)
    )
}

fn percent_encode_path_segment(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn gsc_device_is_mobile(value: &str) -> Option<i64> {
    match value {
        "DESKTOP" => Some(0),
        "MOBILE" | "TABLET" => Some(1),
        _ => None,
    }
}

fn gsc_page_path(page_url: &str, site_url: &str) -> Option<GscPagePath> {
    let url = Url::parse(page_url)
        .ok()
        .or_else(|| Url::parse(site_url).ok()?.join(page_url).ok())?;
    let hostname = url.host_str()?.to_ascii_lowercase();
    let path = if url.path().is_empty() {
        "/".to_string()
    } else {
        url.path().to_string()
    };
    Some(GscPagePath { hostname, path })
}

fn normalize_query(query: &str) -> Option<String> {
    let query = query.trim().to_lowercase();
    if query.is_empty() { None } else { Some(query) }
}

fn metric_to_i64(value: f64) -> i64 {
    if !value.is_finite() || value <= 0.0 {
        0
    } else if value >= i64::MAX as f64 {
        i64::MAX
    } else {
        value.round() as i64
    }
}

fn add_counts(total: &mut SyncCounts, day: SyncCounts) {
    total.records_inserted = total.records_inserted.saturating_add(day.records_inserted);
    total.records_updated = total.records_updated.saturating_add(day.records_updated);
    total.records_deleted = total.records_deleted.saturating_add(day.records_deleted);
}

#[derive(Default)]
struct DailyAggregate {
    clicks: i64,
    impressions: i64,
    weighted_position: f64,
    mobile_clicks: i64,
    desktop_clicks: i64,
}

/// A `search_queries_by_date` row as stored, used to compare fetched against saved.
struct DailyRow {
    position: f64,
    impressions: i64,
    clicks: i64,
    mobile_clicks: i64,
    desktop_clicks: i64,
}

impl DailyRow {
    fn matches(&self, other: &Self) -> bool {
        self.impressions == other.impressions
            && self.clicks == other.clicks
            && self.mobile_clicks == other.mobile_clicks
            && self.desktop_clicks == other.desktop_clicks
            && (self.position - other.position).abs() < GSC_POSITION_EPSILON
    }
}

impl From<&DailyAggregate> for DailyRow {
    fn from(aggregate: &DailyAggregate) -> Self {
        Self {
            position: if aggregate.impressions > 0 {
                aggregate.weighted_position / aggregate.impressions as f64
            } else {
                0.0
            },
            impressions: aggregate.impressions,
            clicks: aggregate.clicks,
            mobile_clicks: aggregate.mobile_clicks,
            desktop_clicks: aggregate.desktop_clicks,
        }
    }
}

struct GscDaySnapshot {
    /// `query -> totals` for one report day.
    daily: FxHashMap<String, DailyAggregate>,
    /// `(query, hostname, path) -> clicks` for one report day.
    paths: FxHashMap<(String, String, String), i64>,
}

struct GscPagePath {
    hostname: String,
    path: String,
}

#[derive(Serialize)]
struct GscSearchAnalyticsRequest<'a> {
    #[serde(rename = "startDate")]
    start_date: &'a str,
    #[serde(rename = "endDate")]
    end_date: &'a str,
    dimensions: &'a [&'static str],
    #[serde(rename = "rowLimit")]
    row_limit: i32,
    #[serde(rename = "startRow")]
    start_row: i32,
    #[serde(rename = "dataState")]
    data_state: &'static str,
}

#[derive(Deserialize)]
struct GscSearchAnalyticsResponse {
    rows: Option<Vec<GscSearchAnalyticsRow>>,
}

#[derive(Deserialize)]
struct GscSearchAnalyticsRow {
    keys: Vec<String>,
    clicks: f64,
    impressions: f64,
    position: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::open_migrated_db;

    #[tokio::test]
    async fn a_backfill_stops_when_the_site_changes() {
        let (server_context, conn) = crate::test_utils::main_test_db("gsc-stop").await;
        let account_id = crate::test_utils::insert_test_account(&conn, "gsc_stop").await;
        let name = format!("p{}", account_id.replace('-', ""));
        let project_id: i32 = conn
            .query_one(
                "INSERT INTO projects (account_id, name) VALUES ($1::text::uuid, $2) RETURNING id",
                &[&account_id, &name],
            )
            .await
            .expect("insert project")
            .get(0);
        let integration_id: i32 = conn
            .query_one(
                "INSERT INTO integrations (project_id, integration_type, credentials_version)
                 VALUES ($1, 'google_search_console', 1) RETURNING id",
                &[&project_id],
            )
            .await
            .expect("insert integration")
            .get(0);
        let integration = GscIntegration {
            id: integration_id,
            project_id: project_id.to_string(),
            site_url: "sc-domain:old.example".to_string(),
            encrypted_client_email: Some(crate::crypto::encrypt(b"sync@example.com")),
            encrypted_private_key: Some(crate::crypto::encrypt(b"not used")),
            synced_through: None,
            credentials_version: 0,
        };

        // The check comes before the first request, so no provider is needed.
        let outcome = sync_integration(server_context.clone(), &integration).await;

        assert!(matches!(outcome, Err(SyncError::CredentialsChanged)));
        conn.execute(
            "DELETE FROM accounts WHERE id = $1::text::uuid",
            &[&account_id],
        )
        .await
        .expect("delete account");
    }

    #[tokio::test]
    async fn a_new_site_still_backfills_after_a_sync_of_the_old_site_finishes() {
        let (server_context, conn) = crate::test_utils::main_test_db("gsc-cursor-race").await;
        let account_id = crate::test_utils::insert_test_account(&conn, "gsc_cursor_race").await;
        let name = format!("p{}", account_id.replace('-', ""));
        let project_id: i32 = conn
            .query_one(
                "INSERT INTO projects (account_id, name) VALUES ($1::text::uuid, $2) RETURNING id",
                &[&account_id, &name],
            )
            .await
            .expect("insert project")
            .get(0);
        let integration_id: i32 = conn
            .query_one(
                "INSERT INTO integrations
                    (project_id, integration_type, external_account_id, encrypted_api_key)
                 VALUES ($1, 'google_search_console', 'sc-domain:old.example', '\\x00')
                 RETURNING id",
                &[&project_id],
            )
            .await
            .expect("insert integration")
            .get(0);
        let newest = date(2026, 6, 1);

        crate::settings::upsert_integration(
            &conn,
            project_id,
            &revscope_api::settings::IntegrationInput {
                integration_type: "google_search_console".to_string(),
                external_account_id: Some("sc-domain:new.example".to_string()),
                api_key: None,
                secret: None,
            },
        )
        .await
        .expect("save new site");
        record_synced_through(server_context.clone(), integration_id, 0, newest)
            .await
            .expect("record old site's cursor");

        let cursor = conn
            .query_one(
                "SELECT synced_through_date::text FROM integrations WHERE id = $1",
                &[&integration_id],
            )
            .await
            .expect("load cursor")
            .get::<_, Option<&str>>(0)
            .and_then(parse_iso_date);
        assert_eq!(cursor, None);
        assert_eq!(
            gsc_sync_start_date(newest, cursor),
            newest - time::Duration::days(GSC_BACKFILL_DAYS)
        );

        conn.execute(
            "DELETE FROM accounts WHERE id = $1::text::uuid",
            &[&account_id],
        )
        .await
        .expect("delete account");
    }

    fn date(year: i32, month: u8, day: u8) -> Date {
        Date::from_calendar_date(year, Month::try_from(month).expect("month"), day).expect("date")
    }

    fn aggregate(impressions: i64, clicks: i64, position: f64) -> DailyAggregate {
        DailyAggregate {
            clicks,
            impressions,
            weighted_position: position * impressions as f64,
            mobile_clicks: clicks,
            desktop_clicks: 0,
        }
    }

    fn snapshot(
        daily: Vec<(&str, DailyAggregate)>,
        paths: Vec<((&str, &str, &str), i64)>,
    ) -> GscDaySnapshot {
        GscDaySnapshot {
            daily: daily
                .into_iter()
                .map(|(query, aggregate)| (query.to_string(), aggregate))
                .collect(),
            paths: paths
                .into_iter()
                .map(|((query, hostname, path), clicks)| {
                    (
                        (query.to_string(), hostname.to_string(), path.to_string()),
                        clicks,
                    )
                })
                .collect(),
        }
    }

    fn write_day(conn: &ProjectConnection, day: Date, snapshot: GscDaySnapshot) -> SyncCounts {
        let mut counts = SyncCounts::default();
        write_day_in_transaction(conn, day, snapshot, &mut counts).expect("write day");
        counts
    }

    #[test]
    fn encodes_site_url_as_path_segment() {
        assert_eq!(
            percent_encode_path_segment("sc-domain:example.com"),
            "sc-domain%3Aexample.com"
        );
        assert_eq!(
            percent_encode_path_segment("https://www.example.com/"),
            "https%3A%2F%2Fwww.example.com%2F"
        );
    }

    #[test]
    fn converts_device_dimension() {
        assert_eq!(gsc_device_is_mobile("DESKTOP"), Some(0));
        assert_eq!(gsc_device_is_mobile("MOBILE"), Some(1));
        assert_eq!(gsc_device_is_mobile("TABLET"), Some(1));
        assert_eq!(gsc_device_is_mobile("OTHER"), None);
    }

    #[test]
    fn extracts_page_hostname_and_path() {
        let page =
            gsc_page_path("https://Example.com/docs?q=1", "sc-domain:example.com").expect("page");
        assert_eq!(page.hostname, "example.com");
        assert_eq!(page.path, "/docs");
    }

    #[test]
    fn converts_report_date_to_unix_midnight() {
        assert_eq!(date_to_unix_utc_midnight(date(1970, 1, 2)), 86_400);
    }

    #[test]
    fn parses_stored_cursor_date() {
        assert_eq!(parse_iso_date("2026-08-11"), Some(date(2026, 8, 11)));
        assert_eq!(parse_iso_date("2026-08-11-01"), None);
        assert_eq!(parse_iso_date("2026-13-11"), None);
        assert_eq!(parse_iso_date(""), None);
    }

    #[test]
    fn resolves_report_date_in_us_pacific() {
        // 2026-08-11T05:00:00Z is still 2026-08-10 on the US west coast.
        let timestamp = date(2026, 8, 11)
            .with_hms(5, 0, 0)
            .expect("time")
            .assume_utc()
            .unix_timestamp();
        assert_eq!(gsc_report_date(timestamp).expect("date"), date(2026, 8, 10));
    }

    #[test]
    fn first_sync_walks_the_full_history() {
        let newest = date(2026, 8, 8);
        assert_eq!(
            gsc_sync_start_date(newest, None),
            newest - time::Duration::days(GSC_BACKFILL_DAYS)
        );
    }

    #[test]
    fn caught_up_sync_refreshes_only_the_trailing_window() {
        let newest = date(2026, 8, 8);
        let incremental_start = newest - time::Duration::days(GSC_INCREMENTAL_DAYS - 1);
        assert_eq!(
            gsc_sync_start_date(newest, Some(newest)),
            incremental_start,
            "a cursor at the newest day re-fetches the whole trailing window"
        );
        assert_eq!(
            gsc_sync_start_date(newest, Some(incremental_start)),
            incremental_start
        );
    }

    #[test]
    fn interrupted_backfill_resumes_after_the_last_written_day() {
        let newest = date(2026, 8, 8);
        let cursor = date(2026, 3, 1);
        assert_eq!(
            gsc_sync_start_date(newest, Some(cursor)),
            date(2026, 3, 2),
            "resumes at the day after the cursor rather than restarting"
        );
    }

    #[test]
    fn cursor_older_than_the_retention_window_restarts_the_backfill() {
        let newest = date(2026, 8, 8);
        let oldest = newest - time::Duration::days(GSC_BACKFILL_DAYS);
        assert_eq!(
            gsc_sync_start_date(newest, Some(oldest - time::Duration::days(1))),
            oldest
        );
    }

    #[test]
    fn requests_final_data_only_for_settled_days() {
        let today = date(2026, 8, 11);
        assert_eq!(
            gsc_data_state(today, today - time::Duration::days(3)),
            GSC_DATA_STATE_ALL
        );
        assert_eq!(
            gsc_data_state(today, today - time::Duration::days(GSC_FINAL_DATA_DAYS)),
            GSC_DATA_STATE_ALL
        );
        assert_eq!(
            gsc_data_state(today, today - time::Duration::days(GSC_FINAL_DATA_DAYS + 1)),
            GSC_DATA_STATE_FINAL
        );
    }

    #[test]
    fn honors_retry_after_seconds_and_ignores_http_dates() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, "42".parse().expect("header"));
        assert_eq!(retry_after_delay(&headers), Some(Duration::from_secs(42)));

        headers.insert(RETRY_AFTER, "86400".parse().expect("header"));
        assert_eq!(
            retry_after_delay(&headers),
            Some(GSC_REQUEST_MAX_BACKOFF),
            "a long quota window is capped so it can't stall the sync scheduler"
        );

        headers.insert(
            RETRY_AFTER,
            "Wed, 21 Oct 2026 07:28:00 GMT".parse().expect("header"),
        );
        assert_eq!(retry_after_delay(&headers), None);
        assert_eq!(retry_after_delay(&HeaderMap::new()), None);
    }

    #[test]
    fn backs_off_exponentially_within_the_cap() {
        assert_eq!(retry_backoff(1), GSC_REQUEST_BASE_BACKOFF);
        assert_eq!(retry_backoff(2), GSC_REQUEST_BASE_BACKOFF * 2);
        assert_eq!(retry_backoff(3), GSC_REQUEST_BASE_BACKOFF * 4);
        assert_eq!(retry_backoff(usize::MAX), GSC_REQUEST_MAX_BACKOFF);
    }

    #[test]
    fn retries_rate_limits_but_not_permission_errors() {
        assert!(is_retryable_response(StatusCode::TOO_MANY_REQUESTS, ""));
        assert!(is_retryable_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "backend error"
        ));
        assert!(is_retryable_response(
            StatusCode::FORBIDDEN,
            r#"{"error":{"errors":[{"reason":"userRateLimitExceeded"}]}}"#
        ));
        assert!(!is_retryable_response(
            StatusCode::FORBIDDEN,
            r#"{"error":{"errors":[{"reason":"forbidden"}]}}"#
        ));
        assert!(!is_retryable_response(StatusCode::NOT_FOUND, ""));
    }

    #[test]
    fn inserts_a_days_rows_on_first_write() {
        let conn = open_migrated_db();
        let day = date(2026, 8, 1);
        let counts = write_day(
            &conn,
            day,
            snapshot(
                vec![("shoes", aggregate(100, 10, 4.5))],
                vec![(("shoes", "acme.test", "/shoes"), 10)],
            ),
        );

        assert_eq!(counts.records_inserted, 2);
        assert_eq!(counts.records_updated, 0);
        assert_eq!(counts.records_deleted, 0);

        let (impressions, clicks, position) = conn
            .query_row(
                "SELECT impressions, clicks, position FROM search_queries_by_date WHERE date = ?1",
                params![date_to_unix_utc_midnight(day)],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, f64>(2)?,
                    ))
                },
            )
            .expect("stored row");
        assert_eq!((impressions, clicks), (100, 10));
        assert!((position - 4.5).abs() < GSC_POSITION_EPSILON);
    }

    #[test]
    fn rewriting_identical_data_touches_nothing() {
        let conn = open_migrated_db();
        let day = date(2026, 8, 1);
        let build = || {
            snapshot(
                vec![("shoes", aggregate(100, 10, 4.5))],
                vec![(("shoes", "acme.test", "/shoes"), 10)],
            )
        };
        write_day(&conn, day, build());

        let counts = write_day(&conn, day, build());
        assert_eq!(counts.records_inserted, 0);
        assert_eq!(counts.records_updated, 0);
        assert_eq!(counts.records_deleted, 0);
    }

    #[test]
    fn updates_rows_whose_metrics_moved() {
        let conn = open_migrated_db();
        let day = date(2026, 8, 1);
        write_day(
            &conn,
            day,
            snapshot(
                vec![("shoes", aggregate(100, 10, 4.5))],
                vec![(("shoes", "acme.test", "/shoes"), 10)],
            ),
        );

        let counts = write_day(
            &conn,
            day,
            snapshot(
                vec![("shoes", aggregate(120, 11, 4.5))],
                vec![(("shoes", "acme.test", "/shoes"), 11)],
            ),
        );
        assert_eq!(counts.records_updated, 2);
        assert_eq!(counts.records_inserted, 0);
        assert_eq!(counts.records_deleted, 0);

        let impressions = conn
            .query_row(
                "SELECT impressions FROM search_queries_by_date WHERE date = ?1",
                params![date_to_unix_utc_midnight(day)],
                |row| row.get::<_, i64>(0),
            )
            .expect("stored row");
        assert_eq!(impressions, 120);
    }

    #[test]
    fn deletes_rows_that_left_the_report() {
        let conn = open_migrated_db();
        let day = date(2026, 8, 1);
        write_day(
            &conn,
            day,
            snapshot(
                vec![
                    ("shoes", aggregate(100, 10, 4.5)),
                    ("boots", aggregate(50, 5, 8.0)),
                ],
                vec![
                    (("shoes", "acme.test", "/shoes"), 10),
                    (("boots", "acme.test", "/boots"), 5),
                ],
            ),
        );

        let counts = write_day(
            &conn,
            day,
            snapshot(
                vec![("shoes", aggregate(100, 10, 4.5))],
                vec![(("shoes", "acme.test", "/shoes"), 10)],
            ),
        );
        assert_eq!(counts.records_deleted, 2);
        assert_eq!(counts.records_updated, 0);
        assert_eq!(counts.records_inserted, 0);

        let remaining = conn
            .query_row(
                "SELECT COUNT(*) FROM search_queries_by_date WHERE date = ?1",
                params![date_to_unix_utc_midnight(day)],
                |row| row.get::<_, i64>(0),
            )
            .expect("count");
        assert_eq!(remaining, 1);
    }

    #[test]
    fn writing_one_day_leaves_other_days_alone() {
        let conn = open_migrated_db();
        let first = date(2026, 8, 1);
        let second = date(2026, 8, 2);
        write_day(
            &conn,
            first,
            snapshot(
                vec![("shoes", aggregate(100, 10, 4.5))],
                vec![(("shoes", "acme.test", "/shoes"), 10)],
            ),
        );
        write_day(
            &conn,
            second,
            snapshot(vec![("boots", aggregate(50, 5, 8.0))], vec![]),
        );

        let days = conn
            .query_row(
                "SELECT COUNT(DISTINCT date) FROM search_queries_by_date",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("count");
        assert_eq!(days, 2);

        let first_clicks = conn
            .query_row(
                "SELECT clicks FROM search_queries_by_date WHERE date = ?1",
                params![date_to_unix_utc_midnight(first)],
                |row| row.get::<_, i64>(0),
            )
            .expect("stored row");
        assert_eq!(first_clicks, 10);
    }
}
