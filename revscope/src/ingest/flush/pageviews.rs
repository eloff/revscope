//! Pageview and landing rows, and the engagement folded into them.

use compact_str::CompactString;
use fxhash::FxHashMap;
use rusqlite::OptionalExtension;
use rusqlite::params;

use crate::db::ProjectConnection;
use crate::ingest::error::PersistError;
use crate::ingest::rows::{LandingContextRow, LandingPageView, PageView, ScreenDimensions};
use crate::referrer::ReferrerType;
use crate::untrusted_keys::UntrustedKeyMap;

use super::lookups::resolve_cached_path_id;
use super::sessions::{resolve_current_session, upsert_user_session};

/// Engagement contributions aggregated by their target pageview, built at flush
/// time so we can fold them into the pageview insert (when the pageview is in
/// the same batch) or apply them as a single update (when it is already in the
/// database).
pub(super) struct EngagementAccum {
    pub(super) duration: i64,
    pub(super) scrolled: Option<i64>,
}

/// Resolve the previous pageview timestamp id for `visitor`, preferring rows
/// already inserted in this flush over the database.
pub(super) fn pageview_previous(
    conn: &ProjectConnection,
    last_pageview_by_visitor: &mut FxHashMap<i64, i64>,
    visitor: i64,
) -> Result<Option<i64>, PersistError> {
    if let Some(&timestamp_id) = last_pageview_by_visitor.get(&visitor) {
        return Ok(Some(timestamp_id));
    }
    resolve_previous_timestamp_id(conn, visitor).map_err(PersistError::from)
}

/// Insert one queued pageview into `pageviews` and related session bookkeeping.
/// Returns the folded duration written to the row.
pub(super) fn flush_pageview(
    conn: &ProjectConnection,
    page: &PageView,
    path_ids: &mut UntrustedKeyMap<(CompactString, CompactString), i64>,
    last_pageview_by_visitor: &mut FxHashMap<i64, i64>,
    engagement_by_page: &mut FxHashMap<i64, EngagementAccum>,
) -> Result<Option<i64>, PersistError> {
    let path_id = resolve_cached_path_id(conn, path_ids, &page.path, &page.hostname)?;
    let previous_timestamp_id =
        pageview_previous(conn, last_pageview_by_visitor, page.visitor_timestamp_id)?;

    let folded = engagement_by_page.remove(&page.timestamp_id);
    let duration_seconds = combine_duration(page.duration_seconds, folded.as_ref());
    let scrolled_percent = combine_scroll(page.scrolled_percent, folded.as_ref());

    insert_pageview_row(
        conn,
        page,
        path_id,
        previous_timestamp_id,
        duration_seconds,
        scrolled_percent,
    )?;

    conn.execute(
        "INSERT INTO client_page_ids_map (visitor_timestamp_id, page_id)
         VALUES (?1, ?2)",
        params![page.timestamp_id, page.client_page_id],
    )?;

    upsert_user_session(
        conn,
        &page.user_hash.to_be_bytes(),
        page.visitor_timestamp_id,
        page.expires_after_days,
    )?;

    last_pageview_by_visitor.insert(page.visitor_timestamp_id, page.timestamp_id);
    Ok(duration_seconds)
}

pub(super) fn insert_pageview_row(
    conn: &ProjectConnection,
    page: &PageView,
    path_id: i64,
    previous_timestamp_id: Option<i64>,
    duration_seconds: Option<i64>,
    scrolled_percent: Option<i64>,
) -> Result<(), PersistError> {
    conn.execute(
        "INSERT INTO pageviews (
            timestamp_id, visitor_timestamp_id, previous_timestamp_id, path_id,
            duration_seconds, scrolled_percent
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            page.timestamp_id,
            page.visitor_timestamp_id,
            previous_timestamp_id,
            path_id,
            duration_seconds,
            scrolled_percent,
        ],
    )?;
    Ok(())
}

pub(super) fn insert_landing_row(
    conn: &ProjectConnection,
    landing: &LandingPageView,
    referrer_id: Option<i64>,
    campaign_id: Option<i64>,
    session_duration_seconds: i64,
) -> Result<(), PersistError> {
    let (screen_width, screen_height, viewport_width, viewport_height) =
        screen_columns(landing.screen_dimensions);
    conn.execute(
        "INSERT INTO landings (
                visitor_timestamp_id, session_duration_seconds, session_page_count,
                consent, referrer, campaign, click_id, referrer_type, is_google,
                timezone, language, country, region, city, tracker_version, os, os_version,
                browser_family, device,
                screen_width, screen_height, viewport_width, viewport_height
            ) VALUES (?1, ?2, 1, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22)
            ON CONFLICT(visitor_timestamp_id) DO UPDATE SET
                session_duration_seconds = excluded.session_duration_seconds,
                session_page_count = excluded.session_page_count,
                consent = excluded.consent,
                referrer = excluded.referrer,
                campaign = excluded.campaign,
                click_id = excluded.click_id,
                referrer_type = excluded.referrer_type,
                is_google = excluded.is_google,
                timezone = excluded.timezone,
                language = excluded.language,
                country = excluded.country,
                region = excluded.region,
                city = excluded.city,
                tracker_version = excluded.tracker_version,
                os = excluded.os,
                os_version = excluded.os_version,
                browser_family = excluded.browser_family,
                device = excluded.device",
        params![
            landing.timestamp_id,
            session_duration_seconds,
            landing.consent.map(i64::from),
            referrer_id,
            campaign_id,
            landing.landing_click_id,
            landing.referrer_type.map(ReferrerType::as_str),
            i64::from(landing.is_google),
            landing.timezone.as_deref(),
            landing.language.as_deref(),
            landing.country.as_deref(),
            landing.region.as_deref(),
            landing.city.as_deref(),
            landing.tracker_version,
            landing.ua_context.os.as_str(),
            landing.ua_context.os_version,
            landing.ua_context.browser_family.as_str(),
            landing.ua_context.device.as_str(),
            screen_width,
            screen_height,
            viewport_width,
            viewport_height,
        ],
    )?;
    Ok(())
}

/// Fold the follow-up device context into the session's landing row. Every field
/// is optional: a visitor who blocks `Intl` or has no `screen` still keeps the
/// values already derived from the request headers.
pub(super) fn apply_landing_context_update(
    conn: &ProjectConnection,
    context: &LandingContextRow,
) -> Result<(), PersistError> {
    let dimensions = context.screen_dimensions;
    conn.execute(
        "UPDATE landings
         SET timezone = COALESCE(?2, timezone),
             language = COALESCE(?3, language),
             screen_width = COALESCE(?4, screen_width),
             screen_height = COALESCE(?5, screen_height),
             viewport_width = COALESCE(?6, viewport_width),
             viewport_height = COALESCE(?7, viewport_height),
             device = CASE WHEN ?8 = 1 AND device = 'desktop' THEN 'desktop_touch' ELSE device END
         WHERE visitor_timestamp_id = ?1",
        params![
            context.visitor_timestamp_id,
            context.timezone.as_deref(),
            context.language.as_deref(),
            dimensions.map(|dimensions| dimensions.screen_width),
            dimensions.map(|dimensions| dimensions.screen_height),
            dimensions.map(|dimensions| dimensions.viewport_width),
            dimensions.map(|dimensions| dimensions.viewport_height),
            i64::from(context.touch),
        ],
    )?;
    Ok(())
}

pub(super) fn apply_engagement_update(
    conn: &ProjectConnection,
    page_timestamp_id: i64,
    accum: &EngagementAccum,
) -> Result<(), PersistError> {
    conn.execute(
        "UPDATE pageviews
         SET duration_seconds = COALESCE(duration_seconds, 0) + ?2,
             scrolled_percent = CASE
                WHEN ?3 IS NULL THEN scrolled_percent
                WHEN scrolled_percent IS NULL THEN ?3
                ELSE MAX(scrolled_percent, ?3)
             END
         WHERE timestamp_id = ?1",
        params![page_timestamp_id, accum.duration, accum.scrolled],
    )?;

    if accum.duration > 0
        && let Some(visitor_timestamp_id) = resolve_current_session(conn, page_timestamp_id)?
    {
        increment_landing_session_metrics(conn, visitor_timestamp_id, 0, accum.duration)?;
    }
    Ok(())
}

pub(super) fn combine_duration(base: Option<i64>, folded: Option<&EngagementAccum>) -> Option<i64> {
    match folded {
        Some(accum) => Some(base.unwrap_or(0) + accum.duration),
        None => base,
    }
}

pub(super) fn combine_scroll(base: Option<i64>, folded: Option<&EngagementAccum>) -> Option<i64> {
    max_option(base, folded.and_then(|accum| accum.scrolled))
}

pub(super) fn max_option(left: Option<i64>, right: Option<i64>) -> Option<i64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

pub(super) fn screen_columns(dimensions: Option<ScreenDimensions>) -> (i64, i64, i64, i64) {
    match dimensions {
        Some(dimensions) => (
            dimensions.screen_width,
            dimensions.screen_height,
            dimensions.viewport_width,
            dimensions.viewport_height,
        ),
        None => (0, 0, 0, 0),
    }
}
pub(super) fn increment_landing_session_metrics(
    conn: &ProjectConnection,
    visitor_timestamp_id: i64,
    page_count_delta: i64,
    duration_delta_seconds: i64,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE landings
         SET session_page_count = session_page_count + ?2,
             session_duration_seconds = session_duration_seconds + ?3
         WHERE visitor_timestamp_id = ?1",
        params![
            visitor_timestamp_id,
            page_count_delta,
            duration_delta_seconds
        ],
    )?;
    Ok(())
}

pub(super) fn resolve_previous_timestamp_id(
    conn: &ProjectConnection,
    visitor_timestamp_id: i64,
) -> Result<Option<i64>, rusqlite::Error> {
    conn.query_row(
        "SELECT timestamp_id FROM pageviews
             WHERE visitor_timestamp_id = ?1
             ORDER BY timestamp_id DESC
             LIMIT 1",
        params![visitor_timestamp_id],
        |row| row.get::<_, i64>(0),
    )
    .optional()
}
