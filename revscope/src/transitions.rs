use std::sync::Arc;

use rusqlite::types::ToSql;
use rusqlite::{OptionalExtension, params_from_iter};
use serde::Serialize;

use crate::auth::{AuthError, map_row_error};
use crate::authorize::AuthorizedProject;
use crate::db::{ProjectConnection, open_project_database_read};
use crate::metrics::ratio;
use crate::query_params::parse_report_query;
use crate::segment_filter::{SegmentFilters, parse_segment_filters};
use crate::server_context::ServerContext;
use crate::util::NANOS_PER_SECOND;

/// Event name the tracker emits when a visitor clicks a link that leaves the
/// site (see `tracker.js`). Its `metadata.to` records the outbound destination.
const OUTBOUND_CLICK_EVENT_NAME: &str = "revscope-outbound-click";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PageTransitionsVariant {
    Pages,
    LandingPages,
}

#[derive(Debug, Serialize)]
pub(crate) struct PageTransitionsResponse {
    pub(crate) total_visitors: i64,
    pub(crate) transitions: Vec<PageTransitionRow>,
}

#[derive(Debug, Serialize)]
pub(crate) struct PageTransitionRow {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) path_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) hostname: Option<String>,
    /// Outbound destination (hostname or `mailto:` domain) for exits that left
    /// via a tracked outbound link. Set only when `path_id` is absent (a real
    /// exit re-attributed from the synthetic exit bucket).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) outbound_to: Option<String>,
    pub(crate) visitors: i64,
    pub(crate) share: f64,
}

pub(crate) struct PageTransitionsQuery {
    pub(crate) start: i64,
    pub(crate) end: i64,
    pub(crate) path_id: i64,
    pub(crate) variant: PageTransitionsVariant,
    pub(crate) segments: SegmentFilters,
}

pub(crate) fn parse_page_transitions_query(
    target: &str,
) -> Result<PageTransitionsQuery, &'static str> {
    let mut path_id = None;
    let mut variant = PageTransitionsVariant::Pages;
    let range = parse_report_query(target, |key, value| {
        match key {
            "path_id" => path_id = value.parse().ok(),
            "variant" => {
                variant = match value {
                    "landing_pages" | "landingPages" => PageTransitionsVariant::LandingPages,
                    "pages" => PageTransitionsVariant::Pages,
                    _ => return Err("invalid variant"),
                }
            }
            _ => {}
        }
        Ok(())
    })?;
    let (start, end) = range.validate()?;
    let path_id = path_id.ok_or("path_id is required")?;
    if path_id < 1 {
        return Err("path_id must be at least 1");
    }
    Ok(PageTransitionsQuery {
        start,
        end,
        path_id,
        variant,
        segments: parse_segment_filters(target),
    })
}

pub(crate) fn page_transitions(
    server_context: Arc<ServerContext>,
    project: &AuthorizedProject,
    query: PageTransitionsQuery,
) -> Result<PageTransitionsResponse, AuthError> {
    fetch_page_transitions(server_context, project.id(), &query)
}

fn fetch_page_transitions(
    server_context: Arc<ServerContext>,
    project_id: &str,
    query: &PageTransitionsQuery,
) -> Result<PageTransitionsResponse, AuthError> {
    let conn = open_project_database_read(server_context.as_ref(), project_id)
        .map_err(|error| AuthError::Db(error.to_string()))?;
    build_page_transitions_response(&conn, query)
}

pub(crate) fn build_page_transitions_response(
    conn: &ProjectConnection,
    query: &PageTransitionsQuery,
) -> Result<PageTransitionsResponse, AuthError> {
    let selected_source = selected_pageview_source(query.variant);
    // A small slice of "exit" visitors actually left via a tracked outbound link.
    // When the project recorded any, re-attribute those exits to their outbound
    // destination; otherwise run the plain transition query unchanged so the
    // common case pays nothing.
    let outbound_event_id = outbound_event_name_id(conn)?;
    // Country placeholders follow the fixed binds (path_id, start, end) and the
    // optional outbound event id, so their first index depends on whether the
    // outbound lookup runs.
    let country_placeholder_start = if outbound_event_id.is_some() { 5 } else { 4 };
    let country_filter = query
        .segments
        .country_exists_clause("pv.visitor_timestamp_id", country_placeholder_start);
    let session_filter = query
        .segments
        .session_filter_clause("pv.visitor_timestamp_id", None);
    let transitions_sql = page_transitions_sql(
        selected_source,
        outbound_event_id.is_some(),
        &format!("{country_filter}{session_filter}"),
    );

    let start = query.start * NANOS_PER_SECOND;
    let end = query.end * NANOS_PER_SECOND;

    let mut sql_params: Vec<&dyn ToSql> = vec![&query.path_id, &start, &end];
    if let Some(outbound_id) = &outbound_event_id {
        sql_params.push(outbound_id);
    }
    query.segments.append_country_params(&mut sql_params);

    let mut transitions = Vec::new();
    let mut total_visitors = 0i64;
    conn.with_query(
        &transitions_sql,
        params_from_iter(sql_params),
        |mut rows| {
            while let Some(row) = rows.next()? {
                let visitors = row.get::<_, i64>(3)?;
                total_visitors += visitors;
                transitions.push(PageTransitionRow {
                    path_id: row.get(0)?,
                    path: row.get(1)?,
                    hostname: row.get(2)?,
                    outbound_to: row.get(4)?,
                    visitors,
                    share: 0.0,
                });
            }
            Ok(())
        },
    )
    .map_err(map_row_error)?;

    for transition in &mut transitions {
        transition.share = ratio(transition.visitors, total_visitors);
    }

    Ok(PageTransitionsResponse {
        total_visitors,
        transitions,
    })
}

/// Builds the page-transitions SQL. Each selected pageview is the
/// `previous_timestamp_id` of at most one later pageview (ingest links the chain
/// to the visitor's most recent prior pageview), so the destination join is 1:1
/// and contributes exactly one destination per visitor (or NULL on exit). The
/// total is therefore the sum of the per-destination buckets and shares sum to
/// 100%.
///
/// When `with_outbound` is set, exits (`path_id IS NULL`) are probed for the
/// first revscope-outbound-click event on the selected page and split out by destination.
/// The `CASE` guard means the events lookup only runs for exit rows.
fn page_transitions_sql(
    selected_source: &str,
    with_outbound: bool,
    country_filter: &str,
) -> String {
    let ctes = format!(
        "WITH first_selected AS (
            SELECT
                pv.visitor_timestamp_id,
                MIN(pv.timestamp_id) AS selected_timestamp_id
            {selected_source}
            JOIN paths selected_path ON selected_path.id = pv.path_id
            WHERE pv.path_id = ?1
              AND pv.timestamp_id >= ?2
              AND pv.timestamp_id < ?3
              AND selected_path.deleted_at IS NULL{country_filter}
            GROUP BY pv.visitor_timestamp_id
         ),
         next_destinations AS (
            SELECT
                selected.selected_timestamp_id AS sel_ts,
                next.path_id AS path_id
            FROM first_selected selected
            LEFT JOIN pageviews next
              ON next.previous_timestamp_id = selected.selected_timestamp_id
         )"
    );

    if with_outbound {
        format!(
            "{ctes},
         classified AS (
            SELECT
                next_destinations.path_id AS path_id,
                CASE WHEN next_destinations.path_id IS NULL THEN (
                    SELECT jsonb_extract(e.metadata, '$.to')
                    FROM events e
                    WHERE e.page_timestamp_id = next_destinations.sel_ts
                      AND e.event_name_id = ?4
                    ORDER BY e.timestamp_id DESC
                    LIMIT 1
                ) END AS outbound_to
            FROM next_destinations
         )
         SELECT
             classified.path_id AS path_id,
             p.path AS path,
             p.hostname AS hostname,
             COUNT(*) AS visitors,
             classified.outbound_to AS outbound_to
         FROM classified
         LEFT JOIN paths p
                ON p.id = classified.path_id AND p.deleted_at IS NULL
         GROUP BY classified.path_id, classified.outbound_to
         ORDER BY visitors DESC, p.path ASC, p.hostname ASC"
        )
    } else {
        format!(
            "{ctes}
         SELECT
             next_destinations.path_id AS path_id,
             p.path AS path,
             p.hostname AS hostname,
             COUNT(*) AS visitors,
             NULL AS outbound_to
         FROM next_destinations
         LEFT JOIN paths p
                ON p.id = next_destinations.path_id AND p.deleted_at IS NULL
         GROUP BY next_destinations.path_id
         ORDER BY visitors DESC, p.path ASC, p.hostname ASC"
        )
    }
}

/// Resolves the `revscope-outbound-click` event name id for this project, or `None` when
/// the project never recorded an outbound click (so callers can skip the
/// outbound lookup entirely).
fn outbound_event_name_id(conn: &ProjectConnection) -> Result<Option<i64>, AuthError> {
    conn.query_row(
        "SELECT id FROM event_names WHERE name = ?1 AND deleted_at IS NULL",
        [OUTBOUND_CLICK_EVENT_NAME],
        |row| row.get::<_, i64>(0),
    )
    .optional()
    .map_err(map_row_error)
}

fn selected_pageview_source(variant: PageTransitionsVariant) -> &'static str {
    match variant {
        PageTransitionsVariant::Pages => "FROM pageviews pv",
        PageTransitionsVariant::LandingPages => {
            "FROM pageviews pv
             JOIN landings l ON l.visitor_timestamp_id = pv.timestamp_id"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::open_migrated_db;

    use rusqlite::params;

    use crate::db::ProjectConnection;

    fn timestamp_id(seed: u8, viewed_at: i64) -> i64 {
        viewed_at * NANOS_PER_SECOND + i64::from(seed)
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

    fn insert_landing(
        conn: &ProjectConnection,
        seed: u8,
        viewed_at: i64,
        path_id: i64,
        session_duration_seconds: i64,
        session_page_count: i64,
        scrolled_percent: Option<i64>,
    ) -> i64 {
        let session_id = timestamp_id(seed, viewed_at);
        conn.execute(
            "INSERT INTO pageviews (
                timestamp_id, visitor_timestamp_id, previous_timestamp_id, path_id,
                duration_seconds, scrolled_percent
             ) VALUES (?1, ?1, NULL, ?2, NULL, ?3)",
            params![session_id, path_id, scrolled_percent],
        )
        .expect("insert landing pageview");
        conn.execute(
            "INSERT INTO landings (
                visitor_timestamp_id, session_duration_seconds, session_page_count,
                is_google, tracker_version, device
             ) VALUES (?1, ?2, ?3, 0, 1.0, 'desktop')",
            params![session_id, session_duration_seconds, session_page_count],
        )
        .expect("insert landing");
        session_id
    }

    fn insert_linked_pageview(
        conn: &ProjectConnection,
        seed: u8,
        viewed_at: i64,
        path_id: i64,
        session_id: i64,
        previous_timestamp_id: i64,
    ) -> i64 {
        let view_id = timestamp_id(seed, viewed_at);
        conn.execute(
            "INSERT INTO pageviews (
                timestamp_id, visitor_timestamp_id, previous_timestamp_id, path_id,
                duration_seconds, scrolled_percent
             ) VALUES (?1, ?2, ?3, ?4, NULL, NULL)",
            params![view_id, session_id, previous_timestamp_id, path_id],
        )
        .expect("insert linked pageview");
        view_id
    }

    fn insert_outbound_click(
        conn: &ProjectConnection,
        seed: u8,
        occurred_at: i64,
        page_timestamp_id: i64,
        to: &str,
    ) {
        conn.execute(
            "INSERT INTO event_names (name) VALUES (?1) ON CONFLICT(name) DO NOTHING",
            params![OUTBOUND_CLICK_EVENT_NAME],
        )
        .expect("insert event name");
        let event_name_id = conn
            .query_row(
                "SELECT id FROM event_names WHERE name = ?1",
                params![OUTBOUND_CLICK_EVENT_NAME],
                |row| row.get::<_, i64>(0),
            )
            .expect("event name id");
        conn.execute(
            "INSERT INTO events (timestamp_id, event_name_id, page_timestamp_id, metadata)
             VALUES (?1, ?2, ?3, jsonb_object('to', ?4))",
            params![
                timestamp_id(seed, occurred_at),
                event_name_id,
                page_timestamp_id,
                to
            ],
        )
        .expect("insert outbound click event");
    }

    fn transitions_query(
        start: i64,
        end: i64,
        path_id: i64,
        variant: PageTransitionsVariant,
    ) -> PageTransitionsQuery {
        PageTransitionsQuery {
            start,
            end,
            path_id,
            variant,
            segments: SegmentFilters::default(),
        }
    }

    #[test]
    fn parses_page_transitions_query() {
        let query = parse_page_transitions_query(
            "/v2/read/projects/1/page-transitions?start=10&end=100&path_id=42&variant=landing_pages",
        )
        .expect("query");
        assert_eq!(query.start, 10);
        assert_eq!(query.end, 100);
        assert_eq!(query.path_id, 42);
        assert_eq!(query.variant, PageTransitionsVariant::LandingPages);
    }

    #[test]
    fn rejects_page_transitions_missing_path_id() {
        assert!(
            parse_page_transitions_query("/v2/read/projects/1/page-transitions?start=10&end=100",)
                .is_err()
        );
    }

    #[test]
    fn page_transitions_bucket_first_selected_view_by_next_page_or_exit() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/", "acme.test");
        let pricing = insert_path(&conn, "/pricing", "acme.test");
        let docs = insert_path(&conn, "/docs", "acme.test");

        // Session 1: home -> pricing.
        let s1 = insert_landing(&conn, 1, 100, home, 120, 2, Some(80));
        insert_linked_pageview(&conn, 2, 110, pricing, s1, s1);
        // Session 2: home -> docs.
        let s2 = insert_landing(&conn, 3, 120, home, 60, 2, Some(60));
        insert_linked_pageview(&conn, 4, 130, docs, s2, s2);
        // Session 3: home only, counted as exit.
        insert_landing(&conn, 5, 140, home, 20, 1, Some(40));
        // Session 4: pricing -> home -> docs; only the first home view is selected.
        let s4 = insert_landing(&conn, 6, 160, pricing, 90, 3, Some(70));
        let home_view = insert_linked_pageview(&conn, 7, 170, home, s4, s4);
        insert_linked_pageview(&conn, 8, 180, docs, s4, home_view);

        let response = build_page_transitions_response(
            &conn,
            &transitions_query(0, 10_000, home, PageTransitionsVariant::Pages),
        )
        .expect("response");

        assert_eq!(response.total_visitors, 4);
        let docs_row = response
            .transitions
            .iter()
            .find(|row| row.path.as_deref() == Some("/docs"))
            .expect("docs transition");
        assert_eq!(docs_row.path_id, Some(docs));
        assert_eq!(docs_row.visitors, 2);
        assert_eq!(docs_row.share, 0.5);

        let pricing_row = response
            .transitions
            .iter()
            .find(|row| row.path.as_deref() == Some("/pricing"))
            .expect("pricing transition");
        assert_eq!(pricing_row.path_id, Some(pricing));
        assert_eq!(pricing_row.visitors, 1);
        assert_eq!(pricing_row.share, 0.25);

        let exit_row = response
            .transitions
            .iter()
            .find(|row| row.path_id.is_none())
            .expect("exit transition");
        assert_eq!(exit_row.visitors, 1);
        assert_eq!(exit_row.share, 0.25);
    }

    #[test]
    fn splits_exit_bucket_by_outbound_click_destination() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/", "acme.test");
        let pricing = insert_path(&conn, "/pricing", "acme.test");

        // Session 1: home -> pricing (internal transition, not an exit).
        let s1 = insert_landing(&conn, 1, 100, home, 120, 2, Some(80));
        insert_linked_pageview(&conn, 2, 110, pricing, s1, s1);
        // Session 2: home, then clicks an outbound link to stripe.com and leaves.
        let s2 = insert_landing(&conn, 3, 120, home, 30, 1, Some(50));
        insert_outbound_click(&conn, 4, 125, s2, "stripe.com");
        // Session 3: home, outbound to stripe.com as well (same destination).
        let s3 = insert_landing(&conn, 5, 140, home, 20, 1, Some(40));
        insert_outbound_click(&conn, 6, 145, s3, "stripe.com");
        // Session 4: home, plain exit (no next page, no outbound click).
        insert_landing(&conn, 7, 160, home, 15, 1, Some(30));

        let response = build_page_transitions_response(
            &conn,
            &transitions_query(0, 10_000, home, PageTransitionsVariant::Pages),
        )
        .expect("response");

        assert_eq!(response.total_visitors, 4);

        let pricing_row = response
            .transitions
            .iter()
            .find(|row| row.path.as_deref() == Some("/pricing"))
            .expect("pricing transition");
        assert_eq!(pricing_row.visitors, 1);

        let outbound_row = response
            .transitions
            .iter()
            .find(|row| row.outbound_to.as_deref() == Some("stripe.com"))
            .expect("outbound transition");
        assert_eq!(outbound_row.path_id, None);
        assert_eq!(outbound_row.visitors, 2);
        assert_eq!(outbound_row.share, 0.5);

        // The plain exit keeps its own bucket (no path_id, no outbound_to).
        let exit_row = response
            .transitions
            .iter()
            .find(|row| row.path_id.is_none() && row.outbound_to.is_none())
            .expect("exit transition");
        assert_eq!(exit_row.visitors, 1);
    }

    #[test]
    fn landing_page_transitions_only_use_session_starts() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/", "acme.test");
        let pricing = insert_path(&conn, "/pricing", "acme.test");
        let docs = insert_path(&conn, "/docs", "acme.test");

        // Session 1 lands on home and moves to pricing.
        let s1 = insert_landing(&conn, 1, 100, home, 120, 2, Some(80));
        insert_linked_pageview(&conn, 2, 110, pricing, s1, s1);
        // Session 2 lands on pricing, later views home and moves to docs. This
        // should not count for the landing-pages variant of home.
        let s2 = insert_landing(&conn, 3, 120, pricing, 90, 3, Some(60));
        let home_view = insert_linked_pageview(&conn, 4, 130, home, s2, s2);
        insert_linked_pageview(&conn, 5, 140, docs, s2, home_view);

        let response = build_page_transitions_response(
            &conn,
            &transitions_query(0, 10_000, home, PageTransitionsVariant::LandingPages),
        )
        .expect("response");

        assert_eq!(response.total_visitors, 1);
        assert_eq!(response.transitions.len(), 1);
        assert_eq!(response.transitions[0].path_id, Some(pricing));
        assert_eq!(response.transitions[0].visitors, 1);
        assert_eq!(response.transitions[0].share, 1.0);
    }
}
