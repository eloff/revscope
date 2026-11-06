use fxhash::FxHashMap;
use std::sync::Arc;

use rusqlite::params_from_iter;
use rusqlite::types::ToSql;

use crate::auth::{AuthError, map_row_error};
use crate::authorize::AuthorizedProject;
use crate::db::{ProjectConnection, open_project_database_read};
use crate::ga_import::ImportedDays;
use crate::pages::{
    PageAccumulator, PagesQuery, PagesResponse, imported_entry, load_session_payments, page_rows,
};
use crate::server_context::ServerContext;
use crate::timezone;
use crate::util::NANOS_PER_SECOND;

pub(crate) fn parse_landing_pages_query(target: &str) -> Result<PagesQuery, &'static str> {
    crate::pages::parse_pages_query(target)
}

pub(crate) fn landing_pages(
    server_context: Arc<ServerContext>,
    project: &AuthorizedProject,
    query: PagesQuery,
) -> Result<PagesResponse, AuthError> {
    fetch_landing_pages(server_context, project, &query)
}

fn fetch_landing_pages(
    server_context: Arc<ServerContext>,
    project: &AuthorizedProject,
    query: &PagesQuery,
) -> Result<PagesResponse, AuthError> {
    let timezone = timezone::project_timezone(server_context.clone(), project.key());
    let conn = open_project_database_read(server_context.as_ref(), project.id())
        .map_err(|error| AuthError::Db(error.to_string()))?;
    let imported =
        ImportedDays::for_report(&conn, &timezone, query.start, query.end, &query.segments)?;
    build_landing_pages_response_with_imported(&conn, query, imported.as_ref())
}

/// Builds landing-page metrics for sessions that started in the requested window.
///
/// A landing page is the first pageview in a session (`landings.visitor_timestamp_id`).
/// Session-scoped values come directly from that landing row, scroll is averaged
/// from the landing pageview, and revenue is the sum of successful payments
/// attached to those sessions (payment date unconstrained; visit cohort dating).
pub(crate) fn build_landing_pages_response_with_imported(
    conn: &ProjectConnection,
    query: &PagesQuery,
    imported: Option<&ImportedDays>,
) -> Result<PagesResponse, AuthError> {
    let session_payments = load_session_payments(conn, query)?;
    let country_filter = query.segments.country_in_clause("l.country", 3);
    // The landing-pages view lists landing pages, so it ignores the landing-page
    // segment filter (users add landing pages from it); other dimensions apply.
    let session_filter = query.segments.session_filter_clause(
        "l.visitor_timestamp_id",
        Some(crate::segment_filter::SegmentDimension::LandingPage),
    );
    let sql = format!(
        "SELECT
            p.id AS path_id,
            p.path AS path,
            p.hostname AS hostname,
            l.visitor_timestamp_id AS session_id,
            pv.scrolled_percent AS scrolled_percent,
            pv.duration_seconds AS duration_seconds,
            l.session_duration_seconds AS session_duration_seconds
         FROM landings l
         JOIN pageviews pv ON pv.timestamp_id = l.visitor_timestamp_id
         JOIN paths p ON p.id = pv.path_id
         WHERE l.visitor_timestamp_id >= ?1
           AND l.visitor_timestamp_id < ?2
           AND p.deleted_at IS NULL{country_filter}{session_filter}
         ORDER BY p.id, l.visitor_timestamp_id"
    );

    let mut pages: FxHashMap<i64, PageAccumulator> = FxHashMap::default();

    let start_ns = query.start * NANOS_PER_SECOND;
    let end_ns = query.end * NANOS_PER_SECOND;
    let mut sql_params: Vec<&dyn ToSql> = vec![&start_ns, &end_ns];
    query.segments.append_country_params(&mut sql_params);

    conn.with_query(&sql, params_from_iter(sql_params), |mut rows| {
        while let Some(row) = rows.next()? {
            let path_id = row.get::<_, i64>(0)?;
            let session_id = row.get::<_, i64>(3)?;
            let scrolled_percent = row.get::<_, Option<i64>>(4)?;
            let duration_seconds = row.get::<_, Option<i64>>(5)?;
            let session_duration_seconds = row.get::<_, i64>(6)?;

            let entry = match pages.get_mut(&path_id) {
                Some(entry) => entry,
                None => {
                    let path = row.get::<_, String>(1)?;
                    let hostname = row.get::<_, String>(2)?;
                    pages
                        .entry(path_id)
                        .or_insert_with(|| PageAccumulator::new(path, hostname))
                }
            };
            entry.add_pageview(scrolled_percent, duration_seconds);
            entry.add_session(session_id, session_duration_seconds, false);
            if let Some(payments) = session_payments.get(&session_id) {
                let landed_at = session_id / NANOS_PER_SECOND;
                for payment in payments {
                    if payment.paid_at >= landed_at {
                        entry.add_landing_payment(
                            session_id,
                            payment.amount,
                            duration_seconds,
                            payment.session_duration_seconds,
                        );
                    }
                }
            }
        }
        Ok(())
    })
    .map_err(map_row_error)?;

    if let Some(days) = imported {
        for page in days.landing_pages(conn)? {
            imported_entry(&mut pages, &page).add_imported_landing(&page);
        }
    }
    Ok(page_rows(pages, query))
}

#[cfg(test)]
pub(crate) fn build_landing_pages_response(
    conn: &ProjectConnection,
    query: &PagesQuery,
) -> Result<PagesResponse, AuthError> {
    build_landing_pages_response_with_imported(conn, query, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::open_migrated_db;
    use rusqlite::params;

    use crate::db::ProjectConnection;

    use crate::test_utils::{PaymentFixture, insert_payment as insert_test_payment};

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
        insert_landing_with_duration(
            conn,
            seed,
            viewed_at,
            path_id,
            session_duration_seconds,
            session_page_count,
            scrolled_percent,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_landing_with_duration(
        conn: &ProjectConnection,
        seed: u8,
        viewed_at: i64,
        path_id: i64,
        session_duration_seconds: i64,
        session_page_count: i64,
        scrolled_percent: Option<i64>,
        duration_seconds: Option<i64>,
    ) -> i64 {
        let session_id = timestamp_id(seed, viewed_at);
        conn.execute(
            "INSERT INTO pageviews (
                timestamp_id, visitor_timestamp_id, previous_timestamp_id, path_id,
                duration_seconds, scrolled_percent
             ) VALUES (?1, ?1, NULL, ?2, ?3, ?4)",
            params![session_id, path_id, duration_seconds, scrolled_percent],
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

    fn insert_pageview(
        conn: &ProjectConnection,
        seed: u8,
        viewed_at: i64,
        path_id: i64,
        session_id: i64,
        scrolled_percent: Option<i64>,
    ) {
        insert_pageview_with_duration(
            conn,
            seed,
            viewed_at,
            path_id,
            session_id,
            scrolled_percent,
            None,
        );
    }

    fn insert_pageview_with_duration(
        conn: &ProjectConnection,
        seed: u8,
        viewed_at: i64,
        path_id: i64,
        session_id: i64,
        scrolled_percent: Option<i64>,
        duration_seconds: Option<i64>,
    ) {
        let view_id = timestamp_id(seed, viewed_at);
        conn.execute(
            "INSERT INTO pageviews (
                timestamp_id, visitor_timestamp_id, previous_timestamp_id, path_id,
                duration_seconds, scrolled_percent
             ) VALUES (?1, ?2, NULL, ?3, ?4, ?5)",
            params![
                view_id,
                session_id,
                path_id,
                duration_seconds,
                scrolled_percent
            ],
        )
        .expect("insert pageview");
    }

    fn insert_payment(
        conn: &ProjectConnection,
        provider_payment_id: &str,
        session_id: i64,
        amount: i64,
        paid_at: i64,
        session_duration_seconds: Option<i64>,
    ) {
        let mut payment = PaymentFixture::new(provider_payment_id, amount, paid_at);
        payment.visitor_timestamp_id = Some(session_id);
        payment.session_duration_seconds = session_duration_seconds;
        insert_test_payment(conn, payment);
    }

    fn baseline_query(start: i64, end: i64) -> PagesQuery {
        PagesQuery {
            start,
            end,
            offset: 0,
            order_by: crate::pages::PagesOrderBy::AttributedRevenue,
            direction: crate::query_params::SortDirection::Desc,
            segments: crate::segment_filter::SegmentFilters::default(),
        }
    }

    #[test]
    fn the_highest_bounce_mark_follows_the_rate_on_show() {
        let conn = open_migrated_db();
        let imported_heavy = insert_path(&conn, "/a", "acme.test");
        let tracked_only = insert_path(&conn, "/b", "acme.test");
        for seed in 0..10u8 {
            let duration = if seed == 0 { 60 } else { 1 };
            insert_landing(
                &conn,
                seed,
                100 + i64::from(seed),
                imported_heavy,
                duration,
                1,
                None,
            );
        }
        for seed in 10..30u8 {
            let duration = if seed < 20 { 60 } else { 1 };
            insert_landing(
                &conn,
                seed,
                100 + i64::from(seed),
                tracked_only,
                duration,
                1,
                None,
            );
        }
        conn.execute_batch(&format!(
            "INSERT INTO imports VALUES (1, 'google_analytics', 'Acme', '1970-01-01', '1970-01-01');
             INSERT INTO imported_landing_pages VALUES (1, '1970-01-01', {imported_heavy}, 990, 890, 0);"
        ))
        .expect("seed");
        let days = ImportedDays::covering(&crate::timezone::ProjectTimezone::utc(), 0, 86_400);
        let response = build_landing_pages_response_with_imported(
            &conn,
            &baseline_query(0, 86_400),
            days.as_ref(),
        )
        .expect("response");
        let row = |id| {
            response
                .pages
                .iter()
                .find(|page| page.id == id)
                .expect("row")
        };
        // 109 of 1000 bounced on the first, 10 of 20 on the second.
        assert!((row(imported_heavy).bounce_rate - 0.109).abs() < 1e-9);
        assert!(row(tracked_only).highest_bounce_rate);
        assert!(!row(imported_heavy).highest_bounce_rate);
    }

    #[test]
    fn attributes_payments_outside_query_window_when_landing_is_inside() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/", "acme.test");
        let session = insert_landing(&conn, 1, 100, home, 120, 1, Some(80));
        insert_payment(&conn, "pay_1", session, 30_000, 500, None);

        let response =
            build_landing_pages_response(&conn, &baseline_query(0, 400)).expect("response");
        assert_eq!(response.pages.len(), 1);
        assert_eq!(response.pages[0].converting_visitors, 1);
        assert_eq!(response.pages[0].attributed_revenue, 30_000.0);
    }

    #[test]
    fn landing_pages_count_only_sessions_started_on_each_page() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/", "acme.test");
        let pricing = insert_path(&conn, "/pricing", "acme.test");

        // Session 1 lands on home, then visits pricing, and converts $30.
        let s1 = insert_landing(&conn, 1, 100, home, 120, 2, Some(80));
        insert_pageview(&conn, 2, 110, pricing, s1, Some(50));
        // Session 2 lands directly on pricing, bounces, and converts $50.
        let s2 = insert_landing(&conn, 3, 120, pricing, 5, 1, Some(10));
        // Session 3 lands on home and does not convert.
        let _s3 = insert_landing(&conn, 4, 130, home, 60, 1, None);

        insert_payment(&conn, "pay_1", s1, 30_000, 200, None);
        insert_payment(&conn, "pay_2", s2, 50_000, 210, None);

        let response =
            build_landing_pages_response(&conn, &baseline_query(0, 10_000)).expect("response");
        assert_eq!(response.pages.len(), 2);

        let pricing_row = &response.pages[0];
        assert_eq!(pricing_row.path, "/pricing");
        assert_eq!(pricing_row.visitors, 1);
        assert_eq!(pricing_row.converting_visitors, 1);
        assert_eq!(pricing_row.attributed_revenue, 50_000.0);
        assert_eq!(pricing_row.conversion_rate, 1.0);
        assert_eq!(pricing_row.bounce_rate, 1.0);
        assert!((pricing_row.reduce_bounces_25pct - 0.25 * 1.0 * 50_000.0).abs() < 1e-6);
        assert!(pricing_row.best_bounce_uplift);
        assert!(pricing_row.highest_bounce_rate);
        assert_eq!(pricing_row.avg_session_duration, 5.0);
        assert_eq!(pricing_row.avg_scroll_percent, 10.0);

        let home_row = &response.pages[1];
        assert_eq!(home_row.path, "/");
        assert_eq!(home_row.visitors, 2);
        assert_eq!(home_row.converting_visitors, 1);
        assert_eq!(home_row.attributed_revenue, 30_000.0);
        assert!((home_row.conversion_rate - 0.5).abs() < 1e-9);
        assert_eq!(home_row.bounce_rate, 0.0);
        assert_eq!(home_row.reduce_bounces_25pct, 0.0);
        assert!(!home_row.best_bounce_uplift);
        assert!(!home_row.highest_bounce_rate);
        assert!((home_row.avg_session_duration - 90.0).abs() < 1e-9);
        // Only the landing pageview scrolls are averaged; s1's pricing pageview is ignored.
        assert_eq!(home_row.avg_scroll_percent, 80.0);
    }

    #[test]
    fn page_segment_filter_narrows_landing_pages_to_sessions_with_page() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/", "acme.test");
        let pricing = insert_path(&conn, "/pricing", "acme.test");
        let docs = insert_path(&conn, "/docs", "acme.test");

        // Session 1 lands on home and also views pricing.
        let s1 = insert_landing(&conn, 1, 100, home, 120, 2, Some(80));
        insert_pageview(&conn, 2, 110, pricing, s1, Some(50));
        // Session 2 lands on docs and never views pricing.
        insert_landing(&conn, 3, 200, docs, 60, 1, Some(40));

        let mut query = baseline_query(0, 10_000);
        query.segments = crate::segment_filter::SegmentFilters {
            countries: Vec::new(),
            sources: Vec::new(),
            pages: vec![pricing],
            landing_pages: Vec::new(),
            events: Vec::new(),
            ..Default::default()
        };
        let response = build_landing_pages_response(&conn, &query).expect("response");
        // Only session 1 contains the pricing pageview, so only its landing
        // page (home) survives; the docs landing is dropped.
        assert_eq!(response.pages.len(), 1);
        assert_eq!(response.pages[0].path, "/");
    }

    #[test]
    fn landing_pages_compute_session_duration_uplift() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/", "acme.test");
        let pricing = insert_path(&conn, "/pricing", "acme.test");

        // Session 1: home (dwell 40) -> pricing (dwell 30), converts $100.
        let s1 = insert_landing_with_duration(&conn, 1, 100, home, 100, 2, Some(80), Some(40));
        insert_pageview_with_duration(&conn, 2, 120, pricing, s1, Some(50), Some(30));
        // Session 2: pricing only, no conversion.
        let _s2 = insert_landing_with_duration(&conn, 3, 130, pricing, 20, 1, Some(10), Some(20));

        insert_payment(&conn, "pay_1", s1, 100_000, 140, Some(70));

        let response =
            build_landing_pages_response(&conn, &baseline_query(0, 10_000)).expect("response");
        let home_row = response
            .pages
            .iter()
            .find(|page| page.path == "/")
            .expect("home row");
        let pricing_row = response
            .pages
            .iter()
            .find(|page| page.path == "/pricing")
            .expect("pricing row");

        let approx = |a: f64, b: f64| assert!((a - b).abs() < 1e-6, "expected {a} ≈ {b}");

        approx(
            home_row.increase_session_25pct,
            100_000.0 * 50.0 / 80.0 - 100_000.0 * 40.0 / 70.0,
        );
        assert!(home_row.best_session_uplift);
        assert_eq!(pricing_row.increase_session_25pct, 0.0);
        assert!(!pricing_row.best_session_uplift);
    }
}
