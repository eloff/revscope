use fxhash::{FxHashMap, FxHashSet};
use rusqlite::params_from_iter;
use rusqlite::types::ToSql;
use serde::Serialize;
use smallvec::SmallVec;
use std::cmp::Ordering;
use std::sync::Arc;

use crate::auth::{AuthError, map_row_error};
use crate::authorize::AuthorizedProject;
use crate::comparison::{self, Change};
use crate::db::{ProjectConnection, open_project_database_read};
use crate::ga_import::{ImportedEvent, ImportedWindows};
use crate::metrics::ratio;
use crate::metrics::{
    WilsonInterval, conversion_rate_wilson_interval, conversion_rate_wilson_lower_bound,
};
use crate::pages::{PagePayment, PagesOrderBy, PagesQuery, load_session_payments};
use crate::query_params::{SortDirection, SortedPage, cmp_f64, paginate, parse_report_query};
use crate::segment_filter::{SegmentFilters, parse_segment_filters};
use crate::server_context::ServerContext;
use crate::timezone;
use crate::util::{NANOS_PER_SECOND, seconds_since_epoch};

#[derive(Debug, Serialize)]
pub(crate) struct EventRow {
    pub(crate) event_name_id: i64,
    pub(crate) event_name: String,
    pub(crate) path_id: i64,
    pub(crate) path: String,
    pub(crate) hostname: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) goal_id: Option<i64>,
    pub(crate) visitors: i64,
    pub(crate) revenue: f64,
    pub(crate) conversions: i64,
    pub(crate) conversion_rate: f64,
    pub(crate) conversion_lower_bound: f64,
    pub(crate) conversion_ci: Option<WilsonInterval>,
    pub(crate) revenue_change: Change,
    pub(crate) visitors_change: Change,
    pub(crate) conversions_change: Change,
    pub(crate) conversion_rate_change: Change,
}

/// Events ranked by the requested `order_by` for the requested window.
///
/// Page size is [`EVENTS_PAGE_SIZE`]. Event×path rows grow without bound, so
/// the table and API both page instead of returning every combination.
#[derive(Debug, Serialize)]
pub(crate) struct EventsResponse {
    pub(crate) events: Vec<EventRow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) next_offset: Option<i64>,
    pub(crate) offset: i64,
    pub(crate) limit: i64,
}

/// Rows returned per page for `/events`.
pub(crate) const EVENTS_PAGE_SIZE: i64 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EventsOrderBy {
    Event,
    Page,
    Visitors,
    Revenue,
    Conversions,
    Conversion,
}

pub(crate) struct EventsQuery {
    pub(crate) start: i64,
    pub(crate) end: i64,
    pub(crate) offset: i64,
    pub(crate) order_by: EventsOrderBy,
    pub(crate) direction: SortDirection,
    pub(crate) segments: SegmentFilters,
}

pub(crate) fn parse_events_query(target: &str) -> Result<EventsQuery, &'static str> {
    let mut sort = SortedPage::new(EventsOrderBy::Revenue, parse_events_order_by);
    let range = parse_report_query(target, |key, value| sort.accept(key, value).map(drop))?;
    let (start, end) = range.validate()?;
    Ok(EventsQuery {
        start,
        end,
        offset: sort.offset,
        order_by: sort.order_by,
        direction: sort.direction,
        segments: parse_segment_filters(target),
    })
}

pub(crate) fn events(
    server_context: Arc<ServerContext>,
    project: &AuthorizedProject,
    query: EventsQuery,
) -> Result<EventsResponse, AuthError> {
    fetch_events(server_context, project, &query)
}

fn fetch_events(
    server_context: Arc<ServerContext>,
    project: &AuthorizedProject,
    query: &EventsQuery,
) -> Result<EventsResponse, AuthError> {
    let timezone = timezone::project_timezone(server_context.clone(), project.key());
    let conn = open_project_database_read(server_context.as_ref(), project.id())
        .map_err(|error| AuthError::Db(error.to_string()))?;
    let imported = ImportedWindows::for_report(
        &conn,
        &timezone,
        comparison::previous_period_start(query.start),
        query.start,
        query.end,
        &query.segments,
    )?;
    build_events_response_at(&conn, query, seconds_since_epoch() as i64, &imported)
}

#[cfg(test)]
pub(crate) fn build_events_response(
    conn: &ProjectConnection,
    query: &EventsQuery,
) -> Result<EventsResponse, AuthError> {
    build_events_response_at(
        conn,
        query,
        seconds_since_epoch() as i64,
        &ImportedWindows::default(),
    )
}

fn build_events_response_at(
    conn: &ProjectConnection,
    query: &EventsQuery,
    now: i64,
    imported: &ImportedWindows,
) -> Result<EventsResponse, AuthError> {
    let previous_start = comparison::previous_period_start(query.start);
    let factor = comparison::normalization_factor(query.start, query.end, now);
    // Visit-cohort dating: load payments for sessions with a pageview in the
    // scan window regardless of paid_at, matching Pages / Landing Pages.
    let payments_query = PagesQuery {
        start: previous_start,
        end: query.end,
        offset: 0,
        order_by: PagesOrderBy::AttributedRevenue,
        direction: SortDirection::Desc,
        segments: query.segments.clone(),
    };
    let session_payments = load_session_payments(conn, &payments_query)?;
    // Events have no country column; reach it through the visitor's landing row.
    let country_filter = query
        .segments
        .country_exists_clause("pv.visitor_timestamp_id", 3);
    // The events view lists events, so it ignores the event segment filter
    // (users add events from it); other dimensions still apply.
    let session_filter = query.segments.session_filter_clause(
        "pv.visitor_timestamp_id",
        Some(crate::segment_filter::SegmentDimension::Event),
    );
    // ORDER BY makes distinct visitors O(1) via last_session_id and lets us
    // attribute payments once on the earliest firing per (event, path, session).
    let sql = format!(
        "SELECT
            en.id AS event_name_id,
            en.name AS event_name,
            p.id AS path_id,
            p.path AS path,
            p.hostname AS hostname,
            pv.visitor_timestamp_id AS session_id,
            e.timestamp_id AS event_timestamp_id,
            funnel_steps.id AS goal_id
         FROM events e
         JOIN event_names en ON en.id = e.event_name_id
         JOIN pageviews pv ON pv.timestamp_id = e.page_timestamp_id
         JOIN paths p ON p.id = pv.path_id
         LEFT JOIN (
            SELECT funnel_steps.event_name_id, MIN(funnel_steps.id) AS id
            FROM funnel_steps
            JOIN funnels f ON f.id = funnel_steps.funnel_id AND f.deleted_at IS NULL
            WHERE funnel_steps.event_name_id IS NOT NULL
            GROUP BY funnel_steps.event_name_id
         ) funnel_steps ON funnel_steps.event_name_id = e.event_name_id
         WHERE e.timestamp_id >= ?1
           AND e.timestamp_id < ?2
           AND en.deleted_at IS NULL
           AND p.deleted_at IS NULL{country_filter}{session_filter}
         ORDER BY en.id, p.id, pv.visitor_timestamp_id, e.timestamp_id"
    );

    let mut current_events: FxHashMap<(i64, i64), EventAccumulator> = FxHashMap::default();
    let mut previous_events: FxHashMap<(i64, i64), EventAccumulator> = FxHashMap::default();

    let start_ns = previous_start * NANOS_PER_SECOND;
    let end_ns = query.end * NANOS_PER_SECOND;
    let mut sql_params: Vec<&dyn ToSql> = vec![&start_ns, &end_ns];
    query.segments.append_country_params(&mut sql_params);

    conn.with_query(&sql, params_from_iter(sql_params), |mut rows| {
        while let Some(row) = rows.next()? {
            let event_name_id = row.get::<_, i64>(0)?;
            let path_id = row.get::<_, i64>(2)?;
            let session_id = row.get::<_, i64>(5)?;
            let event_timestamp_id = row.get::<_, i64>(6)?;
            let goal_id = row.get::<_, Option<i64>>(7)?;
            let event_at = event_timestamp_id / NANOS_PER_SECOND;
            let events = if event_at >= query.start {
                &mut current_events
            } else {
                &mut previous_events
            };
            let entry = match events.get_mut(&(event_name_id, path_id)) {
                Some(entry) => entry,
                None => {
                    let event_name = row.get::<_, String>(1)?;
                    let path = row.get::<_, String>(3)?;
                    let hostname = row.get::<_, String>(4)?;
                    events.entry((event_name_id, path_id)).or_insert_with(|| {
                        EventAccumulator::new(event_name, path, hostname, goal_id)
                    })
                }
            };
            entry.add_event(
                session_id,
                event_timestamp_id,
                session_payments.get(&session_id),
            );
        }
        Ok(())
    })
    .map_err(map_row_error)?;

    if let Some(days) = &imported.current {
        add_imported_events(&mut current_events, days.events(conn)?);
    }
    if let Some(days) = &imported.previous {
        add_imported_events(&mut previous_events, days.events(conn)?);
    }

    Ok(paginate_event_rows(
        event_rows(current_events, &previous_events, factor),
        query,
    ))
}

fn add_imported_events(
    events: &mut FxHashMap<(i64, i64), EventAccumulator>,
    imported: Vec<ImportedEvent>,
) {
    for event in imported {
        events
            .entry((event.event_name_id, event.path_id))
            .or_insert_with(|| {
                EventAccumulator::new(event.event_name, event.path, event.hostname, event.goal_id)
            })
            .imported_visitors += event.sessions;
    }
}

/// Accumulates per-(event, path) metrics during the scan.
///
/// Callers must feed events ordered by
/// `(event_name_id, path_id, session_id, event_timestamp_id)` so
/// [`Self::add_event`] can treat a change in `session_id` as a new distinct
/// visitor and attribute payments once on the earliest firing. Converting
/// sessions stay in a set because cardinalities are small.
struct EventAccumulator {
    event_name: String,
    path: String,
    hostname: String,
    goal_id: Option<i64>,
    /// Last session id seen for this (event, path); used for O(1) distinct visitors.
    last_session_id: Option<i64>,
    seen_converting_sessions: FxHashSet<i64>,
    visitors: i64,
    /// Sessions from imported data, which have no revenue to convert into, so
    /// the conversion rate stays a share of [`Self::visitors`].
    imported_visitors: i64,
    conversions: i64,
    revenue: f64,
}

impl EventAccumulator {
    fn new(event_name: String, path: String, hostname: String, goal_id: Option<i64>) -> Self {
        Self {
            event_name,
            path,
            hostname,
            goal_id,
            last_session_id: None,
            seen_converting_sessions: FxHashSet::default(),
            visitors: 0,
            imported_visitors: 0,
            conversions: 0,
            revenue: 0.0,
        }
    }

    fn add_event(
        &mut self,
        session_id: i64,
        event_timestamp_id: i64,
        payments: Option<&SmallVec<[PagePayment; 1]>>,
    ) {
        // Later firings in the same session are skipped: visitors already
        // counted, and the earliest event_at attributes every payment a later
        // firing would (paid_at >= earlier => paid_at >= later).
        if self.last_session_id == Some(session_id) {
            return;
        }
        self.last_session_id = Some(session_id);
        self.visitors += 1;

        let Some(payments) = payments else {
            return;
        };
        let event_at = event_timestamp_id / NANOS_PER_SECOND;
        for payment in payments {
            if payment.amount <= 0 || payment.paid_at < event_at {
                continue;
            }
            self.revenue += payment.amount as f64;
            if self.seen_converting_sessions.insert(session_id) {
                self.conversions += 1;
            }
        }
    }
}

fn event_rows(
    events: FxHashMap<(i64, i64), EventAccumulator>,
    previous_events: &FxHashMap<(i64, i64), EventAccumulator>,
    factor: f64,
) -> Vec<EventRow> {
    events
        .into_iter()
        .map(|((event_name_id, path_id), event)| {
            let tracked_visitors = event.visitors;
            let visitors = tracked_visitors + event.imported_visitors;
            let conversions = event.conversions;
            let revenue = event.revenue;
            let conversion_rate = ratio(conversions, tracked_visitors);
            let previous = previous_events.get(&(event_name_id, path_id));
            EventRow {
                event_name_id,
                event_name: event.event_name,
                path_id,
                path: event.path,
                hostname: event.hostname,
                goal_id: event.goal_id,
                visitors,
                revenue,
                conversions,
                conversion_rate,
                conversion_lower_bound: conversion_rate_wilson_lower_bound(
                    conversions as f64,
                    tracked_visitors as f64,
                ),
                conversion_ci: conversion_rate_wilson_interval(
                    conversions as f64,
                    tracked_visitors as f64,
                ),
                revenue_change: Change::breakdown_absolute(
                    revenue,
                    previous.map(|p| p.revenue),
                    factor,
                ),
                visitors_change: Change::breakdown_absolute(
                    visitors as f64,
                    previous.map(|p| (p.visitors + p.imported_visitors) as f64),
                    factor,
                ),
                conversions_change: Change::breakdown_absolute(
                    conversions as f64,
                    previous.map(|p| p.conversions as f64),
                    factor,
                ),
                conversion_rate_change: Change::breakdown_rate(
                    conversion_rate,
                    previous.map(|p| ratio(p.conversions, p.visitors)),
                ),
            }
        })
        .collect()
}

fn paginate_event_rows(mut out: Vec<EventRow>, query: &EventsQuery) -> EventsResponse {
    out.sort_by(|a, b| cmp_event_rows(a, b, query.order_by, query.direction));
    let (page, next_offset) = paginate(out, query.offset, EVENTS_PAGE_SIZE);
    EventsResponse {
        events: page,
        next_offset,
        offset: query.offset,
        limit: EVENTS_PAGE_SIZE,
    }
}

fn cmp_event_rows(
    a: &EventRow,
    b: &EventRow,
    order_by: EventsOrderBy,
    direction: SortDirection,
) -> Ordering {
    let primary = match order_by {
        EventsOrderBy::Event => cmp_event_name(a, b),
        EventsOrderBy::Page => cmp_event_page(a, b),
        EventsOrderBy::Visitors => a.visitors.cmp(&b.visitors),
        EventsOrderBy::Revenue => cmp_f64(a.revenue, b.revenue),
        EventsOrderBy::Conversions => a.conversions.cmp(&b.conversions),
        EventsOrderBy::Conversion => cmp_f64(a.conversion_lower_bound, b.conversion_lower_bound),
    };
    let primary = direction.apply(primary);
    if primary != Ordering::Equal {
        return primary;
    }
    if matches!(order_by, EventsOrderBy::Revenue) {
        let visitors = b.visitors.cmp(&a.visitors);
        if visitors != Ordering::Equal {
            return visitors;
        }
    } else if !matches!(order_by, EventsOrderBy::Event | EventsOrderBy::Page) {
        let revenue = cmp_f64(b.revenue, a.revenue);
        if revenue != Ordering::Equal {
            return revenue;
        }
    }
    cmp_event_name(a, b).then_with(|| cmp_event_page(a, b))
}

fn cmp_event_name(a: &EventRow, b: &EventRow) -> Ordering {
    a.event_name
        .to_ascii_lowercase()
        .cmp(&b.event_name.to_ascii_lowercase())
}

fn cmp_event_page(a: &EventRow, b: &EventRow) -> Ordering {
    let left = format!("{}{}", a.hostname, a.path).to_ascii_lowercase();
    let right = format!("{}{}", b.hostname, b.path).to_ascii_lowercase();
    left.cmp(&right)
}

fn parse_events_order_by(value: &str) -> Result<EventsOrderBy, &'static str> {
    match value {
        "event" => Ok(EventsOrderBy::Event),
        "page" => Ok(EventsOrderBy::Page),
        "visitors" => Ok(EventsOrderBy::Visitors),
        "revenue" => Ok(EventsOrderBy::Revenue),
        "conversions" => Ok(EventsOrderBy::Conversions),
        "conversion" => Ok(EventsOrderBy::Conversion),
        _ => Err("invalid order_by"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::open_migrated_db;
    use rusqlite::params;

    use crate::db::ProjectConnection;

    use crate::test_utils::{PaymentFixture, insert_payment as insert_test_payment};

    fn timestamp_id(seed: u8, timestamp: i64) -> i64 {
        timestamp * NANOS_PER_SECOND + i64::from(seed)
    }

    fn insert_path(conn: &ProjectConnection, path: &str, hostname: &str) -> i64 {
        conn.execute(
            "INSERT INTO paths (path, hostname) VALUES (?1, ?2)",
            params![path, hostname],
        )
        .expect("insert path");
        conn.query_row(
            "SELECT id FROM paths WHERE path = ?1 AND hostname = ?2",
            params![path, hostname],
            |row| row.get::<_, i64>(0),
        )
        .expect("path id")
    }

    fn insert_pageview(
        conn: &ProjectConnection,
        seed: u8,
        viewed_at: i64,
        path_id: i64,
        session_id: Option<i64>,
    ) -> i64 {
        let pageview_id = timestamp_id(seed, viewed_at);
        let session_id = session_id.unwrap_or(pageview_id);
        conn.execute(
            "INSERT INTO pageviews (
                timestamp_id, visitor_timestamp_id, previous_timestamp_id, path_id,
                duration_seconds, scrolled_percent
             ) VALUES (?1, ?2, NULL, ?3, NULL, NULL)",
            params![pageview_id, session_id, path_id],
        )
        .expect("insert pageview");
        if session_id == pageview_id {
            conn.execute(
                "INSERT INTO landings (
                    visitor_timestamp_id, session_duration_seconds, session_page_count,
                    is_google, tracker_version, device
                 ) VALUES (?1, 120, 1, 0, 1.0, 'desktop')",
                params![session_id],
            )
            .expect("insert landing");
        }
        pageview_id
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

    fn insert_event_goal(conn: &ProjectConnection, path_id: i64, event_name_id: i64) -> i64 {
        conn.execute("INSERT INTO funnels (name) VALUES ('test')", [])
            .expect("insert funnel");
        let funnel_id = conn
            .query_row(
                "SELECT id FROM funnels ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("funnel id");
        conn.execute(
            "INSERT INTO funnel_steps (funnel_id, step_type, path_id, event_name_id, is_landing, step_order)
             VALUES (?1, 'pageview', ?2, ?3, 0, 0)",
            params![funnel_id, path_id, event_name_id],
        )
        .expect("insert funnel step");
        conn.query_row(
            "SELECT id FROM funnel_steps WHERE event_name_id = ?1",
            params![event_name_id],
            |row| row.get::<_, i64>(0),
        )
        .expect("funnel step id")
    }

    fn insert_event(
        conn: &ProjectConnection,
        seed: u8,
        timestamp: i64,
        event_name_id: i64,
        page_timestamp_id: i64,
        metadata: Option<&str>,
    ) {
        conn.execute(
            "INSERT INTO events (timestamp_id, event_name_id, page_timestamp_id, metadata)
             VALUES (?1, ?2, ?3, jsonb(?4))",
            params![
                timestamp_id(seed, timestamp),
                event_name_id,
                page_timestamp_id,
                metadata
            ],
        )
        .expect("insert event");
    }

    fn insert_payment(
        conn: &ProjectConnection,
        provider_payment_id: &str,
        session_id: i64,
        amount: i64,
        paid_at: i64,
    ) {
        let mut payment = PaymentFixture::new(provider_payment_id, amount, paid_at);
        payment.visitor_timestamp_id = Some(session_id);
        insert_test_payment(conn, payment);
    }

    fn baseline_query(start: i64, end: i64) -> EventsQuery {
        EventsQuery {
            start,
            end,
            offset: 0,
            order_by: EventsOrderBy::Revenue,
            direction: SortDirection::Desc,
            segments: crate::segment_filter::SegmentFilters::default(),
        }
    }

    fn assert_ratio(actual: Option<f64>, expected: f64) {
        let actual = actual.expect("change ratio");
        assert!(
            (actual - expected).abs() < 1e-12,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn parses_events_query_like_pages() {
        let query =
            parse_events_query("/v2/read/projects/1/events?start=10&end=100").expect("query");
        assert_eq!(query.start, 10);
        assert_eq!(query.end, 100);
        assert_eq!(query.offset, 0);
        assert_eq!(query.order_by, EventsOrderBy::Revenue);
        assert_eq!(query.direction, SortDirection::Desc);
    }

    #[test]
    fn parses_offset_and_ordering() {
        let query = parse_events_query(
            "/v2/read/projects/1/events?start=1&end=100&offset=200&order_by=visitors&direction=asc",
        )
        .expect("query");
        assert_eq!(query.offset, 200);
        assert_eq!(query.order_by, EventsOrderBy::Visitors);
        assert_eq!(query.direction, SortDirection::Asc);
    }

    #[test]
    fn rejects_invalid_offset_and_ordering() {
        assert!(
            parse_events_query("/v2/read/projects/1/events?start=1&end=100&offset=-1").is_err()
        );
        assert!(
            parse_events_query("/v2/read/projects/1/events?start=1&end=100&order_by=bad",).is_err()
        );
        assert!(
            parse_events_query("/v2/read/projects/1/events?start=1&end=100&direction=sideways",)
                .is_err()
        );
    }

    #[test]
    fn aggregates_events_by_name_and_page_path() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/", "acme.test");
        let pricing = insert_path(&conn, "/pricing", "acme.test");
        let signup = insert_event_name(&conn, "signup_clicked");

        let s1_page = insert_pageview(&conn, 1, 100, home, None);
        let s1 = s1_page;
        insert_event(&conn, 2, 110, signup, s1_page, None);
        insert_event(&conn, 3, 115, signup, s1_page, None);

        let s2_page = insert_pageview(&conn, 4, 120, pricing, None);
        let s2 = s2_page;
        insert_event(&conn, 5, 130, signup, s2_page, None);

        insert_payment(&conn, "pay_1", s1, 50_000, 200);
        insert_payment(&conn, "pay_2", s2, 25_000, 125);

        let response = build_events_response(&conn, &baseline_query(0, 10_000)).expect("response");
        assert_eq!(response.events.len(), 2);
        assert_eq!(response.offset, 0);
        assert_eq!(response.limit, EVENTS_PAGE_SIZE);
        assert_eq!(response.next_offset, None);

        let home_row = response
            .events
            .iter()
            .find(|row| row.path == "/")
            .expect("home row");
        assert_eq!(home_row.event_name, "signup_clicked");
        assert_eq!(home_row.visitors, 1);
        assert_eq!(home_row.conversions, 1);
        assert_eq!(home_row.revenue, 50_000.0);

        let pricing_row = response
            .events
            .iter()
            .find(|row| row.path == "/pricing")
            .expect("pricing row");
        assert_eq!(pricing_row.visitors, 1);
        assert_eq!(pricing_row.conversions, 0);
        assert_eq!(pricing_row.revenue, 0.0);
        assert!(pricing_row.conversion_ci.is_some());
    }

    #[test]
    fn attributes_payment_once_from_earliest_firing_in_session() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/", "acme.test");
        let signup = insert_event_name(&conn, "signup_clicked");

        let page = insert_pageview(&conn, 1, 100, home, None);
        // Payment sits between two firings: only the earlier event_at qualifies.
        insert_event(&conn, 2, 110, signup, page, None);
        insert_payment(&conn, "pay_mid", page, 40_000, 112);
        insert_event(&conn, 3, 115, signup, page, None);

        let response = build_events_response(&conn, &baseline_query(0, 10_000)).expect("response");
        assert_eq!(response.events.len(), 1);
        let row = &response.events[0];
        assert_eq!(row.visitors, 1);
        assert_eq!(row.conversions, 1);
        assert_eq!(row.revenue, 40_000.0);
    }

    #[test]
    fn event_segment_filter_does_not_narrow_events_view() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/", "acme.test");
        let signup = insert_event_name(&conn, "signup_clicked");
        let checkout = insert_event_name(&conn, "checkout_started");

        let s1 = insert_pageview(&conn, 1, 100, home, None);
        insert_event(&conn, 2, 110, signup, s1, None);
        let s2 = insert_pageview(&conn, 3, 200, home, None);
        insert_event(&conn, 4, 210, checkout, s2, None);

        // Filtering on one event must not hide the other in the events view.
        let mut query = baseline_query(0, 10_000);
        query.segments.events = vec![signup];
        let response = build_events_response(&conn, &query).expect("response");
        let names: Vec<&str> = response
            .events
            .iter()
            .map(|row| row.event_name.as_str())
            .collect();
        assert!(names.contains(&"signup_clicked"));
        assert!(names.contains(&"checkout_started"));
    }

    #[test]
    fn page_segment_filter_narrows_events_to_sessions_with_that_page() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/", "acme.test");
        let pricing = insert_path(&conn, "/pricing", "acme.test");
        let signup = insert_event_name(&conn, "signup_clicked");

        // Session 1 views /pricing and fires signup there.
        let s1 = insert_pageview(&conn, 1, 100, pricing, None);
        insert_event(&conn, 2, 110, signup, s1, None);
        // Session 2 only views /home and fires signup there.
        let s2 = insert_pageview(&conn, 3, 200, home, None);
        insert_event(&conn, 4, 210, signup, s2, None);

        let mut query = baseline_query(0, 10_000);
        query.segments.pages = vec![pricing];
        let response = build_events_response(&conn, &query).expect("response");
        // Only session 1 matches the page filter, so only its event row remains.
        assert_eq!(response.events.len(), 1);
        assert_eq!(response.events[0].path, "/pricing");
        assert_eq!(response.events[0].visitors, 1);
    }

    #[test]
    fn includes_goal_id_for_event_goals() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/", "acme.test");
        let signup = insert_event_name(&conn, "signup_clicked");
        let checkout = insert_event_name(&conn, "checkout_started");
        let goal_id = insert_event_goal(&conn, home, signup);

        let page = insert_pageview(&conn, 1, 100, home, None);
        insert_event(&conn, 2, 110, signup, page, None);
        insert_event(&conn, 3, 115, checkout, page, None);

        let response = build_events_response(&conn, &baseline_query(0, 10_000)).expect("response");
        assert_eq!(response.events.len(), 2);

        let goal_row = response
            .events
            .iter()
            .find(|row| row.event_name == "signup_clicked")
            .expect("goal row");
        assert_eq!(goal_row.goal_id, Some(goal_id));

        let event_row = response
            .events
            .iter()
            .find(|row| row.event_name == "checkout_started")
            .expect("event row");
        assert_eq!(event_row.goal_id, None);
    }

    #[test]
    fn includes_previous_period_changes_for_event_rows() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/", "acme.test");
        let signup = insert_event_name(&conn, "signup_clicked");
        let day = 86_400;
        let start = 30 * day;
        let end = 60 * day;

        let previous_page_1 = insert_pageview(&conn, 1, day, home, None);
        let previous_session_1 = previous_page_1;
        insert_event(&conn, 2, day + 10, signup, previous_page_1, None);
        insert_payment(&conn, "pay_previous", previous_session_1, 500, day + 100);

        // Previous-period event with payment after that window still counts
        // (visit-cohort dating; payment date unconstrained).
        let previous_page_2 = insert_pageview(&conn, 3, 2 * day, home, None);
        let previous_session_2 = previous_page_2;
        insert_event(&conn, 4, 2 * day + 10, signup, previous_page_2, None);
        insert_payment(
            &conn,
            "pay_previous_after_window",
            previous_session_2,
            500,
            start + 100,
        );

        let current_page_1 = insert_pageview(&conn, 5, start + day, home, None);
        let current_session_1 = current_page_1;
        insert_event(&conn, 6, start + day + 10, signup, current_page_1, None);
        insert_payment(
            &conn,
            "pay_current_1",
            current_session_1,
            1_000,
            start + day + 100,
        );

        // Payment after the selected period still attributes to the event cohort.
        let current_page_2 = insert_pageview(&conn, 7, start + 2 * day, home, None);
        let current_session_2 = current_page_2;
        insert_event(&conn, 8, start + 2 * day + 10, signup, current_page_2, None);
        insert_payment(&conn, "pay_current_2", current_session_2, 500, end + 100);

        let current_page_3 = insert_pageview(&conn, 9, start + 3 * day, home, None);
        insert_event(
            &conn,
            10,
            start + 3 * day + 10,
            signup,
            current_page_3,
            None,
        );

        let response = build_events_response_at(
            &conn,
            &baseline_query(start, end),
            end,
            &ImportedWindows::default(),
        )
        .expect("response");
        assert_eq!(response.events.len(), 1);
        let row = &response.events[0];

        assert_eq!(row.visitors, 3);
        assert_eq!(row.conversions, 2);
        assert_eq!(row.revenue, 1_500.0);
        assert_ratio(row.visitors_change.ratio, 0.5);
        assert_ratio(row.conversions_change.ratio, 0.0);
        assert_ratio(row.revenue_change.ratio, 0.5);
        assert_ratio(row.conversion_rate_change.ratio, (2.0 / 3.0) - 1.0);
    }

    #[test]
    fn paginates_events_with_limit_and_offset() {
        let conn = open_migrated_db();
        let signup = insert_event_name(&conn, "signup_clicked");
        let total = (EVENTS_PAGE_SIZE as usize) + 1;
        for index in 0..total {
            let path_id = insert_path(&conn, &format!("/page-{index:03}"), "acme.test");
            let session = insert_pageview(&conn, index as u8, 100 + index as i64, path_id, None);
            insert_event(
                &conn,
                index as u8,
                110 + index as i64,
                signup,
                session,
                None,
            );
            // Distinct revenue so default (revenue desc) order is deterministic:
            // highest remaining index first.
            insert_payment(
                &conn,
                &format!("pay_{index}"),
                session,
                (total - index) as i64 * 1_000,
                5_000,
            );
        }

        let mut query = baseline_query(0, 10_000);
        let first_page = build_events_response(&conn, &query).expect("first page");
        assert_eq!(first_page.events.len(), EVENTS_PAGE_SIZE as usize);
        assert_eq!(first_page.offset, 0);
        assert_eq!(first_page.limit, EVENTS_PAGE_SIZE);
        assert_eq!(first_page.next_offset, Some(EVENTS_PAGE_SIZE));
        assert_eq!(first_page.events[0].path, "/page-000");
        assert_eq!(
            first_page.events.last().expect("last").path,
            format!("/page-{:03}", EVENTS_PAGE_SIZE - 1)
        );

        query.offset = EVENTS_PAGE_SIZE;
        let second_page = build_events_response(&conn, &query).expect("second page");
        assert_eq!(second_page.events.len(), 1);
        assert_eq!(second_page.next_offset, None);
        assert_eq!(
            second_page.events[0].path,
            format!("/page-{:03}", EVENTS_PAGE_SIZE)
        );
    }

    #[test]
    fn sorts_events_by_visitors() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/", "acme.test");
        let pricing = insert_path(&conn, "/pricing", "acme.test");
        let signup = insert_event_name(&conn, "signup_clicked");
        let checkout = insert_event_name(&conn, "checkout_started");

        let s1 = insert_pageview(&conn, 1, 100, home, None);
        insert_event(&conn, 2, 110, signup, s1, None);
        let s2 = insert_pageview(&conn, 3, 200, home, None);
        insert_event(&conn, 4, 210, signup, s2, None);
        let s3 = insert_pageview(&conn, 5, 300, pricing, None);
        insert_event(&conn, 6, 310, checkout, s3, None);

        let mut query = baseline_query(0, 10_000);
        query.order_by = EventsOrderBy::Visitors;
        query.direction = SortDirection::Desc;
        let response = build_events_response(&conn, &query).expect("response");
        assert_eq!(response.events.len(), 2);
        assert_eq!(response.events[0].event_name, "signup_clicked");
        assert_eq!(response.events[0].visitors, 2);
        assert_eq!(response.events[1].event_name, "checkout_started");
        assert_eq!(response.events[1].visitors, 1);
    }
}
