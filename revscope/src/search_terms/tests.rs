use rusqlite::params;

use crate::db::ProjectConnection;
use crate::test_utils::open_migrated_db;
use crate::test_utils::{
    PaymentFixture, SubscriptionFixture, insert_payment as insert_test_payment,
    insert_subscription as insert_test_subscription, visitor_timestamp_id_for_page_id,
};
use crate::util::NANOS_PER_SECOND;

use super::opportunities::{
    attach_ctr_opportunities, expected_ctr_at, mark_search_term_highlights, rank_ctr_curve,
};
use super::rows::{RawTerm, SearchTermLandingPath};
use super::*;

fn page_id(seed: u8) -> [u8; 16] {
    [seed; 16]
}

fn timestamp_id(seed: u8, viewed_at: i64) -> i64 {
    viewed_at * NANOS_PER_SECOND + i64::from(seed)
}

fn insert_path(conn: &ProjectConnection, path: &str) -> i64 {
    conn.execute(
        "INSERT INTO paths (path, hostname) VALUES (?1, 'acme.test')",
        params![path],
    )
    .expect("insert path");
    conn.query_row(
        "SELECT id FROM paths WHERE path = ?1 AND hostname = 'acme.test'",
        params![path],
        |row| row.get::<_, i64>(0),
    )
    .expect("path id")
}

fn insert_referrer(conn: &ProjectConnection, referrer: &str) -> i64 {
    conn.execute(
        "INSERT INTO referrers (referrer) VALUES (?1)",
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

fn insert_search_landing(
    conn: &ProjectConnection,
    seed: u8,
    viewed_at: i64,
    path_id: i64,
    referrer_id: i64,
) {
    let id = page_id(seed);
    let timestamp_id = timestamp_id(seed, viewed_at);
    conn.execute(
        "INSERT INTO pageviews (
            timestamp_id, visitor_timestamp_id, previous_timestamp_id, path_id,
            duration_seconds, scrolled_percent
         ) VALUES (?1, ?1, NULL, ?2, NULL, NULL)",
        params![timestamp_id, path_id],
    )
    .expect("insert pageview");
    conn.execute(
        "INSERT INTO client_page_ids_map (visitor_timestamp_id, page_id)
         VALUES (?1, ?2)",
        params![timestamp_id, id.as_slice()],
    )
    .expect("insert page id map");
    conn.execute(
        "INSERT INTO landings (
            visitor_timestamp_id, referrer, campaign, referrer_type, is_google, timezone,
            language, country, tracker_version, os, os_version, browser_family, device
         ) VALUES (?1, ?2, NULL, 'search', 1, NULL, NULL, NULL, 1.0, NULL, NULL, NULL, 'desktop')",
        params![timestamp_id, referrer_id],
    )
    .expect("insert landing");
}

fn insert_payment(
    conn: &ProjectConnection,
    seed: u8,
    first_page_seed: u8,
    amount: i64,
    paid_at: i64,
) {
    let provider_payment_id = format!("pay_{seed}");
    let first_page = page_id(first_page_seed);
    let visitor_timestamp_id = visitor_timestamp_id_for_page_id(conn, first_page.as_slice())
        .expect("mapped landing timestamp");
    let mut payment = PaymentFixture::new(provider_payment_id, amount, paid_at);
    payment.visitor_timestamp_id = Some(visitor_timestamp_id);
    insert_test_payment(conn, payment);
}

/// Inserts (or reuses) a `search_queries` row and returns its id.
fn insert_search_query(conn: &ProjectConnection, query: &str) -> i64 {
    conn.execute(
        "INSERT INTO search_queries (query) VALUES (?1)
         ON CONFLICT(query) DO NOTHING",
        params![query],
    )
    .expect("insert search query");
    conn.query_row(
        "SELECT id FROM search_queries WHERE query = ?1",
        params![query],
        |row| row.get::<_, i64>(0),
    )
    .expect("search query id")
}

/// Inserts a daily metrics row for `query` and returns its `query_id`.
/// When `mobile_clicks`/`desktop_clicks` are omitted, all clicks are treated
/// as desktop so mobile share stays defined.
fn insert_search_term(
    conn: &ProjectConnection,
    query: &str,
    position: i64,
    impressions: i64,
    clicks: i64,
    date: i64,
) -> i64 {
    insert_search_term_with_devices(conn, query, position, impressions, clicks, 0, clicks, date)
}

#[allow(clippy::too_many_arguments)]
fn insert_search_term_with_devices(
    conn: &ProjectConnection,
    query: &str,
    position: i64,
    impressions: i64,
    clicks: i64,
    mobile_clicks: i64,
    desktop_clicks: i64,
    date: i64,
) -> i64 {
    let query_id = insert_search_query(conn, query);
    conn.execute(
        "INSERT INTO search_queries_by_date
            (date, query_id, position, impressions, clicks, mobile_clicks, desktop_clicks)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            date,
            query_id,
            position,
            impressions,
            clicks,
            mobile_clicks,
            desktop_clicks
        ],
    )
    .expect("insert search query by date");
    query_id
}

fn insert_search_query_path(
    conn: &ProjectConnection,
    query_id: i64,
    path_id: i64,
    clicks: i64,
    date: i64,
) {
    conn.execute(
        "INSERT INTO search_query_paths_by_date (date, query_id, path_id, clicks)
         VALUES (?1, ?2, ?3, ?4)",
        params![date, query_id, path_id, clicks],
    )
    .expect("insert search query path");
}

fn baseline_query(start: i64, end: i64) -> SearchTermsQuery {
    SearchTermsQuery {
        start,
        end,
        offset: 0,
        order_by: SearchTermsOrderBy::EstimatedRevenue,
        direction: SortDirection::Desc,
        segments: SegmentFilters::default(),
    }
}

fn landing_path(path_id: i64, path: &str, clicks: i64) -> SearchTermLandingPath {
    SearchTermLandingPath {
        path_id,
        path: path.to_string(),
        clicks,
    }
}

#[test]
fn rejects_inverted_range() {
    assert!(
        parse_search_terms_query("/v2/read/projects/1/search-terms?start=200&end=100",).is_err()
    );
}

#[test]
fn parses_offset_and_ordering() {
    let query = parse_search_terms_query(
        "/v2/read/projects/1/search-terms?start=1&end=100&offset=200&order_by=clicks&direction=asc",
    )
    .expect("query");
    assert_eq!(query.offset, 200);
    assert_eq!(query.order_by, SearchTermsOrderBy::Clicks);
    assert_eq!(query.direction, SortDirection::Asc);
}

#[test]
fn rejects_invalid_offset_and_ordering() {
    assert!(
        parse_search_terms_query("/v2/read/projects/1/search-terms?start=1&end=100&offset=-1",)
            .is_err()
    );
    assert!(
        parse_search_terms_query("/v2/read/projects/1/search-terms?start=1&end=100&order_by=bad",)
            .is_err()
    );
    assert!(
        parse_search_terms_query(
            "/v2/read/projects/1/search-terms?start=1&end=100&direction=sideways",
        )
        .is_err()
    );
}

#[test]
fn empty_database_returns_no_terms() {
    let conn = open_migrated_db();
    let response =
        build_search_terms_response(&conn, &baseline_query(0, 10_000)).expect("response");
    assert_eq!(response.current.search_channel_revenue, 0);
    assert_eq!(response.current.summary.impressions, 0);
    assert_eq!(response.current.summary.clicks, 0);
    assert_eq!(response.current.summary.estimated_revenue, 0);
    assert_eq!(response.current.summary.position, 0.0);
    assert_eq!(response.current.summary.ctr, 0.0);
    assert_eq!(response.current.summary.mobile_share, 0.0);
    assert!(response.current.terms.is_empty());
    assert_eq!(response.offset, 0);
    assert_eq!(response.limit, SEARCH_TERMS_PAGE_SIZE);
    assert_eq!(response.next_offset, None);
}

#[test]
fn aggregates_and_estimates_revenue_by_query() {
    const DAY: i64 = 86_400;
    let conn = open_migrated_db();
    let path_id = insert_path(&conn, "/");
    let pricing_path_id = insert_path(&conn, "/pricing");
    let google = insert_referrer(&conn, "google.com/search");

    // Search channel: 4 landings, $100k revenue => $25k per visitor.
    insert_search_landing(&conn, 1, 100, path_id, google);
    insert_search_landing(&conn, 2, 110, path_id, google);
    insert_search_landing(&conn, 3, 120, path_id, google);
    insert_search_landing(&conn, 4, 130, path_id, google);
    insert_payment(&conn, 1, 1, 60_000, 200);
    insert_payment(&conn, 2, 2, 40_000, 210);

    // "shoes": two daily rows summing to 30 clicks, 300 impressions
    // (24 mobile + 6 desktop across the window).
    let shoes = insert_search_term_with_devices(&conn, "shoes", 2, 100, 10, 4, 6, 0);
    insert_search_term_with_devices(&conn, "shoes", 4, 200, 20, 20, 0, DAY);
    insert_search_query_path(&conn, shoes, path_id, 12, 0);
    insert_search_query_path(&conn, shoes, pricing_path_id, 18, DAY);

    // "boots": 5 clicks, all mobile.
    let boots = insert_search_term_with_devices(&conn, "boots", 3, 100, 5, 5, 0, 0);
    insert_search_query_path(&conn, boots, path_id, 5, 0);

    let response =
        build_search_terms_response(&conn, &baseline_query(0, 2 * DAY)).expect("response");
    // Channel revenue is reported as-is; per-term estimates sum back to it.
    assert_eq!(response.current.search_channel_revenue, 100_000);
    assert_eq!(response.current.summary.impressions, 400);
    assert_eq!(response.current.summary.clicks, 35);
    assert_eq!(response.current.summary.estimated_revenue, 100_000);
    assert!((response.current.summary.position - (115.0 / 35.0)).abs() < 1e-9);
    assert!((response.current.summary.ctr - (13.0 / 140.0)).abs() < 1e-9);
    assert!((response.current.summary.mobile_share - (29.0 / 35.0)).abs() < 1e-9);
    assert_eq!(response.current.terms.len(), 2);

    // Revenue is split by click share over 35 total windowed clicks.
    // shoes: 100_000 * 30 / 35 = 85_714; boots: 100_000 * 5 / 35 = 14_286.
    let shoes = response
        .current
        .terms
        .iter()
        .find(|term| term.query == "shoes")
        .expect("shoes");
    assert_eq!(
        shoes.landing_paths,
        vec![
            landing_path(pricing_path_id, "/pricing", 18),
            landing_path(path_id, "/", 12),
        ]
    );
    assert_eq!(shoes.clicks, 30);
    assert_eq!(shoes.impressions, 300);
    assert_eq!(shoes.estimated_revenue, 85_714);
    // impression-weighted position: (2*100 + 4*200) / 300 = 1000/300.
    assert!((shoes.position - (1_000.0 / 300.0)).abs() < 1e-9);
    // mobile_clicks = 4 + 20 = 24; detail_clicks = 24 + 6 = 30.
    assert_eq!(shoes.mobile_clicks, 24);
    assert_eq!(shoes.detail_clicks, 30);

    let boots = response
        .current
        .terms
        .iter()
        .find(|term| term.query == "boots")
        .expect("boots");
    assert_eq!(boots.landing_paths, vec![landing_path(path_id, "/", 5)]);
    assert_eq!(boots.clicks, 5);
    assert_eq!(boots.estimated_revenue, 14_286);
    assert_eq!(boots.mobile_clicks, 5);
    assert_eq!(boots.detail_clicks, 5);

    // Per-term estimates sum back to the reported channel revenue.
    assert_eq!(
        shoes.estimated_revenue + boots.estimated_revenue,
        response.current.search_channel_revenue
    );
}

#[test]
fn excludes_rows_outside_window() {
    const DAY: i64 = 86_400;
    let conn = open_migrated_db();
    let path_id = insert_path(&conn, "/");
    let google = insert_referrer(&conn, "google.com/search");
    insert_search_landing(&conn, 1, 2 * DAY + 150, path_id, google);
    insert_payment(&conn, 1, 1, 10_000, 2 * DAY + 200);

    // Search Console stores one row per UTC date, so an in-window and an
    // out-of-window row are separate days.
    insert_search_term(&conn, "shoes", 2, 100, 10, 2 * DAY);
    insert_search_term(&conn, "shoes", 2, 999, 999, 0);

    let response =
        build_search_terms_response(&conn, &baseline_query(2 * DAY, 3 * DAY)).expect("response");
    assert_eq!(response.current.search_channel_revenue, 10_000);
    assert_eq!(response.current.terms.len(), 1);
    let shoes = &response.current.terms[0];
    assert_eq!(shoes.clicks, 10);
    assert_eq!(shoes.impressions, 100);
    // Only query in window, so it gets all the channel revenue.
    assert_eq!(shoes.estimated_revenue, 10_000);
}

/// A range picked in a non-UTC zone starts and ends mid-UTC-day. The daily
/// Search Console rows cannot be split, so every UTC day the range touches
/// contributes rather than a partial day being silently dropped.
#[test]
fn includes_every_utc_day_the_window_overlaps() {
    const DAY: i64 = 86_400;
    const HOUR: i64 = 3_600;
    let conn = open_migrated_db();

    // One calendar day as seen from UTC-5: 2 days in, 05:00Z to 05:00Z.
    let start = 2 * DAY + 5 * HOUR;
    let end = start + DAY;
    insert_search_term(&conn, "shoes", 2, 100, 10, 2 * DAY);
    insert_search_term(&conn, "shoes", 2, 100, 7, 3 * DAY);
    // The day before and the day after the overlapped pair stay out.
    insert_search_term(&conn, "shoes", 2, 100, 99, DAY);
    insert_search_term(&conn, "shoes", 2, 100, 99, 4 * DAY);

    let response =
        build_search_terms_response(&conn, &baseline_query(start, end)).expect("response");
    assert_eq!(response.current.terms.len(), 1);
    assert_eq!(response.current.terms[0].clicks, 17);
}

#[test]
fn no_search_revenue_yields_zero_estimates() {
    let conn = open_migrated_db();
    insert_search_term(&conn, "shoes", 2, 100, 10, 0);
    let response =
        build_search_terms_response(&conn, &baseline_query(0, 10_000)).expect("response");
    assert_eq!(response.current.search_channel_revenue, 0);
    assert_eq!(response.current.terms.len(), 1);
    assert_eq!(response.current.terms[0].estimated_revenue, 0);
}

#[test]
fn estimated_revenue_excludes_subscription_renewals() {
    let conn = open_migrated_db();
    let path_id = insert_path(&conn, "/");
    let google = insert_referrer(&conn, "google.com/search");
    insert_search_landing(&conn, 1, 100, path_id, google);
    let session_id = timestamp_id(1, 100);
    let mut subscription = SubscriptionFixture::new("sub_1");
    subscription.provider_customer_id = Some("cus_1".to_string());
    subscription.customer_email = Some("sub@example.com".to_string());
    subscription.current_period_start = Some(100);
    subscription.current_period_end = Some(200);
    subscription.started_at = Some(100);
    let subscription_id = insert_test_subscription(&conn, subscription);
    let mut first_payment = PaymentFixture::new("pay_sub_new", 50_000, 200);
    first_payment.subscription_id = Some(subscription_id);
    first_payment.interval_unit = Some("month".to_string());
    first_payment.visitor_timestamp_id = Some(session_id);
    first_payment.payment_type = "subscription".to_string();
    first_payment.mrr_amount = Some(50_000);
    insert_test_payment(&conn, first_payment);
    let mut renewal = PaymentFixture::new("pay_sub_renewal", 50_000, 300);
    renewal.subscription_id = Some(subscription_id);
    renewal.interval_unit = Some("month".to_string());
    renewal.visitor_timestamp_id = Some(session_id);
    renewal.payment_type = "subscription".to_string();
    renewal.mrr_amount = Some(50_000);
    insert_test_payment(&conn, renewal);
    insert_search_term(&conn, "shoes", 2, 100, 10, 100);

    let response =
        build_search_terms_response(&conn, &baseline_query(0, 10_000)).expect("response");
    assert_eq!(response.current.search_channel_revenue, 50_000);
    assert_eq!(response.current.terms.len(), 1);
    assert_eq!(response.current.terms[0].estimated_revenue, 50_000);
}

#[test]
fn change_compares_prior_thirty_days() {
    let conn = open_migrated_db();
    let path_id = insert_path(&conn, "/");
    let google = insert_referrer(&conn, "google.com/search");

    // Current 30-day window [DAY*30, DAY*60): "shoes" with 20 clicks.
    const DAY: i64 = 86_400;
    insert_search_landing(&conn, 2, DAY * 40, path_id, google);
    insert_payment(&conn, 2, 2, 2_000, DAY * 41);
    insert_search_term(&conn, "shoes", 3, 100, 20, DAY * 40);

    // Prior 30-day window [DAY*0, DAY*30): same query with 10 clicks.
    insert_search_landing(&conn, 1, DAY * 10, path_id, google);
    insert_payment(&conn, 1, 1, 1_000, DAY * 11);
    insert_search_term(&conn, "shoes", 5, 50, 10, DAY * 10);

    // A 30-day current window so the normalization factor is exactly 1.0
    // when `now` is pinned to the window end.
    let query = baseline_query(DAY * 30, DAY * 60);
    let response = build_search_terms_response_at(&conn, &query, DAY * 60).expect("response");
    assert_eq!(response.current.search_channel_revenue, 2_000);
    assert_eq!(response.current.terms.len(), 1);
    let shoes = &response.current.terms[0];
    assert_eq!(shoes.query, "shoes");
    assert_eq!(shoes.clicks, 20);
    // Clicks doubled vs the prior 30 days (10 -> 20, factor 1.0): +100%.
    assert!((shoes.clicks_change.ratio.expect("ratio") - 1.0).abs() < 1e-9);
    // Revenue doubled too (1_000 -> 2_000).
    assert!((shoes.estimated_revenue_change.ratio.expect("ratio") - 1.0).abs() < 1e-9);
    // Summary mirrors the single term's clicks change.
    assert!((response.current.summary.clicks_change.ratio.expect("ratio") - 1.0).abs() < 1e-9);
}

#[test]
fn previous_summary_includes_terms_absent_from_page() {
    let conn = open_migrated_db();
    let path_id = insert_path(&conn, "/");
    let google = insert_referrer(&conn, "google.com/search");
    const DAY: i64 = 86_400;

    // Current window: only "shoes".
    insert_search_landing(&conn, 2, DAY * 40, path_id, google);
    insert_payment(&conn, 2, 2, 2_000, DAY * 41);
    insert_search_term(&conn, "shoes", 3, 100, 20, DAY * 40);

    // Prior window: "shoes" plus an extra term that will not appear on the
    // current page — summary clicks must still count both.
    insert_search_landing(&conn, 1, DAY * 10, path_id, google);
    insert_payment(&conn, 1, 1, 1_000, DAY * 11);
    insert_search_term(&conn, "shoes", 5, 50, 10, DAY * 10);
    insert_search_term(&conn, "boots", 4, 80, 10, DAY * 10);

    let query = baseline_query(DAY * 30, DAY * 60);
    let response = build_search_terms_response_at(&conn, &query, DAY * 60).expect("response");
    assert_eq!(response.current.terms.len(), 1);
    assert_eq!(response.current.terms[0].query, "shoes");
    // Prior summary clicks = 10 (shoes) + 10 (boots) = 20; current = 20.
    // With factor 1.0 the summary clicks change ratio is 0.
    assert!((response.current.summary.clicks_change.ratio.expect("ratio") - 0.0).abs() < 1e-9);
    // Per-term shoes still compares against prior shoes only (10 -> 20).
    assert!(
        (response.current.terms[0]
            .clicks_change
            .ratio
            .expect("ratio")
            - 1.0)
            .abs()
            < 1e-9
    );
}

#[test]
fn change_absent_without_prior_match() {
    let conn = open_migrated_db();
    insert_search_term(&conn, "shoes", 3, 100, 10, 0);
    let response =
        build_search_terms_response(&conn, &baseline_query(0, 10_000)).expect("response");
    let shoes = &response.current.terms[0];
    // No matching prior-period query: no comparison is available (the hover
    // omits the delta).
    assert!(shoes.clicks_change.ratio.is_none());
}

#[test]
fn paginates_search_terms_with_limit_and_offset() {
    let conn = open_migrated_db();
    let total = (SEARCH_TERMS_PAGE_SIZE as usize) + 1;
    for index in 0..total {
        // Distinct click counts so default (estimated-revenue ≡ clicks) order
        // is deterministic: highest clicks first.
        let clicks = (total - index) as i64;
        insert_search_term(&conn, &format!("term-{index:03}"), 1, 100, clicks, 0);
    }

    let mut query = baseline_query(0, 10_000);
    let first_page = build_search_terms_response(&conn, &query).expect("first page");
    assert_eq!(
        first_page.current.terms.len(),
        SEARCH_TERMS_PAGE_SIZE as usize
    );
    assert_eq!(first_page.offset, 0);
    assert_eq!(first_page.limit, SEARCH_TERMS_PAGE_SIZE);
    assert_eq!(first_page.next_offset, Some(SEARCH_TERMS_PAGE_SIZE));
    // Highlighted CTR-gap rows are promoted to the front. Lowest clicks
    // (worst CTR at rank 1) are term-100, term-099, term-098.
    assert_eq!(first_page.current.terms[0].query, "term-100");
    assert!(first_page.current.terms[0].highest_lost_clicks);
    assert_eq!(
        first_page.current.terms.last().expect("last").query,
        "term-096"
    );
    // Summary still covers the full window, not just the page.
    assert_eq!(
        first_page.current.summary.clicks,
        (1..=total as i64).sum::<i64>()
    );
    let first_flagged: Vec<&str> = first_page
        .current
        .terms
        .iter()
        .filter(|term| term.highest_lost_clicks)
        .map(|term| term.query.as_str())
        .collect();
    assert_eq!(first_flagged, vec!["term-100", "term-099", "term-098"]);

    query.offset = SEARCH_TERMS_PAGE_SIZE;
    let second_page = build_search_terms_response(&conn, &query).expect("second page");
    assert_eq!(second_page.current.terms.len(), 1);
    assert_eq!(second_page.next_offset, None);
    assert_eq!(second_page.current.terms[0].query, "term-097");
    assert!(!second_page.current.terms[0].highest_lost_clicks);
}

#[test]
fn sorts_by_ctr() {
    let conn = open_migrated_db();
    let home = insert_path(&conn, "/");
    // CTR: alpha 0.5, bravo 0.1, charlie 0.2 — desc => alpha, charlie, bravo.
    let alpha = insert_search_term(&conn, "alpha", 1, 10, 5, 0);
    insert_search_query_path(&conn, alpha, home, 5, 0);
    insert_search_term(&conn, "bravo", 1, 100, 10, 0);
    insert_search_term(&conn, "charlie", 1, 50, 10, 0);

    let mut query = baseline_query(0, 10_000);
    query.order_by = SearchTermsOrderBy::Ctr;
    query.direction = SortDirection::Desc;
    let response = build_search_terms_response(&conn, &query).expect("response");
    let queries: Vec<_> = response
        .current
        .terms
        .iter()
        .map(|term| term.query.as_str())
        .collect();
    assert_eq!(queries, vec!["alpha", "charlie", "bravo"]);
    assert_eq!(
        response.current.terms[0].landing_paths,
        vec![landing_path(home, "/", 5)]
    );
    assert!(response.current.terms[1].landing_paths.is_empty());
}

#[test]
fn estimated_revenue_order_matches_clicks_order() {
    let conn = open_migrated_db();
    insert_search_term(&conn, "alpha", 1, 100, 5, 0);
    insert_search_term(&conn, "bravo", 1, 100, 20, 0);
    insert_search_term(&conn, "charlie", 1, 100, 10, 0);

    let mut by_revenue = baseline_query(0, 10_000);
    by_revenue.order_by = SearchTermsOrderBy::EstimatedRevenue;
    by_revenue.direction = SortDirection::Desc;
    let revenue_page = build_search_terms_response(&conn, &by_revenue).expect("revenue order");

    let mut by_clicks = baseline_query(0, 10_000);
    by_clicks.order_by = SearchTermsOrderBy::Clicks;
    by_clicks.direction = SortDirection::Desc;
    let clicks_page = build_search_terms_response(&conn, &by_clicks).expect("clicks order");

    let revenue_queries: Vec<_> = revenue_page
        .current
        .terms
        .iter()
        .map(|term| term.query.as_str())
        .collect();
    let clicks_queries: Vec<_> = clicks_page
        .current
        .terms
        .iter()
        .map(|term| term.query.as_str())
        .collect();
    assert_eq!(revenue_queries, clicks_queries);
    // Alpha and charlie sit below the rank-1 CTR curve, so they are
    // promoted ahead of bravo on the default clicks/revenue ranking.
    assert_eq!(revenue_queries, vec!["alpha", "charlie", "bravo"]);
}

#[test]
fn returns_all_landing_paths_ordered_by_clicks() {
    let conn = open_migrated_db();
    let paths = [
        insert_path(&conn, "/a"),
        insert_path(&conn, "/b"),
        insert_path(&conn, "/c"),
        insert_path(&conn, "/d"),
        insert_path(&conn, "/e"),
        insert_path(&conn, "/f"),
    ];
    let shoes = insert_search_term(&conn, "shoes", 1, 100, 10, 0);
    insert_search_query_path(&conn, shoes, paths[0], 10, 0);
    insert_search_query_path(&conn, shoes, paths[1], 9, 0);
    insert_search_query_path(&conn, shoes, paths[2], 8, 0);
    insert_search_query_path(&conn, shoes, paths[3], 7, 0);
    insert_search_query_path(&conn, shoes, paths[4], 6, 0);
    insert_search_query_path(&conn, shoes, paths[5], 1, 0);

    let response =
        build_search_terms_response(&conn, &baseline_query(0, 10_000)).expect("response");
    assert_eq!(response.current.terms.len(), 1);
    assert_eq!(
        response.current.terms[0].landing_paths,
        vec![
            landing_path(paths[0], "/a", 10),
            landing_path(paths[1], "/b", 9),
            landing_path(paths[2], "/c", 8),
            landing_path(paths[3], "/d", 7),
            landing_path(paths[4], "/e", 6),
            landing_path(paths[5], "/f", 1),
        ]
    );
}

#[test]
fn sorts_by_landing_paths_and_still_returns_paths() {
    let conn = open_migrated_db();
    let home = insert_path(&conn, "/");
    let pricing = insert_path(&conn, "/pricing");
    let docs = insert_path(&conn, "/docs");

    let alpha = insert_search_term(&conn, "alpha", 1, 100, 1, 0);
    insert_search_query_path(&conn, alpha, pricing, 1, 0);

    let bravo = insert_search_term(&conn, "bravo", 1, 100, 1, 0);
    insert_search_query_path(&conn, bravo, home, 1, 0);

    let charlie = insert_search_term(&conn, "charlie", 1, 100, 1, 0);
    insert_search_query_path(&conn, charlie, docs, 1, 0);

    let mut query = baseline_query(0, 10_000);
    query.order_by = SearchTermsOrderBy::LandingPaths;
    query.direction = SortDirection::Asc;
    let response = build_search_terms_response(&conn, &query).expect("response");
    let queries: Vec<_> = response
        .current
        .terms
        .iter()
        .map(|term| term.query.as_str())
        .collect();
    // "/" < "/docs" < "/pricing"
    assert_eq!(queries, vec!["bravo", "charlie", "alpha"]);
    assert_eq!(
        response.current.terms[0].landing_paths,
        vec![landing_path(home, "/", 1)]
    );
    assert_eq!(
        response.current.terms[1].landing_paths,
        vec![landing_path(docs, "/docs", 1)]
    );
    assert_eq!(
        response.current.terms[2].landing_paths,
        vec![landing_path(pricing, "/pricing", 1)]
    );
}

#[test]
fn landing_paths_respect_date_window() {
    const DAY: i64 = 86_400;
    let conn = open_migrated_db();
    let home = insert_path(&conn, "/");
    let pricing = insert_path(&conn, "/pricing");
    let docs = insert_path(&conn, "/docs");

    // In-window clicks: /pricing dominates. Out-of-window /docs would win
    // if paths were all-time aggregates.
    let shoes = insert_search_term(&conn, "shoes", 1, 100, 10, 2 * DAY);
    insert_search_query_path(&conn, shoes, pricing, 8, 2 * DAY);
    insert_search_query_path(&conn, shoes, home, 2, 2 * DAY);
    insert_search_query_path(&conn, shoes, docs, 50, 0);

    let response =
        build_search_terms_response(&conn, &baseline_query(2 * DAY, 3 * DAY)).expect("response");
    assert_eq!(response.current.terms.len(), 1);
    assert_eq!(
        response.current.terms[0].landing_paths,
        vec![
            landing_path(pricing, "/pricing", 8),
            landing_path(home, "/", 2),
        ]
    );
}

#[test]
fn landing_paths_sum_clicks_across_days_in_window() {
    const DAY: i64 = 86_400;
    let conn = open_migrated_db();
    let home = insert_path(&conn, "/");
    let pricing = insert_path(&conn, "/pricing");

    let shoes = insert_search_term(&conn, "shoes", 1, 100, 10, 0);
    insert_search_term(&conn, "shoes", 1, 100, 10, DAY);
    // Across two in-window days home totals 12 and pricing 8.
    insert_search_query_path(&conn, shoes, home, 5, 0);
    insert_search_query_path(&conn, shoes, pricing, 6, 0);
    insert_search_query_path(&conn, shoes, home, 7, DAY);
    insert_search_query_path(&conn, shoes, pricing, 2, DAY);

    let response =
        build_search_terms_response(&conn, &baseline_query(0, 2 * DAY)).expect("response");
    assert_eq!(
        response.current.terms[0].landing_paths,
        vec![
            landing_path(home, "/", 12),
            landing_path(pricing, "/pricing", 8),
        ]
    );
}

fn raw_term(query: &str, position: f64, impressions: i64, clicks: i64) -> RawTerm {
    RawTerm {
        query_id: 0,
        query: query.to_string(),
        landing_paths: Vec::new(),
        position,
        impressions,
        clicks,
        mobile_clicks: 0,
        detail_clicks: clicks,
        estimated_revenue: 0,
        expected_ctr: 0.0,
        lost_clicks: 0.0,
        lost_revenue: 0.0,
        cannibalized_clicks: 0,
        highest_lost_clicks: false,
        highest_cannibalization: false,
    }
}

#[test]
fn expected_ctr_interpolates_adjacent_rank_buckets() {
    let terms = vec![raw_term("r1", 1.0, 100, 40), raw_term("r2", 2.0, 100, 20)];
    let curve = rank_ctr_curve(&terms);
    assert!((curve[1].expect("rank 1") - 0.4).abs() < 1e-9);
    assert!((curve[2].expect("rank 2") - 0.2).abs() < 1e-9);
    assert!((expected_ctr_at(1.0, &curve).expect("pos 1") - 0.4).abs() < 1e-9);
    assert!((expected_ctr_at(1.5, &curve).expect("pos 1.5") - 0.3).abs() < 1e-9);
    assert!(expected_ctr_at(11.0, &curve).is_none());
    assert!(expected_ctr_at(0.5, &curve).is_none());
}

#[test]
fn lost_clicks_are_impressions_times_gap_vs_rank_curve() {
    let mut terms = vec![
        raw_term("good", 1.0, 1_000, 400),
        raw_term("leak", 1.0, 1_000, 100),
        raw_term("page-two", 12.0, 10_000, 50),
        // High-impression zero-click query must not pull the rank-1 curve to 0.
        raw_term("unclicked", 1.0, 10_000, 0),
    ];
    attach_ctr_opportunities(&mut terms, 50_000, 500);
    // Expected rank-1 CTR = 500/2000 = 0.25 (unclicked excluded).
    assert!((terms[0].expected_ctr - 0.25).abs() < 1e-9);
    assert!(terms[0].lost_clicks.abs() < 1e-9);
    assert!((terms[1].lost_clicks - 150.0).abs() < 1e-9);
    // 50_000 / 500 clicks = 100 thousandths per click; 150 lost × 100 = 15_000.
    assert!((terms[1].lost_revenue - 15_000.0).abs() < 1e-9);
    assert_eq!(terms[2].expected_ctr, 0.0);
    assert_eq!(terms[2].lost_clicks, 0.0);
    assert!((terms[3].expected_ctr - 0.25).abs() < 1e-9);
    assert!((terms[3].lost_clicks - 2_500.0).abs() < 1e-9);
    mark_search_term_highlights(&mut terms);
    assert!(terms[1].highest_lost_clicks);
    assert!(!terms[3].highest_lost_clicks);
}

#[test]
fn flags_three_highest_lost_clicks_not_one_per_rank() {
    let conn = open_migrated_db();
    // Rank 1: three large leaks plus one strong query. Rank 2 has a smaller
    // leak that would win a per-rank highlight but not the global top 3.
    insert_search_term(&conn, "r1-a", 1, 5_000, 200, 0);
    insert_search_term(&conn, "r1-b", 1, 4_000, 200, 0);
    insert_search_term(&conn, "r1-c", 1, 3_000, 150, 0);
    insert_search_term(&conn, "r1-good", 1, 2_000, 800, 0);
    insert_search_term(&conn, "r2-leak", 2, 1_000, 20, 0);
    insert_search_term(&conn, "r2-good", 2, 1_000, 200, 0);
    insert_search_term(&conn, "deep", 12, 10_000, 50, 0);

    let response =
        build_search_terms_response(&conn, &baseline_query(0, 10_000)).expect("response");
    let flagged: Vec<&str> = response
        .current
        .terms
        .iter()
        .filter(|term| term.highest_lost_clicks)
        .map(|term| term.query.as_str())
        .collect();
    assert_eq!(flagged, vec!["r1-a", "r1-b", "r1-c"]);
    let r2 = response
        .current
        .terms
        .iter()
        .find(|term| term.query == "r2-leak")
        .expect("r2-leak");
    assert!(r2.lost_clicks > 0.0);
    assert!(!r2.highest_lost_clicks);
    let deep = response
        .current
        .terms
        .iter()
        .find(|term| term.query == "deep")
        .expect("deep");
    assert_eq!(deep.lost_clicks, 0.0);
    assert!(!deep.highest_lost_clicks);
}

#[test]
fn flags_three_most_cannibalized_queries() {
    let conn = open_migrated_db();
    let home = insert_path(&conn, "/");
    let pricing = insert_path(&conn, "/pricing");
    let docs = insert_path(&conn, "/docs");

    let q1 = insert_search_term(&conn, "q1", 1, 200, 150, 0);
    insert_search_query_path(&conn, q1, home, 100, 0);
    insert_search_query_path(&conn, q1, pricing, 50, 0);

    let q2 = insert_search_term(&conn, "q2", 1, 200, 140, 0);
    insert_search_query_path(&conn, q2, home, 80, 0);
    insert_search_query_path(&conn, q2, pricing, 40, 0);
    insert_search_query_path(&conn, q2, docs, 20, 0);

    let q3 = insert_search_term(&conn, "q3", 1, 100, 40, 0);
    insert_search_query_path(&conn, q3, home, 30, 0);
    insert_search_query_path(&conn, q3, pricing, 10, 0);

    let q4 = insert_search_term(&conn, "q4", 1, 200, 135, 0);
    insert_search_query_path(&conn, q4, home, 90, 0);
    insert_search_query_path(&conn, q4, pricing, 45, 0);

    let q5 = insert_search_term(&conn, "q5", 1, 150, 95, 0);
    insert_search_query_path(&conn, q5, home, 70, 0);
    insert_search_query_path(&conn, q5, pricing, 25, 0);

    let q6 = insert_search_term(&conn, "q6", 1, 200, 200, 0);
    insert_search_query_path(&conn, q6, home, 200, 0);

    let mut query = baseline_query(0, 10_000);
    query.order_by = SearchTermsOrderBy::Query;
    query.direction = SortDirection::Asc;
    let response = build_search_terms_response(&conn, &query).expect("response");
    let flagged: Vec<&str> = response
        .current
        .terms
        .iter()
        .filter(|term| term.highest_cannibalization)
        .map(|term| term.query.as_str())
        .collect();
    // q2=60, q1=50, q4=45 beat q5=25, q3=10, q6=0.
    assert_eq!(flagged, vec!["q1", "q2", "q4"]);
}
