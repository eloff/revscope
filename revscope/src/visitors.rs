use std::sync::Arc;

use rusqlite::types::ToSql;
use rusqlite::{OptionalExtension, params, params_from_iter};
use serde::Serialize;

use crate::auth::{AuthError, map_row_error};
use crate::authorize::AuthorizedProject;
use crate::consent;
use crate::db::{ProjectConnection, open_project_database_read};
use crate::payment_journey::{
    PaymentJourneyAction, fetch_event_actions, fetch_pageview_actions, sort_journey_timeline,
    source_action_from_parts,
};
use crate::payments::{PaymentCampaign, PaymentReferrer, campaign_display_label};
use crate::query_params::{SortDirection, SortedPage, parse_positive_id, parse_report_query};
use crate::referrer::ReferrerType;
use crate::segment_filter::{SegmentFilters, parse_segment_filters};
use crate::server_context::ServerContext;
use crate::util::{NANOS_PER_SECOND, encode_visitor_timestamp_id, parse_visitor_timestamp_id_hex};

pub(crate) const VISITORS_PAGE_SIZE: i64 = 100;

#[derive(Debug, Serialize)]
pub(crate) struct VisitorsResponse {
    pub(crate) visitors: Vec<VisitorEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) next_offset: Option<i64>,
    pub(crate) offset: i64,
    pub(crate) limit: i64,
    /// Human-readable description of the active session filter, resolved so the
    /// dashboard can label and clear it without re-deriving names from ids.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) filter: Option<VisitorsFilterInfo>,
}

#[derive(Debug, Serialize)]
pub(crate) struct VisitorsFilterInfo {
    /// One of `pageview`, `landing_page`, or `event`.
    pub(crate) kind: &'static str,
    /// Request path the filter targets.
    pub(crate) path: String,
    /// Hostname the targeted path was served on.
    pub(crate) hostname: String,
    /// Event name; present only for the `event` filter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) event_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct VisitorEntry {
    #[serde(serialize_with = "serialize_visitor_timestamp_id_as_hex")]
    pub(crate) visitor_timestamp_id: i64,
    pub(crate) viewed_at: i64,
    pub(crate) last_view_at: i64,
    pub(crate) session_duration_seconds: i64,
    pub(crate) session_page_count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) click_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) referrer_type: Option<ReferrerType>,
    pub(crate) is_google: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) timezone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) country: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) city: Option<String>,
    pub(crate) tracker_version: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) os: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) os_version: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) browser_family: Option<String>,
    pub(crate) device: String,
    pub(crate) screen_width: i64,
    pub(crate) screen_height: i64,
    pub(crate) viewport_width: i64,
    pub(crate) viewport_height: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) referrer: Option<PaymentReferrer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) campaign: Option<PaymentCampaign>,
}

#[derive(Debug, Serialize)]
pub(crate) struct VisitorJourneyResponse {
    pub(crate) visitor: VisitorEntry,
    pub(crate) timeline: Vec<PaymentJourneyAction>,
}

pub(crate) struct VisitorsQuery {
    pub(crate) start: i64,
    pub(crate) end: i64,
    pub(crate) offset: i64,
    pub(crate) order_by: VisitorsOrderBy,
    pub(crate) direction: SortDirection,
    pub(crate) filter: VisitorsSessionFilter,
    pub(crate) segments: SegmentFilters,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VisitorsOrderBy {
    Source,
    LastViewAt,
    ActiveDuration,
    SessionAge,
    PagesViewed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VisitorsSessionFilter {
    All,
    Pageview { path_id: i64 },
    LandingPage { path_id: i64 },
    Event { event_name_id: i64, path_id: i64 },
}

pub(crate) struct VisitorJourneyQuery {
    pub(crate) visitor_timestamp_id: i64,
}

pub(crate) fn parse_visitors_query(target: &str) -> Result<VisitorsQuery, &'static str> {
    let mut sort = SortedPage::new(VisitorsOrderBy::LastViewAt, parse_order_by);
    let mut filter = None;
    let mut path_id = None;
    let mut event_name_id = None;
    let range = parse_report_query(target, |key, value| {
        if sort.accept(key, value)? {
            return Ok(());
        }
        match key {
            "filter" | "visitor_filter" => filter = Some(value),
            "path_id" => path_id = Some(parse_positive_id(value, "invalid path_id")?),
            "event_name_id" => {
                event_name_id = Some(parse_positive_id(value, "invalid event_name_id")?);
            }
            _ => {}
        }
        Ok(())
    })?;
    let (start, end) = range.validate()?;
    let filter = parse_session_filter(filter, path_id, event_name_id)?;
    Ok(VisitorsQuery {
        start,
        end,
        offset: sort.offset,
        order_by: sort.order_by,
        direction: sort.direction,
        filter,
        segments: parse_segment_filters(target),
    })
}

pub(crate) fn parse_visitor_journey_query(
    target: &str,
) -> Result<VisitorJourneyQuery, &'static str> {
    let value = crate::http::query_param(target, "visitor_timestamp_id")
        .ok_or("visitor_timestamp_id is required")?;
    Ok(VisitorJourneyQuery {
        visitor_timestamp_id: parse_visitor_timestamp_id_hex(value)?,
    })
}

pub(crate) fn visitors(
    server_context: Arc<ServerContext>,
    project: &AuthorizedProject,
    query: VisitorsQuery,
) -> Result<VisitorsResponse, AuthError> {
    fetch_visitors(server_context, project.id(), &query)
}

pub(crate) fn visitor_journey(
    server_context: Arc<ServerContext>,
    project: &AuthorizedProject,
    query: VisitorJourneyQuery,
) -> Result<VisitorJourneyResponse, AuthError> {
    fetch_visitor_journey(server_context, project.id(), &query)
}

fn fetch_visitors(
    server_context: Arc<ServerContext>,
    project_id: &str,
    query: &VisitorsQuery,
) -> Result<VisitorsResponse, AuthError> {
    let conn = open_project_database_read(server_context.as_ref(), project_id)
        .map_err(|error| AuthError::Db(error.to_string()))?;
    build_visitors_response(&conn, query)
}

fn fetch_visitor_journey(
    server_context: Arc<ServerContext>,
    project_id: &str,
    query: &VisitorJourneyQuery,
) -> Result<VisitorJourneyResponse, AuthError> {
    let conn = open_project_database_read(server_context.as_ref(), project_id)
        .map_err(|error| AuthError::Db(error.to_string()))?;
    build_visitor_journey_response(&conn, query.visitor_timestamp_id)
}

pub(crate) fn build_visitors_response(
    conn: &ProjectConnection,
    query: &VisitorsQuery,
) -> Result<VisitorsResponse, AuthError> {
    let mut entries = fetch_visitor_entries(conn, query)?;
    let has_more = entries.len() as i64 > VISITORS_PAGE_SIZE;
    if has_more {
        entries.truncate(VISITORS_PAGE_SIZE as usize);
    }
    let next_offset = if has_more {
        Some(query.offset + VISITORS_PAGE_SIZE)
    } else {
        None
    };
    let filter = fetch_filter_info(conn, query.filter)?;
    Ok(VisitorsResponse {
        visitors: entries,
        next_offset,
        offset: query.offset,
        limit: VISITORS_PAGE_SIZE,
        filter,
    })
}

/// Resolves the active session filter into the path/event names the dashboard
/// shows in its filter chip. Returns `None` for the unfiltered listing or when
/// the targeted path no longer exists.
fn fetch_filter_info(
    conn: &ProjectConnection,
    filter: VisitorsSessionFilter,
) -> Result<Option<VisitorsFilterInfo>, AuthError> {
    let (kind, path_id) = match filter {
        VisitorsSessionFilter::All => return Ok(None),
        VisitorsSessionFilter::Pageview { path_id } => ("pageview", path_id),
        VisitorsSessionFilter::LandingPage { path_id } => ("landing_page", path_id),
        VisitorsSessionFilter::Event { path_id, .. } => ("event", path_id),
    };
    let path_row = conn
        .query_row(
            "SELECT path, hostname FROM paths WHERE id = ?1",
            params![path_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(map_row_error)?;
    let Some((path, hostname)) = path_row else {
        return Ok(None);
    };
    let event_name = match filter {
        VisitorsSessionFilter::Event { event_name_id, .. } => conn
            .query_row(
                "SELECT name FROM event_names WHERE id = ?1",
                params![event_name_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(map_row_error)?,
        _ => None,
    };
    Ok(Some(VisitorsFilterInfo {
        kind,
        path,
        hostname,
        event_name,
    }))
}

pub(crate) fn build_visitor_journey_response(
    conn: &ProjectConnection,
    visitor_timestamp_id: i64,
) -> Result<VisitorJourneyResponse, AuthError> {
    let visitor = fetch_visitor_entry_by_id(conn, visitor_timestamp_id)?
        .ok_or(AuthError::InvalidRequest("visitor not found"))?;
    let mut timeline = Vec::new();
    timeline.push(source_action_from_parts(
        visitor.viewed_at,
        visitor.campaign.as_ref().map(campaign_display_label),
        visitor
            .referrer
            .as_ref()
            .map(|referrer| referrer.referrer.clone()),
        visitor.referrer_type,
    ));
    timeline.extend(fetch_pageview_actions(conn, visitor.visitor_timestamp_id)?);
    timeline.extend(fetch_event_actions(conn, visitor.visitor_timestamp_id)?);
    sort_journey_timeline(&mut timeline);
    Ok(VisitorJourneyResponse { visitor, timeline })
}

fn fetch_visitor_entries(
    conn: &ProjectConnection,
    query: &VisitorsQuery,
) -> Result<Vec<VisitorEntry>, AuthError> {
    let window_start_nanos = query.start * NANOS_PER_SECOND;
    let window_end_nanos = query.end * NANOS_PER_SECOND;
    let order_clause = visitors_order_clause(query.order_by, query.direction);
    let path_id = filter_path_id(query.filter).unwrap_or_default();
    let event_name_id = filter_event_name_id(query.filter).unwrap_or_default();
    // The fixed binds occupy ?1..?7, so country placeholders begin at ?8.
    let country_filter = query.segments.country_in_clause("l.country", 8);
    let session_filter = query
        .segments
        .session_filter_clause("l.visitor_timestamp_id", None);
    let consent_filter = consent::consent_visibility_clause("l.consent", "l.country", "l.region");
    let inner_where = format!(
        "{}{country_filter}{session_filter}{consent_filter}",
        visitors_filter_where(query.filter)
    );
    let page_limit = VISITORS_PAGE_SIZE + 1;
    let nanos = NANOS_PER_SECOND;
    let mut sql_params: Vec<&dyn ToSql> = vec![
        &window_start_nanos,
        &window_end_nanos,
        &page_limit,
        &query.offset,
        &nanos,
        &path_id,
        &event_name_id,
    ];
    query.segments.append_country_params(&mut sql_params);
    conn.with_query(
        visitor_entries_sql(
            &inner_where,
            &format!("ORDER BY {order_clause} LIMIT ?3 OFFSET ?4"),
        )
        .as_str(),
        params_from_iter(sql_params),
        |mut rows| {
            let mut out = Vec::with_capacity((VISITORS_PAGE_SIZE + 1) as usize);
            while let Some(row) = rows.next()? {
                out.push(visitor_entry_from_row(row)?);
            }
            Ok(out)
        },
    )
    .map_err(map_row_error)
}

fn visitors_filter_where(filter: VisitorsSessionFilter) -> &'static str {
    match filter {
        VisitorsSessionFilter::All => {
            "l.visitor_timestamp_id >= ?1
             AND l.visitor_timestamp_id < ?2
             AND ?6 = ?6
             AND ?7 = ?7"
        }
        VisitorsSessionFilter::Pageview { .. } => {
            "EXISTS (
                SELECT 1
                FROM pageviews filter_pv
                JOIN paths filter_path ON filter_path.id = filter_pv.path_id
                WHERE filter_pv.visitor_timestamp_id = l.visitor_timestamp_id
                  AND filter_pv.timestamp_id >= ?1
                  AND filter_pv.timestamp_id < ?2
                  AND filter_pv.path_id = ?6
                  AND filter_path.deleted_at IS NULL
             )
             AND ?7 = ?7"
        }
        VisitorsSessionFilter::LandingPage { .. } => {
            "l.visitor_timestamp_id >= ?1
             AND l.visitor_timestamp_id < ?2
             AND EXISTS (
                SELECT 1
                FROM pageviews filter_pv
                JOIN paths filter_path ON filter_path.id = filter_pv.path_id
                WHERE filter_pv.timestamp_id = l.visitor_timestamp_id
                  AND filter_pv.path_id = ?6
                  AND filter_path.deleted_at IS NULL
             )
             AND ?7 = ?7"
        }
        VisitorsSessionFilter::Event { .. } => {
            "EXISTS (
                SELECT 1
                FROM events filter_event
                JOIN event_names filter_event_name
                  ON filter_event_name.id = filter_event.event_name_id
                JOIN pageviews filter_pv ON filter_pv.timestamp_id = filter_event.page_timestamp_id
                JOIN paths filter_path ON filter_path.id = filter_pv.path_id
                WHERE filter_pv.visitor_timestamp_id = l.visitor_timestamp_id
                  AND filter_event.timestamp_id >= ?1
                  AND filter_event.timestamp_id < ?2
                  AND filter_event.event_name_id = ?7
                  AND filter_pv.path_id = ?6
                  AND filter_event_name.deleted_at IS NULL
                  AND filter_path.deleted_at IS NULL
             )"
        }
    }
}

fn filter_path_id(filter: VisitorsSessionFilter) -> Option<i64> {
    match filter {
        VisitorsSessionFilter::Pageview { path_id }
        | VisitorsSessionFilter::LandingPage { path_id }
        | VisitorsSessionFilter::Event { path_id, .. } => Some(path_id),
        VisitorsSessionFilter::All => None,
    }
}

fn filter_event_name_id(filter: VisitorsSessionFilter) -> Option<i64> {
    match filter {
        VisitorsSessionFilter::Event { event_name_id, .. } => Some(event_name_id),
        VisitorsSessionFilter::All
        | VisitorsSessionFilter::Pageview { .. }
        | VisitorsSessionFilter::LandingPage { .. } => None,
    }
}

fn fetch_visitor_entry_by_id(
    conn: &ProjectConnection,
    visitor_timestamp_id: i64,
) -> Result<Option<VisitorEntry>, AuthError> {
    let consent_filter = consent::consent_visibility_clause("l.consent", "l.country", "l.region");
    conn.with_query(
        visitor_entries_sql(
            &format!("l.visitor_timestamp_id = ?1{consent_filter}"),
            "ORDER BY last_view_at DESC, visitor_timestamp_id ASC
             LIMIT 1",
        )
        .as_str(),
        params![visitor_timestamp_id, 0_i64, 0_i64, 0_i64, NANOS_PER_SECOND],
        |mut rows| rows.next()?.map(visitor_entry_from_row).transpose(),
    )
    .map_err(map_row_error)
}

fn visitor_entries_sql(inner_where: &str, outer_clause: &str) -> String {
    format!(
        "WITH visitor_rows AS (
            SELECT
                l.visitor_timestamp_id,
                l.visitor_timestamp_id / ?5 AS viewed_at,
                COALESCE(
                    MAX(pv.timestamp_id + COALESCE(pv.duration_seconds, 0) * ?5),
                    l.visitor_timestamp_id
                ) / ?5 AS last_view_at,
                l.session_duration_seconds,
                l.session_page_count,
                l.click_id,
                l.referrer_type,
                l.is_google,
                l.timezone,
                l.language,
                l.country,
                l.city,
                l.tracker_version,
                l.os,
                l.os_version,
                l.browser_family,
                l.device,
                l.screen_width,
                l.screen_height,
                l.viewport_width,
                l.viewport_height,
                r.id,
                r.referrer,
                r.deleted_at,
                c.id,
                c.source,
                c.medium,
                c.campaign,
                c.network,
                c.display_name,
                c.first_seen_at,
                c.deleted_at,
                lower(
                    CASE
                        WHEN c.id IS NOT NULL THEN COALESCE(
                            NULLIF(c.display_name, ''),
                            COALESCE(c.network, '') || ' ' || c.source || ' ' || c.medium || ' ' || c.campaign
                        )
                        WHEN r.id IS NOT NULL THEN r.referrer
                        ELSE 'Direct'
                    END
                ) AS source_sort_key
             FROM landings l
             LEFT JOIN pageviews pv ON pv.visitor_timestamp_id = l.visitor_timestamp_id
             LEFT JOIN referrers r ON r.id = l.referrer
             LEFT JOIN campaigns c ON c.id = l.campaign
             WHERE {inner_where}
             GROUP BY l.visitor_timestamp_id
        )
        SELECT * FROM visitor_rows
        {outer_clause}"
    )
}

fn visitors_order_clause(order_by: VisitorsOrderBy, direction: SortDirection) -> String {
    let direction = direction.sql();
    let primary = match order_by {
        VisitorsOrderBy::Source => "source_sort_key",
        VisitorsOrderBy::LastViewAt => "last_view_at",
        VisitorsOrderBy::ActiveDuration => "session_duration_seconds",
        VisitorsOrderBy::SessionAge => "(last_view_at - viewed_at)",
        VisitorsOrderBy::PagesViewed => "session_page_count",
    };
    if matches!(order_by, VisitorsOrderBy::LastViewAt) {
        return format!("{primary} {direction}, visitor_timestamp_id ASC");
    }
    format!("{primary} {direction}, last_view_at DESC, visitor_timestamp_id ASC")
}

fn visitor_entry_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<VisitorEntry> {
    let referrer_type = row
        .get::<_, Option<String>>(6)?
        .and_then(|value| ReferrerType::from_stored(&value));
    Ok(VisitorEntry {
        visitor_timestamp_id: row.get(0)?,
        viewed_at: row.get(1)?,
        last_view_at: row.get(2)?,
        session_duration_seconds: row.get(3)?,
        session_page_count: row.get(4)?,
        click_id: row.get(5)?,
        referrer_type,
        is_google: row.get::<_, i64>(7)? != 0,
        timezone: row.get(8)?,
        language: row.get(9)?,
        country: row.get(10)?,
        city: row.get(11)?,
        tracker_version: row.get(12)?,
        os: row.get(13)?,
        os_version: row.get(14)?,
        browser_family: row.get(15)?,
        device: row.get(16)?,
        screen_width: row.get(17)?,
        screen_height: row.get(18)?,
        viewport_width: row.get(19)?,
        viewport_height: row.get(20)?,
        referrer: build_referrer(row)?,
        campaign: build_campaign(row)?,
    })
}

fn build_referrer(row: &rusqlite::Row<'_>) -> rusqlite::Result<Option<PaymentReferrer>> {
    let id = row.get::<_, Option<i64>>(21)?;
    Ok(id.map(|id| PaymentReferrer {
        id,
        referrer: row.get(22).unwrap_or_default(),
        deleted_at: row.get(23).unwrap_or(None),
    }))
}

fn build_campaign(row: &rusqlite::Row<'_>) -> rusqlite::Result<Option<PaymentCampaign>> {
    let id = row.get::<_, Option<i64>>(24)?;
    Ok(id.map(|id| PaymentCampaign {
        id,
        source: row.get(25).unwrap_or_default(),
        medium: row.get(26).unwrap_or_default(),
        campaign: row.get(27).unwrap_or_default(),
        network: row.get(28).unwrap_or(None),
        display_name: row.get(29).unwrap_or(None),
        first_seen_at: row.get(30).unwrap_or_default(),
        deleted_at: row.get(31).unwrap_or(None),
    }))
}

fn parse_order_by(value: &str) -> Result<VisitorsOrderBy, &'static str> {
    match value {
        "source" => Ok(VisitorsOrderBy::Source),
        "last_view_at" => Ok(VisitorsOrderBy::LastViewAt),
        "active_duration" => Ok(VisitorsOrderBy::ActiveDuration),
        "session_age" => Ok(VisitorsOrderBy::SessionAge),
        "pages_viewed" => Ok(VisitorsOrderBy::PagesViewed),
        _ => Err("invalid order_by"),
    }
}

fn parse_session_filter(
    filter: Option<&str>,
    path_id: Option<i64>,
    event_name_id: Option<i64>,
) -> Result<VisitorsSessionFilter, &'static str> {
    match filter {
        None | Some("all") => Ok(VisitorsSessionFilter::All),
        Some("pageview") | Some("page_view") => Ok(VisitorsSessionFilter::Pageview {
            path_id: path_id.ok_or("path_id is required for pageview filter")?,
        }),
        Some("landing_page") | Some("landing") => Ok(VisitorsSessionFilter::LandingPage {
            path_id: path_id.ok_or("path_id is required for landing_page filter")?,
        }),
        Some("event") => Ok(VisitorsSessionFilter::Event {
            event_name_id: event_name_id.ok_or("event_name_id is required for event filter")?,
            path_id: path_id.ok_or("path_id is required for event filter")?,
        }),
        Some(_) => Err("invalid visitor filter"),
    }
}

fn serialize_visitor_timestamp_id_as_hex<S>(value: &i64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&encode_visitor_timestamp_id(*value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::open_migrated_db;

    use rusqlite::params;

    fn timestamp_id(seed: u8, viewed_at: i64) -> i64 {
        viewed_at * NANOS_PER_SECOND + i64::from(seed)
    }

    fn insert_path(conn: &ProjectConnection, path: &str) -> i64 {
        conn.execute(
            "INSERT INTO paths (path, hostname) VALUES (?1, 'example.com')",
            params![path],
        )
        .expect("insert path");
        conn.query_row(
            "SELECT id FROM paths WHERE path = ?1",
            params![path],
            |row| row.get::<_, i64>(0),
        )
        .expect("path id")
    }

    fn insert_referrer(conn: &ProjectConnection, referrer: &str) -> i64 {
        conn.execute(
            "INSERT INTO referrers (referrer, deleted_at) VALUES (?1, NULL)",
            params![referrer],
        )
        .expect("insert referrer");
        conn.query_row(
            "SELECT id FROM referrers WHERE referrer = ?1",
            params![referrer],
            |row| row.get::<_, i64>(0),
        )
        .expect("referrer id")
    }

    fn insert_campaign(conn: &ProjectConnection) -> i64 {
        conn.execute(
            "INSERT INTO campaigns (
                source, medium, campaign, network, display_name, first_seen_at, deleted_at
             ) VALUES ('newsletter', 'email', 'launch', NULL, 'Launch Email', 100, NULL)",
            [],
        )
        .expect("insert campaign");
        conn.query_row("SELECT id FROM campaigns", [], |row| row.get::<_, i64>(0))
            .expect("campaign id")
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_landing(
        conn: &ProjectConnection,
        seed: u8,
        viewed_at: i64,
        path_id: i64,
        referrer: Option<i64>,
        campaign: Option<i64>,
        session_duration_seconds: i64,
        session_page_count: i64,
    ) -> i64 {
        let visitor_timestamp_id = timestamp_id(seed, viewed_at);
        conn.execute(
            "INSERT INTO pageviews (
                timestamp_id, visitor_timestamp_id, previous_timestamp_id, path_id,
                duration_seconds, scrolled_percent
             ) VALUES (?1, ?1, NULL, ?2, 5, NULL)",
            params![visitor_timestamp_id, path_id],
        )
        .expect("insert landing pageview");
        conn.execute(
            "INSERT INTO landings (
                visitor_timestamp_id, session_duration_seconds, session_page_count,
                referrer, campaign, click_id, referrer_type, is_google, timezone,
                language, country, city, tracker_version, os, os_version, browser_family,
                device, screen_width, screen_height, viewport_width, viewport_height
             ) VALUES (
                ?1, ?2, ?3,
                ?4, ?5, NULL, 'search', ?6, 'UTC',
                'EN', 'US', 'Austin', 1.0, 'linux', 6.0, 'chrome',
                'desktop', 1920, 1080, 1440, 900
             )",
            params![
                visitor_timestamp_id,
                session_duration_seconds,
                session_page_count,
                referrer,
                campaign,
                if referrer.is_some() { 1 } else { 0 },
            ],
        )
        .expect("insert landing");
        visitor_timestamp_id
    }

    fn insert_pageview(
        conn: &ProjectConnection,
        visitor_timestamp_id: i64,
        seed: u8,
        viewed_at: i64,
        path_id: i64,
    ) {
        conn.execute(
            "INSERT INTO pageviews (
                timestamp_id, visitor_timestamp_id, previous_timestamp_id, path_id,
                duration_seconds, scrolled_percent
             ) VALUES (?1, ?2, ?2, ?3, 10, 50)",
            params![timestamp_id(seed, viewed_at), visitor_timestamp_id, path_id],
        )
        .expect("insert pageview");
    }

    fn insert_event_name(conn: &ProjectConnection, name: &str) -> i64 {
        conn.execute("INSERT INTO event_names (name) VALUES (?1)", params![name])
            .expect("insert event name");
        conn.query_row(
            "SELECT id FROM event_names WHERE name = ?1",
            params![name],
            |row| row.get::<_, i64>(0),
        )
        .expect("event name id")
    }

    fn insert_event(
        conn: &ProjectConnection,
        seed: u8,
        timestamp: i64,
        event_name_id: i64,
        page_timestamp_id: i64,
    ) {
        conn.execute(
            "INSERT INTO events (timestamp_id, event_name_id, page_timestamp_id, metadata)
             VALUES (?1, ?2, ?3, NULL)",
            params![
                timestamp_id(seed, timestamp),
                event_name_id,
                page_timestamp_id
            ],
        )
        .expect("insert event");
    }

    fn baseline_query(start: i64, end: i64) -> VisitorsQuery {
        VisitorsQuery {
            start,
            end,
            offset: 0,
            order_by: VisitorsOrderBy::LastViewAt,
            direction: SortDirection::Desc,
            filter: VisitorsSessionFilter::All,
            segments: SegmentFilters::default(),
        }
    }

    #[test]
    fn parses_visitor_journey_query_hex() {
        let visitor = timestamp_id(1, 100);
        let hex = encode_visitor_timestamp_id(visitor);
        let query = parse_visitor_journey_query(&format!(
            "/v2/read/projects/1/visitor-journey?visitor_timestamp_id={hex}"
        ))
        .expect("query");

        assert_eq!(query.visitor_timestamp_id, visitor);
    }

    #[test]
    fn rejects_decimal_visitor_journey_query() {
        assert!(
            parse_visitor_journey_query(
                "/v2/read/projects/1/visitor-journey?visitor_timestamp_id=1"
            )
            .is_err()
        );
    }

    #[test]
    fn serializes_visitor_timestamp_id_as_hex() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/");
        let visitor = insert_landing(&conn, 1, 100, home, None, None, 20, 2);
        let response = build_visitor_journey_response(&conn, visitor).expect("journey");
        let json = serde_json::to_string(&response.visitor).expect("json");

        assert!(json.contains(&encode_visitor_timestamp_id(visitor)));
        assert!(!json.contains(&format!("\"visitor_timestamp_id\":{visitor}")));
    }

    #[test]
    fn parses_offset_and_ordering() {
        let query = parse_visitors_query(
            "/v2/read/projects/1/visitors?start=1&end=100&offset=200&order_by=pages_viewed&direction=asc",
        )
        .expect("query");

        assert_eq!(query.offset, 200);
        assert_eq!(query.order_by, VisitorsOrderBy::PagesViewed);
        assert_eq!(query.direction, SortDirection::Asc);
        assert_eq!(query.filter, VisitorsSessionFilter::All);
    }

    #[test]
    fn parses_session_filters() {
        let pageview = parse_visitors_query(
            "/v2/read/projects/1/visitors?start=1&end=100&filter=pageview&path_id=42",
        )
        .expect("pageview query");
        assert_eq!(
            pageview.filter,
            VisitorsSessionFilter::Pageview { path_id: 42 }
        );

        let landing_page = parse_visitors_query(
            "/v2/read/projects/1/visitors?start=1&end=100&filter=landing_page&path_id=43",
        )
        .expect("landing query");
        assert_eq!(
            landing_page.filter,
            VisitorsSessionFilter::LandingPage { path_id: 43 }
        );

        let event = parse_visitors_query(
            "/v2/read/projects/1/visitors?start=1&end=100&filter=event&event_name_id=7&path_id=44",
        )
        .expect("event query");
        assert_eq!(
            event.filter,
            VisitorsSessionFilter::Event {
                event_name_id: 7,
                path_id: 44
            }
        );
    }

    #[test]
    fn rejects_invalid_offset_and_ordering() {
        assert!(
            parse_visitors_query("/v2/read/projects/1/visitors?start=1&end=100&offset=-1",)
                .is_err()
        );
        assert!(
            parse_visitors_query("/v2/read/projects/1/visitors?start=1&end=100&order_by=bad",)
                .is_err()
        );
        assert!(
            parse_visitors_query(
                "/v2/read/projects/1/visitors?start=1&end=100&direction=sideways",
            )
            .is_err()
        );
        assert!(
            parse_visitors_query("/v2/read/projects/1/visitors?start=1&end=100&filter=pageview",)
                .is_err()
        );
        assert!(
            parse_visitors_query(
                "/v2/read/projects/1/visitors?start=1&end=100&filter=event&path_id=1",
            )
            .is_err()
        );
    }

    #[test]
    fn returns_visitors_sorted_by_last_view_desc() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/");
        let pricing = insert_path(&conn, "/pricing");
        let google = insert_referrer(&conn, "google.com");
        let campaign = insert_campaign(&conn);
        let first = insert_landing(&conn, 1, 100, home, Some(google), None, 80, 2);
        insert_pageview(&conn, first, 2, 150, pricing);
        let second = insert_landing(&conn, 3, 120, home, Some(google), Some(campaign), 30, 1);

        let response = build_visitors_response(&conn, &baseline_query(0, 1_000)).expect("response");

        assert_eq!(response.visitors.len(), 2);
        assert_eq!(response.visitors[0].visitor_timestamp_id, first);
        assert_eq!(response.visitors[0].viewed_at, 100);
        assert_eq!(response.visitors[0].last_view_at, 160);
        assert_eq!(response.visitors[0].session_duration_seconds, 80);
        assert_eq!(response.visitors[0].session_page_count, 2);
        assert_eq!(
            response.visitors[0]
                .referrer
                .as_ref()
                .map(|referrer| referrer.referrer.as_str()),
            Some("google.com")
        );
        assert_eq!(response.visitors[1].visitor_timestamp_id, second);
        assert_eq!(
            response.visitors[1]
                .campaign
                .as_ref()
                .map(|campaign| campaign.display_name.as_deref()),
            Some(Some("Launch Email"))
        );
    }

    #[test]
    fn paginates_visitors_with_limit_and_offset() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/");
        let first = insert_landing(&conn, 1, 100, home, None, None, 5, 1);
        let second = insert_landing(&conn, 2, 90, home, None, None, 5, 1);
        insert_pageview(&conn, first, 3, 200, home);
        insert_pageview(&conn, second, 4, 150, home);

        let mut query = baseline_query(0, 1_000);
        let first_page = build_visitors_response(&conn, &query).expect("first page");
        assert_eq!(first_page.visitors[0].visitor_timestamp_id, first);
        assert_eq!(first_page.offset, 0);
        assert_eq!(first_page.limit, VISITORS_PAGE_SIZE);
        assert_eq!(first_page.next_offset, None);

        query.offset = 1;
        let second_page = build_visitors_response(&conn, &query).expect("second page");
        assert_eq!(second_page.visitors[0].visitor_timestamp_id, second);
        assert_eq!(second_page.next_offset, None);
    }

    #[test]
    fn orders_visitors_by_active_duration() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/");
        let short = insert_landing(&conn, 1, 100, home, None, None, 5, 1);
        let long = insert_landing(&conn, 2, 90, home, None, None, 50, 1);

        let mut query = baseline_query(0, 1_000);
        query.order_by = VisitorsOrderBy::ActiveDuration;
        query.direction = SortDirection::Asc;
        let response = build_visitors_response(&conn, &query).expect("response");

        assert_eq!(
            response
                .visitors
                .iter()
                .map(|visitor| visitor.visitor_timestamp_id)
                .collect::<Vec<_>>(),
            vec![short, long]
        );
    }

    #[test]
    fn filters_visitors_by_pageview_in_window() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/");
        let pricing = insert_path(&conn, "/pricing");
        let docs = insert_path(&conn, "/docs");
        let first = insert_landing(&conn, 1, 50, home, None, None, 80, 2);
        insert_pageview(&conn, first, 2, 150, pricing);
        let second = insert_landing(&conn, 3, 120, home, None, None, 20, 2);
        insert_pageview(&conn, second, 4, 160, docs);

        let mut query = baseline_query(100, 200);
        query.filter = VisitorsSessionFilter::Pageview { path_id: pricing };
        let response = build_visitors_response(&conn, &query).expect("response");

        assert_eq!(response.visitors.len(), 1);
        assert_eq!(response.visitors[0].visitor_timestamp_id, first);
    }

    #[test]
    fn filters_visitors_by_landing_page_in_window() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/");
        let pricing = insert_path(&conn, "/pricing");
        let first = insert_landing(&conn, 1, 90, pricing, None, None, 40, 1);
        let second = insert_landing(&conn, 2, 120, pricing, None, None, 20, 1);
        let _third = insert_landing(&conn, 3, 130, home, None, None, 20, 1);

        let mut query = baseline_query(100, 200);
        query.filter = VisitorsSessionFilter::LandingPage { path_id: pricing };
        let response = build_visitors_response(&conn, &query).expect("response");

        assert_eq!(response.visitors.len(), 1);
        assert_eq!(response.visitors[0].visitor_timestamp_id, second);
        assert_ne!(response.visitors[0].visitor_timestamp_id, first);
    }

    #[test]
    fn filters_visitors_by_event_on_page_in_window() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/");
        let pricing = insert_path(&conn, "/pricing");
        let signup = insert_event_name(&conn, "signup_clicked");
        let first = insert_landing(&conn, 1, 90, home, None, None, 80, 2);
        insert_pageview(&conn, first, 2, 150, pricing);
        let second = insert_landing(&conn, 3, 110, home, None, None, 20, 1);
        let first_pricing_pageview = timestamp_id(2, 150);
        let second_home_pageview = second;
        insert_event(&conn, 4, 160, signup, first_pricing_pageview);
        insert_event(&conn, 5, 170, signup, second_home_pageview);

        let mut query = baseline_query(100, 200);
        query.filter = VisitorsSessionFilter::Event {
            event_name_id: signup,
            path_id: pricing,
        };
        let response = build_visitors_response(&conn, &query).expect("response");

        assert_eq!(response.visitors.len(), 1);
        assert_eq!(response.visitors[0].visitor_timestamp_id, first);
    }

    #[test]
    fn unfiltered_response_has_no_filter_info() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/");
        insert_landing(&conn, 1, 100, home, None, None, 20, 1);

        let response = build_visitors_response(&conn, &baseline_query(0, 1_000)).expect("response");

        assert!(response.filter.is_none());
    }

    #[test]
    fn pageview_filter_response_describes_path() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/");
        let pricing = insert_path(&conn, "/pricing");
        let visitor = insert_landing(&conn, 1, 50, home, None, None, 80, 2);
        insert_pageview(&conn, visitor, 2, 150, pricing);

        let mut query = baseline_query(100, 200);
        query.filter = VisitorsSessionFilter::Pageview { path_id: pricing };
        let response = build_visitors_response(&conn, &query).expect("response");

        let filter = response.filter.expect("filter info");
        assert_eq!(filter.kind, "pageview");
        assert_eq!(filter.path, "/pricing");
        assert_eq!(filter.hostname, "example.com");
        assert!(filter.event_name.is_none());
    }

    #[test]
    fn event_filter_response_describes_event_and_path() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/");
        let pricing = insert_path(&conn, "/pricing");
        let signup = insert_event_name(&conn, "signup_clicked");
        let visitor = insert_landing(&conn, 1, 90, home, None, None, 80, 2);
        insert_pageview(&conn, visitor, 2, 150, pricing);
        insert_event(&conn, 4, 160, signup, timestamp_id(2, 150));

        let mut query = baseline_query(100, 200);
        query.filter = VisitorsSessionFilter::Event {
            event_name_id: signup,
            path_id: pricing,
        };
        let response = build_visitors_response(&conn, &query).expect("response");

        let filter = response.filter.expect("filter info");
        assert_eq!(filter.kind, "event");
        assert_eq!(filter.path, "/pricing");
        assert_eq!(filter.event_name.as_deref(), Some("signup_clicked"));
    }

    #[test]
    fn builds_visitor_journey_without_purchase() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/");
        let pricing = insert_path(&conn, "/pricing");
        let visitor = insert_landing(&conn, 1, 100, home, None, None, 20, 2);
        insert_pageview(&conn, visitor, 2, 110, pricing);

        let response = build_visitor_journey_response(&conn, visitor).expect("journey");

        assert_eq!(response.visitor.visitor_timestamp_id, visitor);
        assert_eq!(
            response
                .timeline
                .iter()
                .map(|action| action.kind.as_str())
                .collect::<Vec<_>>(),
            vec!["source", "pageview", "pageview"]
        );
    }

    #[test]
    fn excludes_consent_required_sessions_without_consent() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/");
        let us = insert_landing(&conn, 1, 100, home, None, None, 10, 1);
        let de_no_consent = insert_landing(&conn, 2, 110, home, None, None, 10, 1);
        let de_granted = insert_landing(&conn, 3, 120, home, None, None, 10, 1);
        let qc_no_consent = insert_landing(&conn, 4, 130, home, None, None, 10, 1);
        let on_no_consent = insert_landing(&conn, 5, 140, home, None, None, 10, 1);
        let br_no_consent = insert_landing(&conn, 6, 150, home, None, None, 10, 1);
        let br_granted = insert_landing(&conn, 7, 160, home, None, None, 10, 1);

        set_landing_geo(&conn, us, None, "US", None);
        set_landing_geo(&conn, de_no_consent, None, "DE", None);
        set_landing_geo(&conn, de_granted, Some(1), "DE", None);
        set_landing_geo(&conn, qc_no_consent, None, "CA", Some("QC"));
        set_landing_geo(&conn, on_no_consent, None, "CA", Some("ON"));
        set_landing_geo(&conn, br_no_consent, None, "BR", None);
        set_landing_geo(&conn, br_granted, Some(1), "BR", None);

        let response = build_visitors_response(&conn, &baseline_query(50, 200)).expect("list");
        let ids: Vec<i64> = response
            .visitors
            .iter()
            .map(|entry| entry.visitor_timestamp_id)
            .collect();
        assert!(ids.contains(&us));
        assert!(ids.contains(&de_granted));
        assert!(ids.contains(&on_no_consent));
        assert!(ids.contains(&br_granted));
        assert!(!ids.contains(&de_no_consent));
        assert!(!ids.contains(&qc_no_consent));
        assert!(!ids.contains(&br_no_consent));

        assert!(build_visitor_journey_response(&conn, de_granted).is_ok());
        assert!(build_visitor_journey_response(&conn, br_granted).is_ok());
        assert!(build_visitor_journey_response(&conn, de_no_consent).is_err());
        assert!(build_visitor_journey_response(&conn, qc_no_consent).is_err());
        assert!(build_visitor_journey_response(&conn, br_no_consent).is_err());
    }

    fn set_landing_geo(
        conn: &ProjectConnection,
        visitor_timestamp_id: i64,
        consent: Option<i64>,
        country: &str,
        region: Option<&str>,
    ) {
        conn.execute(
            "UPDATE landings SET consent = ?1, country = ?2, region = ?3
             WHERE visitor_timestamp_id = ?4",
            params![consent, country, region, visitor_timestamp_id],
        )
        .expect("set landing geo");
    }
}
