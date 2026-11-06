use super::rows::{CITIES_PAGE_SIZE, CitiesOrderBy};
use super::*;
use crate::comparison::Change;
use crate::query_params::SortDirection;
use crate::test_utils::open_migrated_db;
use rusqlite::params;

use crate::db::ProjectConnection;

use crate::test_utils::{
    PaymentFixture, SubscriptionFixture, insert_payment as insert_test_payment,
    insert_subscription as insert_test_subscription, visitor_timestamp_id_for_page_id,
};

fn page_id(seed: u8) -> [u8; 16] {
    [seed; 16]
}

fn timestamp_id(seed: u8, viewed_at: i64) -> i64 {
    viewed_at * NANOS_PER_SECOND + i64::from(seed)
}

fn insert_path(conn: &ProjectConnection) -> i64 {
    conn.execute(
        "INSERT INTO paths (path, hostname) VALUES ('/', 'acme.test')",
        [],
    )
    .expect("insert path");
    conn.query_row("SELECT id FROM paths LIMIT 1", [], |row| {
        row.get::<_, i64>(0)
    })
    .expect("path id")
}

#[allow(clippy::too_many_arguments)]
fn insert_first_visit(
    conn: &ProjectConnection,
    seed: u8,
    viewed_at: i64,
    path_id: i64,
    country: Option<&str>,
    browser_family: Option<&str>,
    os: Option<&str>,
    device: &str,
) {
    insert_first_visit_with_dimensions(
        conn,
        seed,
        viewed_at,
        path_id,
        country,
        None,
        browser_family,
        os,
        device,
        0,
        0,
    );
}

#[allow(clippy::too_many_arguments)]
fn insert_first_visit_with_viewport(
    conn: &ProjectConnection,
    seed: u8,
    viewed_at: i64,
    path_id: i64,
    country: Option<&str>,
    browser_family: Option<&str>,
    os: Option<&str>,
    device: &str,
    viewport_width: i64,
) {
    insert_first_visit_with_dimensions(
        conn,
        seed,
        viewed_at,
        path_id,
        country,
        None,
        browser_family,
        os,
        device,
        0,
        viewport_width,
    );
}

#[allow(clippy::too_many_arguments)]
fn insert_first_visit_with_dimensions(
    conn: &ProjectConnection,
    seed: u8,
    viewed_at: i64,
    path_id: i64,
    country: Option<&str>,
    city: Option<&str>,
    browser_family: Option<&str>,
    os: Option<&str>,
    device: &str,
    screen_width: i64,
    viewport_width: i64,
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
            language, country, city, tracker_version, os, os_version, browser_family, device,
            screen_width, viewport_width
         ) VALUES (?1, NULL, NULL, NULL, 0, NULL, NULL, ?2, ?3, 1.0, ?4, NULL, ?5, ?6, ?7, ?8)",
        params![
            timestamp_id,
            country,
            city,
            os,
            browser_family,
            device,
            screen_width,
            viewport_width
        ],
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

fn baseline_query(start: i64, end: i64) -> AudienceQuery {
    AudienceQuery {
        start,
        end,
        offset: 0,
        order_by: CitiesOrderBy::Revenue,
        direction: SortDirection::Desc,
        segments: SegmentFilters::default(),
    }
}

#[test]
fn rejects_inverted_range() {
    assert!(parse_audience_query("/v2/read/projects/1/audience?start=200&end=100",).is_err());
}

#[test]
fn parses_offset_and_ordering() {
    let query = parse_audience_query(
        "/v2/read/projects/1/audience?start=1&end=100&offset=200&order_by=visitors&direction=asc",
    )
    .expect("query");
    assert_eq!(query.offset, 200);
    assert_eq!(query.order_by, CitiesOrderBy::Visitors);
    assert_eq!(query.direction, SortDirection::Asc);
}

#[test]
fn rejects_invalid_offset_and_ordering() {
    assert!(
        parse_audience_query("/v2/read/projects/1/audience?start=1&end=100&offset=-1",).is_err()
    );
    assert!(
        parse_audience_query("/v2/read/projects/1/audience?start=1&end=100&order_by=bad",).is_err()
    );
    assert!(
        parse_audience_query("/v2/read/projects/1/audience?start=1&end=100&direction=sideways",)
            .is_err()
    );
}

#[test]
fn empty_database_returns_no_countries() {
    let conn = open_migrated_db();
    let response = build_audience_response(&conn, &baseline_query(0, 10_000)).expect("response");
    assert!(response.countries.current.is_empty());
    assert!(response.cities.current.is_empty());
    assert_eq!(response.cities.offset, 0);
    assert_eq!(response.cities.limit, CITIES_PAGE_SIZE);
    assert_eq!(response.cities.next_offset, None);
    assert!(response.browsers.current.is_empty());
    assert!(response.os.current.is_empty());
    assert!(response.devices.current.is_empty());
    assert!(response.screen_sizes.current.is_empty());
    assert!(response.viewports.current.is_empty());
}

#[test]
fn revenue_excludes_subscription_renewals() {
    let conn = open_migrated_db();
    let path_id = insert_path(&conn);
    insert_first_visit(&conn, 1, 100, path_id, Some("US"), None, None, "desktop");
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

    let response = build_audience_response(&conn, &baseline_query(0, 10_000)).expect("response");
    let current = &response.countries.current;
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].country.as_deref(), Some("US"));
    assert_eq!(current[0].metrics.revenue, 50_000);
    assert_eq!(current[0].metrics.converting_visitors, 1);
}

#[test]
fn groups_by_country_ranked_by_revenue() {
    let conn = open_migrated_db();
    let path_id = insert_path(&conn);
    // US: 3 visitors, 1 converting, $40k.
    insert_first_visit(&conn, 1, 100, path_id, Some("US"), None, None, "desktop");
    insert_first_visit(&conn, 2, 110, path_id, Some("US"), None, None, "desktop");
    insert_first_visit(&conn, 3, 120, path_id, Some("US"), None, None, "desktop");
    // GB: 2 visitors, 2 converting, $30k total.
    insert_first_visit(&conn, 4, 130, path_id, Some("GB"), None, None, "desktop");
    insert_first_visit(&conn, 5, 140, path_id, Some("GB"), None, None, "desktop");
    // No country: 1 visitor, no revenue.
    insert_first_visit(&conn, 6, 150, path_id, None, None, None, "desktop");

    insert_payment(&conn, 1, 1, 40_000, 200);
    insert_payment(&conn, 2, 4, 20_000, 210);
    insert_payment(&conn, 3, 5, 10_000, 220);

    let response = build_audience_response(&conn, &baseline_query(0, 10_000)).expect("response");
    let current = &response.countries.current;
    assert_eq!(current.len(), 3);

    // US ranks first by revenue.
    assert_eq!(current[0].country.as_deref(), Some("US"));
    assert_eq!(current[0].country_name, "United States");
    assert_eq!(current[0].metrics.visitors, 3);
    assert_eq!(current[0].metrics.converting_visitors, 1);
    assert_eq!(current[0].metrics.revenue, 40_000);
    assert!((current[0].conversion_rate - (1.0 / 3.0)).abs() < 1e-9);

    // GB second.
    assert_eq!(current[1].country.as_deref(), Some("GB"));
    assert_eq!(current[1].country_name, "United Kingdom");
    assert_eq!(current[1].metrics.visitors, 2);
    assert_eq!(current[1].metrics.converting_visitors, 2);
    assert_eq!(current[1].metrics.revenue, 30_000);
    assert_eq!(current[1].conversion_rate, 1.0);

    // Unknown country last, no revenue.
    assert_eq!(current[2].country, None);
    assert_eq!(current[2].country_name, "Unknown");
    assert_eq!(current[2].metrics.visitors, 1);
    assert_eq!(current[2].metrics.converting_visitors, 0);
    assert_eq!(current[2].metrics.revenue, 0);
    assert_eq!(current[2].conversion_rate, 0.0);

    // The whole fixture lives inside [0, 10_000), so the comparison window
    // (the prior 30 days) has no data at all: every row reports no comparison.
    assert_eq!(current[0].metrics.revenue_change, Change::none());
    assert_eq!(current[0].metrics.visitors_change, Change::none());
    assert_eq!(current[0].metrics.conversions_change, Change::none());
}

#[test]
fn attribute_filters_narrow_every_breakdown_but_their_own() {
    let conn = open_migrated_db();
    let path_id = insert_path(&conn);
    for (seed, country, city, browser, device) in [
        (1, "US", "New York", "chrome", "desktop"),
        (2, "GB", "London", "firefox", "mobile"),
        (3, "US", "Boston", "safari", "mobile"),
    ] {
        insert_first_visit_with_dimensions(
            &conn,
            seed,
            100 + i64::from(seed),
            path_id,
            Some(country),
            Some(city),
            Some(browser),
            None,
            device,
            0,
            0,
        );
    }

    let mut query = baseline_query(0, 10_000);
    query.segments = parse_segment_filters("?device=mobile&city=US-Boston");
    let response = build_audience_response(&conn, &query).expect("response");

    let devices: Vec<&str> = response
        .devices
        .current
        .iter()
        .map(|row| row.device.as_str())
        .collect();
    assert_eq!(devices, vec!["mobile"], "the city filter still applies");
    let mut cities: Vec<&str> = response
        .cities
        .current
        .iter()
        .map(|row| row.city.as_str())
        .collect();
    cities.sort_unstable();
    assert_eq!(cities, vec!["Boston", "London"]);
    let browsers: Vec<&str> = response
        .browsers
        .current
        .iter()
        .map(|row| row.browser_family.as_str())
        .collect();
    assert_eq!(browsers, vec!["safari"]);

    query.segments = parse_segment_filters("?device=mobile");
    let response = build_audience_response(&conn, &query).expect("response");
    let mut devices: Vec<&str> = response
        .devices
        .current
        .iter()
        .map(|row| row.device.as_str())
        .collect();
    devices.sort_unstable();
    assert_eq!(devices, vec!["desktop", "mobile"]);
    assert_eq!(response.countries.current.len(), 2);
}

#[test]
fn country_segment_filter_does_not_narrow_country_breakdown() {
    let conn = open_migrated_db();
    let path_id = insert_path(&conn);
    insert_first_visit_with_dimensions(
        &conn,
        1,
        100,
        path_id,
        Some("US"),
        Some("New York"),
        Some("chrome"),
        None,
        "desktop",
        0,
        0,
    );
    insert_first_visit_with_dimensions(
        &conn,
        2,
        110,
        path_id,
        Some("GB"),
        Some("London"),
        Some("firefox"),
        None,
        "mobile",
        0,
        0,
    );

    let mut query = baseline_query(0, 10_000);
    query.segments = SegmentFilters::from_country_codes(["US"]);
    let response = build_audience_response(&conn, &query).expect("response");

    let countries: Vec<_> = response
        .countries
        .current
        .iter()
        .map(|row| row.country.as_deref())
        .collect();
    assert!(countries.contains(&Some("US")));
    assert!(countries.contains(&Some("GB")));

    let cities: Vec<_> = response
        .cities
        .current
        .iter()
        .map(|row| row.city.as_str())
        .collect();
    assert_eq!(cities, vec!["New York"]);

    let browsers: Vec<_> = response
        .browsers
        .current
        .iter()
        .map(|row| row.browser_family.as_str())
        .collect();
    assert_eq!(browsers, vec!["chrome"]);
}

#[test]
fn previous_window_is_prior_thirty_days_normalized() {
    let conn = open_migrated_db();
    let path_id = insert_path(&conn);
    // Current window [start, end) is a 30-day span ending now, so the
    // normalization factor is 1.0 and previous raw counts are compared
    // directly.
    let day = 86_400;
    let start = 100 * day;
    let end = start + 30 * day;
    let now = end;

    // Current period: one US visit with $6k revenue.
    insert_first_visit(
        &conn,
        1,
        start + day,
        path_id,
        Some("US"),
        None,
        None,
        "desktop",
    );
    // Previous 30 days [start - 30d, start): two US visits, $3k revenue.
    insert_first_visit(
        &conn,
        2,
        start - day,
        path_id,
        Some("US"),
        None,
        None,
        "desktop",
    );
    insert_first_visit(
        &conn,
        3,
        start - 2 * day,
        path_id,
        Some("US"),
        None,
        None,
        "desktop",
    );

    insert_payment(&conn, 1, 1, 6_000, start + 2 * day);
    insert_payment(&conn, 2, 2, 3_000, start - day + 10);

    let response = build_audience_response_at(
        &conn,
        &baseline_query(start, end),
        now,
        &ImportedWindows::default(),
    )
    .expect("response");
    let current = &response.countries.current;
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].country.as_deref(), Some("US"));
    assert_eq!(current[0].metrics.visitors, 1);
    assert_eq!(current[0].metrics.revenue, 6_000);

    // factor == 1.0: revenue 6_000 vs previous 3_000 -> +100%.
    assert_eq!(current[0].metrics.revenue_change.ratio, Some(1.0));
    // visitors 1 vs previous 2 -> -50%.
    assert_eq!(current[0].metrics.visitors_change.ratio, Some(-0.5));
    // conversions 1 vs previous 1 -> 0%.
    assert_eq!(current[0].metrics.conversions_change.ratio, Some(0.0));
}

#[test]
fn short_period_normalizes_previous_window() {
    let conn = open_migrated_db();
    let path_id = insert_path(&conn);
    let day = 86_400;
    // 3-day current window fully in the past -> factor 3/30 = 0.1.
    let start = 100 * day;
    let end = start + 3 * day;
    let now = 200 * day;

    // Current: 3 US visits.
    insert_first_visit(
        &conn,
        1,
        start + 1,
        path_id,
        Some("US"),
        None,
        None,
        "desktop",
    );
    insert_first_visit(
        &conn,
        2,
        start + 2,
        path_id,
        Some("US"),
        None,
        None,
        "desktop",
    );
    insert_first_visit(
        &conn,
        3,
        start + 3,
        path_id,
        Some("US"),
        None,
        None,
        "desktop",
    );
    // Previous 30 days: 30 US visits -> normalized to 30 * 0.1 = 3.
    for seed in 10u8..40 {
        insert_first_visit(
            &conn,
            seed,
            start - day,
            path_id,
            Some("US"),
            None,
            None,
            "desktop",
        );
    }

    let response = build_audience_response_at(
        &conn,
        &baseline_query(start, end),
        now,
        &ImportedWindows::default(),
    )
    .expect("response");
    let current = &response.countries.current;
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].metrics.visitors, 3);
    // 3 current vs 30 previous normalized to 3 -> 0% change.
    assert_eq!(current[0].metrics.visitors_change.ratio, Some(0.0));
}

#[test]
fn landings_before_comparison_window_are_ignored() {
    let conn = open_migrated_db();
    let path_id = insert_path(&conn);
    let day = 86_400;
    let start = 100 * day;
    let end = start + 7 * day;
    let now = 200 * day;
    let previous_start = comparison::previous_period_start(start);

    insert_first_visit(
        &conn,
        1,
        start + day,
        path_id,
        Some("US"),
        None,
        None,
        "desktop",
    );
    // Outside the prior 30-day comparison window — must not affect deltas.
    insert_first_visit(
        &conn,
        2,
        previous_start - day,
        path_id,
        Some("US"),
        None,
        None,
        "desktop",
    );
    insert_payment(&conn, 1, 2, 5_000, previous_start - day + 10);

    let response = build_audience_response_at(
        &conn,
        &baseline_query(start, end),
        now,
        &ImportedWindows::default(),
    )
    .expect("response");
    let current = &response.countries.current;
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].country.as_deref(), Some("US"));
    assert_eq!(current[0].metrics.visitors, 1);
    assert_eq!(current[0].metrics.converting_visitors, 0);
    assert_eq!(current[0].metrics.visitors_change, Change::none());
    assert_eq!(current[0].metrics.conversions_change, Change::none());
    assert_eq!(current[0].metrics.revenue_change, Change::none());
}

#[test]
fn groups_by_city_ranked_by_revenue_with_country() {
    let conn = open_migrated_db();
    let path_id = insert_path(&conn);
    insert_first_visit_with_dimensions(
        &conn,
        1,
        100,
        path_id,
        Some("US"),
        Some("New York"),
        None,
        None,
        "desktop",
        0,
        0,
    );
    insert_first_visit_with_dimensions(
        &conn,
        2,
        110,
        path_id,
        Some("US"),
        Some("New York"),
        None,
        None,
        "desktop",
        0,
        0,
    );
    insert_first_visit_with_dimensions(
        &conn,
        3,
        120,
        path_id,
        Some("GB"),
        Some("London"),
        None,
        None,
        "desktop",
        0,
        0,
    );
    // No city: excluded from city breakdown.
    insert_first_visit(&conn, 4, 130, path_id, Some("US"), None, None, "desktop");

    insert_payment(&conn, 1, 1, 50_000, 200);
    insert_payment(&conn, 2, 3, 10_000, 210);

    let response = build_audience_response(&conn, &baseline_query(0, 10_000)).expect("response");
    let current = &response.cities.current;
    assert_eq!(current.len(), 2);

    assert_eq!(current[0].city, "New York");
    assert_eq!(current[0].country.as_deref(), Some("US"));
    assert_eq!(current[0].metrics.visitors, 2);
    assert_eq!(current[0].metrics.converting_visitors, 1);
    assert_eq!(current[0].metrics.revenue, 50_000);

    assert_eq!(current[1].city, "London");
    assert_eq!(current[1].country.as_deref(), Some("GB"));
    assert_eq!(current[1].metrics.visitors, 1);
    assert_eq!(current[1].metrics.converting_visitors, 1);
    assert_eq!(current[1].metrics.revenue, 10_000);

    assert_eq!(current[0].metrics.revenue_change, Change::none());
    assert_eq!(response.cities.offset, 0);
    assert_eq!(response.cities.limit, CITIES_PAGE_SIZE);
    assert_eq!(response.cities.next_offset, None);
}

#[test]
fn paginates_cities_with_limit_and_offset() {
    let conn = open_migrated_db();
    let path_id = insert_path(&conn);
    let total = (CITIES_PAGE_SIZE as usize) + 1;
    for index in 0..total {
        let seed = index as u8;
        insert_first_visit_with_dimensions(
            &conn,
            seed,
            100 + i64::from(seed),
            path_id,
            Some("US"),
            Some(&format!("City-{index:03}")),
            None,
            None,
            "desktop",
            0,
            0,
        );
        // Distinct revenue so default (revenue desc) order is deterministic.
        insert_payment(&conn, seed, seed, (total - index) as i64 * 1_000, 200);
    }

    let mut query = baseline_query(0, 10_000);
    let first_page = build_audience_response(&conn, &query).expect("first page");
    assert_eq!(first_page.cities.current.len(), CITIES_PAGE_SIZE as usize);
    assert_eq!(first_page.cities.offset, 0);
    assert_eq!(first_page.cities.limit, CITIES_PAGE_SIZE);
    assert_eq!(first_page.cities.next_offset, Some(CITIES_PAGE_SIZE));
    assert_eq!(first_page.cities.current[0].city, "City-000");
    assert_eq!(
        first_page.cities.current.last().expect("last").city,
        format!("City-{:03}", CITIES_PAGE_SIZE - 1)
    );

    query.offset = CITIES_PAGE_SIZE;
    let second_page = build_audience_response(&conn, &query).expect("second page");
    assert_eq!(second_page.cities.current.len(), 1);
    assert_eq!(second_page.cities.next_offset, None);
    assert_eq!(
        second_page.cities.current[0].city,
        format!("City-{:03}", CITIES_PAGE_SIZE)
    );
}

#[test]
fn sorts_cities_by_visitors() {
    let conn = open_migrated_db();
    let path_id = insert_path(&conn);
    insert_first_visit_with_dimensions(
        &conn,
        1,
        100,
        path_id,
        Some("US"),
        Some("Alpha"),
        None,
        None,
        "desktop",
        0,
        0,
    );
    insert_first_visit_with_dimensions(
        &conn,
        2,
        110,
        path_id,
        Some("US"),
        Some("Bravo"),
        None,
        None,
        "desktop",
        0,
        0,
    );
    insert_first_visit_with_dimensions(
        &conn,
        3,
        120,
        path_id,
        Some("US"),
        Some("Bravo"),
        None,
        None,
        "desktop",
        0,
        0,
    );
    insert_payment(&conn, 1, 1, 50_000, 200);

    let mut query = baseline_query(0, 10_000);
    query.order_by = CitiesOrderBy::Visitors;
    query.direction = SortDirection::Desc;
    let response = build_audience_response(&conn, &query).expect("response");
    let cities: Vec<_> = response
        .cities
        .current
        .iter()
        .map(|row| row.city.as_str())
        .collect();
    assert_eq!(cities, vec!["Bravo", "Alpha"]);
    assert_eq!(response.cities.current[0].metrics.visitors, 2);
    assert_eq!(response.cities.current[1].metrics.visitors, 1);
    assert_eq!(response.cities.current[1].metrics.revenue, 50_000);
}

#[test]
fn city_change_compares_prior_thirty_days() {
    let conn = open_migrated_db();
    let path_id = insert_path(&conn);
    let day = 86_400;
    let start = 100 * day;
    let end = start + 30 * day;
    let now = end;
    // Current: one Chicago visit, $8k.
    insert_first_visit_with_dimensions(
        &conn,
        1,
        start + day,
        path_id,
        Some("US"),
        Some("Chicago"),
        None,
        None,
        "desktop",
        0,
        0,
    );
    // Prior 30 days: one Chicago visit, $2k. factor 1.0 -> +300%.
    insert_first_visit_with_dimensions(
        &conn,
        2,
        start - day,
        path_id,
        Some("US"),
        Some("Chicago"),
        None,
        None,
        "desktop",
        0,
        0,
    );
    insert_payment(&conn, 1, 1, 8_000, start + 2 * day);
    insert_payment(&conn, 2, 2, 2_000, start - day + 10);

    let response = build_audience_response_at(
        &conn,
        &baseline_query(start, end),
        now,
        &ImportedWindows::default(),
    )
    .expect("response");
    let current = &response.cities.current;
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].city, "Chicago");
    assert_eq!(current[0].metrics.visitors, 1);
    assert_eq!(current[0].metrics.revenue, 8_000);
    assert_eq!(current[0].metrics.revenue_change.ratio, Some(3.0));
    assert_eq!(current[0].metrics.visitors_change.ratio, Some(0.0));
}

#[test]
fn groups_by_browser_family_ranked_by_revenue() {
    let conn = open_migrated_db();
    let path_id = insert_path(&conn);
    insert_first_visit(
        &conn,
        1,
        100,
        path_id,
        None,
        Some("chrome"),
        None,
        "desktop",
    );
    insert_first_visit(
        &conn,
        2,
        110,
        path_id,
        None,
        Some("chrome"),
        None,
        "desktop",
    );
    insert_first_visit(
        &conn,
        3,
        120,
        path_id,
        None,
        Some("firefox"),
        None,
        "desktop",
    );
    insert_first_visit(&conn, 4, 130, path_id, None, None, None, "desktop");

    insert_payment(&conn, 1, 1, 50_000, 200);
    insert_payment(&conn, 2, 3, 10_000, 210);

    let response = build_audience_response(&conn, &baseline_query(0, 10_000)).expect("response");
    let current = &response.browsers.current;
    assert_eq!(current.len(), 3);

    assert_eq!(current[0].browser_family, "chrome");
    assert_eq!(current[0].browser_family_name, "Chrome");
    assert_eq!(current[0].metrics.visitors, 2);
    assert_eq!(current[0].metrics.converting_visitors, 1);
    assert_eq!(current[0].metrics.revenue, 50_000);

    assert_eq!(current[1].browser_family, "firefox");
    assert_eq!(current[1].browser_family_name, "Firefox");
    assert_eq!(current[1].metrics.visitors, 1);
    assert_eq!(current[1].metrics.converting_visitors, 1);
    assert_eq!(current[1].metrics.revenue, 10_000);

    assert_eq!(current[2].browser_family, "unknown");
    assert_eq!(current[2].browser_family_name, "Unknown");
    assert_eq!(current[2].metrics.visitors, 1);
    assert_eq!(current[2].metrics.converting_visitors, 0);
    assert_eq!(current[2].metrics.revenue, 0);
}

#[test]
fn browser_change_compares_prior_thirty_days() {
    let conn = open_migrated_db();
    let path_id = insert_path(&conn);
    let day = 86_400;
    let start = 100 * day;
    let end = start + 30 * day;
    let now = end;
    insert_first_visit(
        &conn,
        1,
        start + day,
        path_id,
        None,
        Some("safari"),
        None,
        "desktop",
    );
    insert_first_visit(
        &conn,
        2,
        start - day,
        path_id,
        None,
        Some("safari"),
        None,
        "desktop",
    );
    insert_payment(&conn, 1, 1, 8_000, start + 2 * day);
    insert_payment(&conn, 2, 2, 2_000, start - day + 10);

    let response = build_audience_response_at(
        &conn,
        &baseline_query(start, end),
        now,
        &ImportedWindows::default(),
    )
    .expect("response");
    let current = &response.browsers.current;
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].browser_family, "safari");
    assert_eq!(current[0].metrics.visitors, 1);
    assert_eq!(current[0].metrics.revenue, 8_000);
    assert_eq!(current[0].metrics.revenue_change.ratio, Some(3.0));
    assert_eq!(current[0].metrics.visitors_change.ratio, Some(0.0));
}

#[test]
fn groups_by_os_ranked_by_revenue() {
    let conn = open_migrated_db();
    let path_id = insert_path(&conn);
    insert_first_visit(
        &conn,
        1,
        100,
        path_id,
        None,
        None,
        Some("windows"),
        "desktop",
    );
    insert_first_visit(
        &conn,
        2,
        110,
        path_id,
        None,
        None,
        Some("windows"),
        "desktop",
    );
    insert_first_visit(&conn, 3, 120, path_id, None, None, Some("macos"), "desktop");
    insert_first_visit(&conn, 4, 130, path_id, None, None, None, "desktop");

    insert_payment(&conn, 1, 1, 50_000, 200);
    insert_payment(&conn, 2, 3, 10_000, 210);

    let response = build_audience_response(&conn, &baseline_query(0, 10_000)).expect("response");
    let current = &response.os.current;
    assert_eq!(current.len(), 3);

    assert_eq!(current[0].os, "windows");
    assert_eq!(current[0].os_name, "Windows");
    assert_eq!(current[0].metrics.visitors, 2);
    assert_eq!(current[0].metrics.converting_visitors, 1);
    assert_eq!(current[0].metrics.revenue, 50_000);

    assert_eq!(current[1].os, "macos");
    assert_eq!(current[1].os_name, "macOS");
    assert_eq!(current[1].metrics.visitors, 1);
    assert_eq!(current[1].metrics.converting_visitors, 1);
    assert_eq!(current[1].metrics.revenue, 10_000);

    assert_eq!(current[2].os, "unknown");
    assert_eq!(current[2].os_name, "Unknown");
    assert_eq!(current[2].metrics.visitors, 1);
    assert_eq!(current[2].metrics.converting_visitors, 0);
    assert_eq!(current[2].metrics.revenue, 0);
}

#[test]
fn os_change_compares_prior_thirty_days() {
    let conn = open_migrated_db();
    let path_id = insert_path(&conn);
    let day = 86_400;
    let start = 100 * day;
    let end = start + 30 * day;
    let now = end;
    insert_first_visit(
        &conn,
        1,
        start + day,
        path_id,
        None,
        None,
        Some("linux"),
        "desktop",
    );
    insert_first_visit(
        &conn,
        2,
        start - day,
        path_id,
        None,
        None,
        Some("linux"),
        "desktop",
    );
    insert_payment(&conn, 1, 1, 8_000, start + 2 * day);
    insert_payment(&conn, 2, 2, 2_000, start - day + 10);

    let response = build_audience_response_at(
        &conn,
        &baseline_query(start, end),
        now,
        &ImportedWindows::default(),
    )
    .expect("response");
    let current = &response.os.current;
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].os, "linux");
    assert_eq!(current[0].metrics.visitors, 1);
    assert_eq!(current[0].metrics.revenue, 8_000);
    assert_eq!(current[0].metrics.revenue_change.ratio, Some(3.0));
    assert_eq!(current[0].metrics.visitors_change.ratio, Some(0.0));
}

#[test]
fn groups_by_device_ranked_by_revenue() {
    let conn = open_migrated_db();
    let path_id = insert_path(&conn);
    insert_first_visit(&conn, 1, 100, path_id, None, None, None, "desktop");
    insert_first_visit(&conn, 2, 110, path_id, None, None, None, "desktop");
    insert_first_visit(&conn, 3, 120, path_id, None, None, None, "mobile");
    insert_first_visit(&conn, 4, 130, path_id, None, None, None, "tablet");

    insert_payment(&conn, 1, 1, 50_000, 200);
    insert_payment(&conn, 2, 3, 10_000, 210);

    let response = build_audience_response(&conn, &baseline_query(0, 10_000)).expect("response");
    let current = &response.devices.current;
    assert_eq!(current.len(), 3);

    assert_eq!(current[0].device, "desktop");
    assert_eq!(current[0].device_name, "Desktop");
    assert_eq!(current[0].metrics.visitors, 2);
    assert_eq!(current[0].metrics.converting_visitors, 1);
    assert_eq!(current[0].metrics.revenue, 50_000);

    assert_eq!(current[1].device, "mobile");
    assert_eq!(current[1].device_name, "Mobile");
    assert_eq!(current[1].metrics.visitors, 1);
    assert_eq!(current[1].metrics.converting_visitors, 1);
    assert_eq!(current[1].metrics.revenue, 10_000);

    assert_eq!(current[2].device, "tablet");
    assert_eq!(current[2].device_name, "Tablet");
    assert_eq!(current[2].metrics.visitors, 1);
    assert_eq!(current[2].metrics.converting_visitors, 0);
    assert_eq!(current[2].metrics.revenue, 0);
}

#[test]
fn device_change_compares_prior_thirty_days() {
    let conn = open_migrated_db();
    let path_id = insert_path(&conn);
    let day = 86_400;
    let start = 100 * day;
    let end = start + 30 * day;
    let now = end;
    insert_first_visit(&conn, 1, start + day, path_id, None, None, None, "mobile");
    insert_first_visit(&conn, 2, start - day, path_id, None, None, None, "mobile");
    insert_payment(&conn, 1, 1, 8_000, start + 2 * day);
    insert_payment(&conn, 2, 2, 2_000, start - day + 10);

    let response = build_audience_response_at(
        &conn,
        &baseline_query(start, end),
        now,
        &ImportedWindows::default(),
    )
    .expect("response");
    let current = &response.devices.current;
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].device, "mobile");
    assert_eq!(current[0].metrics.visitors, 1);
    assert_eq!(current[0].metrics.revenue, 8_000);
    assert_eq!(current[0].metrics.revenue_change.ratio, Some(3.0));
    assert_eq!(current[0].metrics.visitors_change.ratio, Some(0.0));
}

#[test]
fn groups_by_viewport_width_bucket_ranked_by_revenue() {
    let conn = open_migrated_db();
    let path_id = insert_path(&conn);
    insert_first_visit_with_viewport(&conn, 1, 100, path_id, None, None, None, "desktop", 1920);
    insert_first_visit_with_viewport(&conn, 2, 110, path_id, None, None, None, "desktop", 1920);
    insert_first_visit_with_viewport(&conn, 3, 120, path_id, None, None, None, "mobile", 390);
    insert_first_visit_with_viewport(&conn, 4, 130, path_id, None, None, None, "tablet", 800);
    // viewport_width 0 is excluded from viewport breakdown.
    insert_first_visit(&conn, 5, 140, path_id, None, None, None, "desktop");

    insert_payment(&conn, 1, 1, 50_000, 200);
    insert_payment(&conn, 2, 3, 10_000, 210);

    let response = build_audience_response(&conn, &baseline_query(0, 10_000)).expect("response");
    let current = &response.viewports.current;
    assert_eq!(current.len(), 3);

    assert_eq!(current[0].viewport, "x_large_desktop");
    assert_eq!(current[0].metrics.visitors, 2);
    assert_eq!(current[0].metrics.converting_visitors, 1);
    assert_eq!(current[0].metrics.revenue, 50_000);

    assert_eq!(current[1].viewport, "mobile");
    assert_eq!(current[1].metrics.visitors, 1);
    assert_eq!(current[1].metrics.converting_visitors, 1);
    assert_eq!(current[1].metrics.revenue, 10_000);

    assert_eq!(current[2].viewport, "tablet");
    assert_eq!(current[2].metrics.visitors, 1);
    assert_eq!(current[2].metrics.converting_visitors, 0);
    assert_eq!(current[2].metrics.revenue, 0);

    assert!(response.screen_sizes.current.is_empty());
    assert_eq!(response.devices.current.len(), 3);
}

#[test]
fn groups_by_screen_width_bucket_ranked_by_revenue() {
    let conn = open_migrated_db();
    let path_id = insert_path(&conn);
    insert_first_visit_with_dimensions(
        &conn, 1, 100, path_id, None, None, None, None, "desktop", 1920, 1200,
    );
    insert_first_visit_with_dimensions(
        &conn, 2, 110, path_id, None, None, None, None, "desktop", 1920, 1200,
    );
    insert_first_visit_with_dimensions(
        &conn, 3, 120, path_id, None, None, None, None, "mobile", 390, 390,
    );
    insert_first_visit_with_dimensions(
        &conn, 4, 130, path_id, None, None, None, None, "tablet", 800, 768,
    );
    // screen_width 0 is excluded from screen-size breakdown.
    insert_first_visit(&conn, 5, 140, path_id, None, None, None, "desktop");

    insert_payment(&conn, 1, 1, 50_000, 200);
    insert_payment(&conn, 2, 3, 10_000, 210);

    let response = build_audience_response(&conn, &baseline_query(0, 10_000)).expect("response");
    let current = &response.screen_sizes.current;
    assert_eq!(current.len(), 3);

    assert_eq!(current[0].screen_size, "x_large_desktop");
    assert_eq!(current[0].metrics.visitors, 2);
    assert_eq!(current[0].metrics.converting_visitors, 1);
    assert_eq!(current[0].metrics.revenue, 50_000);

    assert_eq!(current[1].screen_size, "mobile");
    assert_eq!(current[1].metrics.visitors, 1);
    assert_eq!(current[1].metrics.revenue, 10_000);

    assert_eq!(current[2].screen_size, "tablet");
    assert_eq!(current[2].metrics.visitors, 1);
    assert_eq!(current[2].metrics.revenue, 0);
}

#[test]
fn width_bucket_boundaries() {
    assert_eq!(width_bucket(359), Some("small_mobile"));
    assert_eq!(width_bucket(360), Some("mobile"));
    assert_eq!(width_bucket(767), Some("mobile"));
    assert_eq!(width_bucket(768), Some("tablet"));
    assert_eq!(width_bucket(1023), Some("tablet"));
    assert_eq!(width_bucket(1024), Some("desktop"));
    assert_eq!(width_bucket(1439), Some("desktop"));
    assert_eq!(width_bucket(1440), Some("large_desktop"));
    assert_eq!(width_bucket(1919), Some("large_desktop"));
    assert_eq!(width_bucket(1920), Some("x_large_desktop"));
}

#[test]
fn screen_size_change_compares_prior_thirty_days() {
    let conn = open_migrated_db();
    let path_id = insert_path(&conn);
    let day = 86_400;
    let start = 100 * day;
    let end = start + 30 * day;
    let now = end;
    insert_first_visit_with_dimensions(
        &conn,
        1,
        start + day,
        path_id,
        None,
        None,
        None,
        None,
        "desktop",
        1200,
        800,
    );
    insert_first_visit_with_dimensions(
        &conn,
        2,
        start - day,
        path_id,
        None,
        None,
        None,
        None,
        "desktop",
        1200,
        800,
    );
    insert_payment(&conn, 1, 1, 8_000, start + 2 * day);
    insert_payment(&conn, 2, 2, 2_000, start - day + 10);

    let response = build_audience_response_at(
        &conn,
        &baseline_query(start, end),
        now,
        &ImportedWindows::default(),
    )
    .expect("response");
    let current = &response.screen_sizes.current;
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].screen_size, "desktop");
    assert_eq!(current[0].metrics.visitors, 1);
    assert_eq!(current[0].metrics.revenue, 8_000);
    assert_eq!(current[0].metrics.revenue_change.ratio, Some(3.0));
    assert_eq!(current[0].metrics.visitors_change.ratio, Some(0.0));
}

#[test]
fn viewport_change_compares_prior_thirty_days() {
    let conn = open_migrated_db();
    let path_id = insert_path(&conn);
    let day = 86_400;
    let start = 100 * day;
    let end = start + 30 * day;
    let now = end;
    insert_first_visit_with_dimensions(
        &conn,
        1,
        start + day,
        path_id,
        None,
        None,
        None,
        None,
        "desktop",
        1920,
        1200,
    );
    insert_first_visit_with_dimensions(
        &conn,
        2,
        start - day,
        path_id,
        None,
        None,
        None,
        None,
        "desktop",
        1920,
        1200,
    );
    insert_payment(&conn, 1, 1, 8_000, start + 2 * day);
    insert_payment(&conn, 2, 2, 2_000, start - day + 10);

    let response = build_audience_response_at(
        &conn,
        &baseline_query(start, end),
        now,
        &ImportedWindows::default(),
    )
    .expect("response");
    let current = &response.viewports.current;
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].viewport, "desktop");
    assert_eq!(current[0].metrics.visitors, 1);
    assert_eq!(current[0].metrics.revenue, 8_000);
    assert_eq!(current[0].metrics.revenue_change.ratio, Some(3.0));
    assert_eq!(current[0].metrics.visitors_change.ratio, Some(0.0));
}
