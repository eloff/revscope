use rusqlite::params;

use super::report::{
    build_funnel_data_response, build_funnel_data_response_at, build_funnel_data_route_response,
};
use super::store::{load_funnel_options, load_funnel_steps};
use super::walk::{OUTBOUND_CLICK_EVENT_NAME, compute_funnel_counts};
use super::*;
use crate::comparison;
use crate::test_utils::open_migrated_db;
use crate::util::NANOS_PER_SECOND;

use crate::db::ProjectConnection;

use crate::test_utils::{PaymentFixture, insert_payment as insert_test_payment};

fn nanos(seconds: i64, seed: i64) -> i64 {
    seconds * NANOS_PER_SECOND + seed
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

/// Inserts a pageview. `previous` is `None` for a session's landing view.
fn insert_pageview(
    conn: &ProjectConnection,
    timestamp_id: i64,
    visitor_timestamp_id: i64,
    previous: Option<i64>,
    path_id: i64,
) {
    conn.execute(
        "INSERT INTO pageviews (
            timestamp_id, visitor_timestamp_id, previous_timestamp_id, path_id,
            duration_seconds, scrolled_percent
         ) VALUES (?1, ?2, ?3, ?4, NULL, NULL)",
        params![timestamp_id, visitor_timestamp_id, previous, path_id],
    )
    .expect("insert pageview");
}

fn insert_payment(
    conn: &ProjectConnection,
    provider_payment_id: &str,
    visitor_timestamp_id: i64,
    amount: i64,
    paid_at: i64,
) {
    let mut payment = PaymentFixture::new(provider_payment_id, amount, paid_at);
    payment.visitor_timestamp_id = Some(visitor_timestamp_id);
    payment.provider_order_id = Some("order_1".to_string());
    payment.provider_invoice_id = Some("invoice_1".to_string());
    payment.provider_customer_id = Some("customer_1".to_string());
    payment.customer_email = Some("buyer@example.com".to_string());
    payment.fee_amount = Some(100);
    payment.net_amount = Some(amount - 100);
    insert_test_payment(conn, payment);
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
    timestamp_id: i64,
    event_name_id: i64,
    page_timestamp_id: i64,
) {
    conn.execute(
        "INSERT INTO events (timestamp_id, event_name_id, page_timestamp_id, metadata)
         VALUES (?1, ?2, ?3, NULL)",
        params![timestamp_id, event_name_id, page_timestamp_id],
    )
    .expect("insert event");
}

/// Builds a home (landing) -> pricing (pageview) -> payment funnel and
/// returns its id.
fn insert_payment_funnel(conn: &ProjectConnection, home: i64, pricing: i64) -> i64 {
    conn.execute("INSERT INTO funnels (name) VALUES ('checkout')", [])
        .expect("insert funnel");
    let funnel_id = conn
        .query_row(
            "SELECT id FROM funnels ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("funnel id");
    conn.execute(
        "INSERT INTO funnel_steps (funnel_id, step_order, step_type, path_id, is_landing)
         VALUES (?1, 0, 'pageview', ?2, 1)",
        params![funnel_id, home],
    )
    .expect("insert step 0");
    conn.execute(
        "INSERT INTO funnel_steps (funnel_id, step_order, step_type, path_id, is_landing)
         VALUES (?1, 1, 'pageview', ?2, 0)",
        params![funnel_id, pricing],
    )
    .expect("insert step 1");
    conn.execute(
        "INSERT INTO funnel_steps (funnel_id, step_order, step_type, payment_types, is_landing)
         VALUES (?1, 2, 'payment', 'single', 0)",
        params![funnel_id],
    )
    .expect("insert step 2");
    funnel_id
}

/// Builds a single-step funnel for an event that must occur on `path_id`.
fn insert_event_funnel(conn: &ProjectConnection, path_id: i64, event_name_id: i64) -> i64 {
    conn.execute("INSERT INTO funnels (name) VALUES ('signup')", [])
        .expect("insert funnel");
    let funnel_id = conn
        .query_row(
            "SELECT id FROM funnels ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("funnel id");
    conn.execute(
        "INSERT INTO funnel_steps (funnel_id, step_order, step_type, path_id, event_name_id)
         VALUES (?1, 0, 'pageview', ?2, ?3)",
        params![funnel_id, path_id, event_name_id],
    )
    .expect("insert event step");
    funnel_id
}

fn data_query(funnel_id: i64) -> FunnelDataQuery {
    FunnelDataQuery {
        funnel_id: Some(funnel_id),
        start: 0,
        end: 10_000,
        previous_start: -10_000,
        previous_end: 0,
        segments: SegmentFilters::default(),
    }
}

fn bootstrap_query() -> FunnelDataQuery {
    FunnelDataQuery {
        funnel_id: None,
        start: 0,
        end: 10_000,
        previous_start: -10_000,
        previous_end: 0,
        segments: SegmentFilters::default(),
    }
}

#[test]
fn parse_funnel_data_query_allows_omitted_funnel_id() {
    let query =
        parse_funnel_data_query("/v2/read/projects/1/funnel-data?start=0&end=100").expect("parse");
    assert_eq!(query.funnel_id, None);
}

#[test]
fn funnel_data_bootstrap_returns_funnels_and_first_funnel_data() {
    let conn = open_migrated_db();
    let home = insert_path(&conn, "/");
    let pricing = insert_path(&conn, "/pricing");
    let first = insert_payment_funnel(&conn, home, pricing);
    let second = insert_payment_funnel(&conn, home, pricing);
    assert!(first < second);

    let response = build_funnel_data_route_response(&conn, &bootstrap_query()).expect("bootstrap");
    let FunnelDataRouteResponse::Bootstrap(bootstrap) = response else {
        panic!("expected bootstrap response");
    };
    assert_eq!(bootstrap.funnels.len(), 2);
    assert_eq!(bootstrap.funnels[0].id, first);
    let data = bootstrap.data.expect("first funnel data");
    assert_eq!(data.funnel_id, first);
    assert_eq!(data.steps.len(), 3);
}

#[test]
fn funnel_data_bootstrap_empty_project_has_no_data() {
    let conn = open_migrated_db();
    let response = build_funnel_data_route_response(&conn, &bootstrap_query()).expect("bootstrap");
    let FunnelDataRouteResponse::Bootstrap(bootstrap) = response else {
        panic!("expected bootstrap response");
    };
    assert!(bootstrap.funnels.is_empty());
    assert!(bootstrap.data.is_none());
}

#[test]
fn funnel_options_include_unique_event_path_ids() {
    let conn = open_migrated_db();
    let home = insert_path(&conn, "/");
    let pricing = insert_path(&conn, "/pricing");
    let signup = insert_event_name(&conn, "Signup");
    let demo = insert_event_name(&conn, "Demo requested");
    let unused = insert_event_name(&conn, "Unused");

    let home_session = nanos(100, 10);
    insert_pageview(&conn, home_session, home_session, None, home);
    insert_event(&conn, nanos(110, 11), signup, home_session);
    insert_event(&conn, nanos(120, 12), signup, home_session);

    let pricing_session = nanos(200, 20);
    insert_pageview(&conn, pricing_session, pricing_session, None, pricing);
    insert_event(&conn, nanos(210, 21), signup, pricing_session);
    insert_event(&conn, nanos(220, 22), demo, pricing_session);

    let response = load_funnel_options(&conn).expect("funnel options");

    let signup_option = response
        .events
        .iter()
        .find(|event| event.id == signup)
        .expect("signup event option");
    assert_eq!(signup_option.path_ids, vec![home, pricing]);

    let demo_option = response
        .events
        .iter()
        .find(|event| event.id == demo)
        .expect("demo event option");
    assert_eq!(demo_option.path_ids, vec![pricing]);

    assert!(!response.events.iter().any(|event| event.id == unused));
}

#[test]
fn funnel_data_reports_rates_revenue_and_transition_paths() {
    let conn = open_migrated_db();
    let home = insert_path(&conn, "/");
    let pricing = insert_path(&conn, "/pricing");
    let blog = insert_path(&conn, "/blog");
    let about = insert_path(&conn, "/about");
    let funnel_id = insert_payment_funnel(&conn, home, pricing);

    // Session A: completes the whole funnel and pays 25_000.
    let a_home = nanos(100, 10);
    let a_pricing = nanos(110, 11);
    insert_pageview(&conn, a_home, a_home, None, home);
    insert_pageview(&conn, a_pricing, a_home, Some(a_home), pricing);
    insert_pageview(&conn, nanos(120, 12), a_home, Some(a_pricing), blog);
    insert_payment(&conn, "pay_a", a_home, 25_000, 130);

    // Session B: home -> pricing -> /blog, then drops off.
    let b_home = nanos(100, 20);
    let b_pricing = nanos(110, 21);
    insert_pageview(&conn, b_home, b_home, None, home);
    insert_pageview(&conn, b_pricing, b_home, Some(b_home), pricing);
    insert_pageview(&conn, nanos(120, 22), b_home, Some(b_pricing), blog);

    // Session C: home -> /about, drops off after the first step.
    let c_home = nanos(100, 30);
    insert_pageview(&conn, c_home, c_home, None, home);
    insert_pageview(&conn, nanos(110, 31), c_home, Some(c_home), about);

    // Session D: home -> pricing, then exits with no further pageview.
    let d_home = nanos(100, 40);
    let d_pricing = nanos(110, 41);
    insert_pageview(&conn, d_home, d_home, None, home);
    insert_pageview(&conn, d_pricing, d_home, Some(d_home), pricing);

    let response =
        build_funnel_data_response(&conn, &data_query(funnel_id), funnel_id).expect("funnel data");

    assert_eq!(response.total_revenue, Some(25_000));
    assert_eq!(response.steps.len(), 3);

    let step0 = &response.steps[0];
    assert_eq!(step0.step_type, "pageview");
    assert_eq!(step0.path_id, Some(home));
    assert_eq!(step0.visitors, 4);
    assert_eq!(step0.previous_visitors, 0);
    assert!(step0.visitors_change.ratio.is_none());
    assert_eq!(step0.dropoff_rate, Some(0.25));
    assert_eq!(step0.revenue_per_visitor, Some(25_000 / 4));
    let step0_dropoff = step0.dropoff.as_ref().expect("step 0 dropoff");
    // Only session C left after home without going to pricing.
    assert_eq!(step0_dropoff.total_visitors, 1);
    assert_eq!(step0_dropoff.transitions.len(), 1);
    assert_eq!(step0_dropoff.transitions[0].path_id, Some(about));
    assert_eq!(step0_dropoff.transitions[0].visitors, 1);

    let step1 = &response.steps[1];
    assert_eq!(step1.step_type, "pageview");
    assert_eq!(step1.path_id, Some(pricing));
    assert_eq!(step1.visitors, 3);
    assert!((step1.dropoff_rate.unwrap() - 2.0 / 3.0).abs() < 1e-9);
    assert_eq!(step1.revenue_per_visitor, Some(25_000 / 3));
    let step1_dropoff = step1.dropoff.as_ref().expect("step 1 dropoff");
    // Session B -> /blog, session D exits; session A continued to payment.
    assert_eq!(step1_dropoff.total_visitors, 2);
    let blog_row = step1_dropoff
        .transitions
        .iter()
        .find(|row| row.path.as_deref() == Some("/blog"))
        .expect("blog row");
    assert_eq!(blog_row.visitors, 1);
    let exit_row = step1_dropoff
        .transitions
        .iter()
        .find(|row| row.path_id.is_none())
        .expect("exit row");
    assert_eq!(exit_row.visitors, 1);

    let step2 = &response.steps[2];
    assert_eq!(step2.step_type, "payment");
    assert_eq!(step2.path_id, None);
    assert_eq!(step2.visitors, 1);
    assert_eq!(step2.dropoff_rate, None);
    assert!(step2.dropoff.is_none());
    assert_eq!(step2.revenue_per_visitor, Some(25_000));
}

#[test]
fn funnel_data_normalizes_previous_visitors_change() {
    let conn = open_migrated_db();
    let home = insert_path(&conn, "/");
    let pricing = insert_path(&conn, "/pricing");
    let (funnel_id, _, _) = insert_pageview_funnel(&conn, home, pricing);

    // Current 3-day window: 3 visitors reach the first step.
    for seed in 1..=3 {
        let ts = nanos(comparison::PREVIOUS_PERIOD_SECONDS + seed, seed);
        insert_pageview(&conn, ts, ts, None, home);
    }
    // Prior 30 days: 30 visitors (one per day), so normalized previous for a
    // 3-day current window is 3 and the change ratio is 0.
    for day in 0..30 {
        let ts = nanos(day * 86_400 + 1, day + 100);
        insert_pageview(&conn, ts, ts, None, home);
    }

    let start = comparison::PREVIOUS_PERIOD_SECONDS;
    let end = start + 3 * 86_400;
    let query = FunnelDataQuery {
        funnel_id: Some(funnel_id),
        start,
        end,
        previous_start: comparison::previous_period_start(start),
        previous_end: start,
        segments: SegmentFilters::default(),
    };
    let response =
        build_funnel_data_response_at(&conn, &query, funnel_id, end).expect("funnel data");

    assert_eq!(response.steps[0].visitors, 3);
    assert_eq!(response.steps[0].previous_visitors, 30);
    assert_eq!(response.steps[0].visitors_change.ratio, Some(0.0));
}

#[test]
fn funnel_temp_table_walk_reports_per_step_counts() {
    let conn = open_migrated_db();
    let home = insert_path(&conn, "/");
    let pricing = insert_path(&conn, "/pricing");
    let funnel_id = insert_payment_funnel(&conn, home, pricing);

    let a_home = nanos(100, 10);
    let a_pricing = nanos(110, 11);
    insert_pageview(&conn, a_home, a_home, None, home);
    insert_pageview(&conn, a_pricing, a_home, Some(a_home), pricing);
    insert_payment(&conn, "pay_a", a_home, 25_000, 130);

    let b_home = nanos(100, 20);
    insert_pageview(&conn, b_home, b_home, None, home);
    insert_pageview(&conn, nanos(110, 21), b_home, Some(b_home), pricing);

    let c_home = nanos(100, 30);
    insert_pageview(&conn, c_home, c_home, None, home);

    let steps = load_funnel_steps(&conn, funnel_id).expect("funnel steps");
    let counts = compute_funnel_counts(&conn, &steps, 0, 10_000, &SegmentFilters::default())
        .expect("funnel counts");
    assert_eq!(counts, vec![3, 2, 1]);
}

#[test]
fn funnel_without_payment_step_has_no_revenue() {
    let conn = open_migrated_db();
    let home = insert_path(&conn, "/");
    let pricing = insert_path(&conn, "/pricing");
    // Reuse the builder but drop the payment step.
    conn.execute("INSERT INTO funnels (name) VALUES ('views')", [])
        .expect("insert funnel");
    let funnel_id = conn
        .query_row(
            "SELECT id FROM funnels ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("funnel id");
    conn.execute(
        "INSERT INTO funnel_steps (funnel_id, step_order, step_type, path_id, is_landing)
         VALUES (?1, 0, 'pageview', ?2, 1)",
        params![funnel_id, home],
    )
    .expect("insert step 0");
    conn.execute(
        "INSERT INTO funnel_steps (funnel_id, step_order, step_type, path_id, is_landing)
         VALUES (?1, 1, 'pageview', ?2, 0)",
        params![funnel_id, pricing],
    )
    .expect("insert step 1");

    let home_ts = nanos(100, 10);
    insert_pageview(&conn, home_ts, home_ts, None, home);
    insert_pageview(&conn, nanos(110, 11), home_ts, Some(home_ts), pricing);

    let response =
        build_funnel_data_response(&conn, &data_query(funnel_id), funnel_id).expect("funnel data");

    assert_eq!(response.total_revenue, None);
    for step in &response.steps {
        assert_eq!(step.revenue_per_visitor, None);
    }
}

#[test]
fn event_step_counts_only_events_on_its_configured_page() {
    let conn = open_migrated_db();
    let pricing = insert_path(&conn, "/pricing");
    let docs = insert_path(&conn, "/docs");
    let signup = insert_event_name(&conn, "Signup");
    // The step requires the Signup event to occur on /pricing.
    let funnel_id = insert_event_funnel(&conn, pricing, signup);

    let pricing_session_a = nanos(100, 10);
    let pricing_session_b = nanos(100, 20);
    let docs_session = nanos(100, 30);
    insert_pageview(&conn, pricing_session_a, pricing_session_a, None, pricing);
    insert_pageview(&conn, pricing_session_b, pricing_session_b, None, pricing);
    insert_pageview(&conn, docs_session, docs_session, None, docs);
    insert_event(&conn, nanos(110, 11), signup, pricing_session_a);
    insert_event(&conn, nanos(110, 21), signup, pricing_session_b);
    // This Signup happened on /docs, so it is excluded by the page anchor.
    insert_event(&conn, nanos(110, 31), signup, docs_session);

    let response =
        build_funnel_data_response(&conn, &data_query(funnel_id), funnel_id).expect("funnel data");

    assert_eq!(response.steps.len(), 1);
    assert_eq!(response.steps[0].path_id, Some(pricing));
    assert_eq!(response.steps[0].visitors, 2);
    // Single-step funnels have no following step, so no drop-off destinations.
    assert!(response.steps[0].dropoff.is_none());
}

fn insert_pageview_funnel(conn: &ProjectConnection, home: i64, pricing: i64) -> (i64, i64, i64) {
    conn.execute("INSERT INTO funnels (name) VALUES ('checkout')", [])
        .expect("insert funnel");
    let funnel_id = conn
        .query_row(
            "SELECT id FROM funnels ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("funnel id");
    conn.execute(
        "INSERT INTO funnel_steps (funnel_id, step_order, step_type, path_id, is_landing)
         VALUES (?1, 0, 'pageview', ?2, 0)",
        params![funnel_id, home],
    )
    .expect("insert step 0");
    conn.execute(
        "INSERT INTO funnel_steps (funnel_id, step_order, step_type, path_id, is_landing)
         VALUES (?1, 1, 'pageview', ?2, 0)",
        params![funnel_id, pricing],
    )
    .expect("insert step 1");
    let step0 = conn
        .query_row(
            "SELECT id FROM funnel_steps WHERE funnel_id = ?1 AND step_order = 0",
            params![funnel_id],
            |row| row.get::<_, i64>(0),
        )
        .expect("step 0 id");
    let step1 = conn
        .query_row(
            "SELECT id FROM funnel_steps WHERE funnel_id = ?1 AND step_order = 1",
            params![funnel_id],
            |row| row.get::<_, i64>(0),
        )
        .expect("step 1 id");
    (funnel_id, step0, step1)
}

fn insert_outbound_click(
    conn: &ProjectConnection,
    timestamp_id: i64,
    page_timestamp_id: i64,
    to: &str,
) {
    let event_name_id = conn
        .query_row(
            "SELECT id FROM event_names WHERE name = ?1",
            params![OUTBOUND_CLICK_EVENT_NAME],
            |row| row.get::<_, i64>(0),
        )
        .ok()
        .unwrap_or_else(|| insert_event_name(conn, OUTBOUND_CLICK_EVENT_NAME));
    conn.execute(
        "INSERT INTO events (timestamp_id, event_name_id, page_timestamp_id, metadata)
         VALUES (?1, ?2, ?3, jsonb_object('to', ?4))",
        params![timestamp_id, event_name_id, page_timestamp_id, to],
    )
    .expect("insert outbound click event");
}

#[test]
fn buckets_dropoff_sessions_by_next_page_or_exit() {
    let conn = open_migrated_db();
    let home = insert_path(&conn, "/");
    let pricing = insert_path(&conn, "/pricing");
    let blog = insert_path(&conn, "/blog");
    let about = insert_path(&conn, "/about");
    let (funnel_id, _step0, _step1) = insert_pageview_funnel(&conn, home, pricing);

    // Session A: home -> pricing (completes both steps, not a drop-off).
    let a_home = nanos(100, 10);
    let a_pricing = nanos(110, 11);
    insert_pageview(&conn, a_home, a_home, None, home);
    insert_pageview(&conn, a_pricing, a_home, Some(a_home), pricing);

    // Session B: home -> blog (drops off, goes to /blog).
    let b_home = nanos(100, 20);
    insert_pageview(&conn, b_home, b_home, None, home);
    insert_pageview(&conn, nanos(110, 21), b_home, Some(b_home), blog);

    // Session C: home -> about (drops off, goes to /about).
    let c_home = nanos(100, 30);
    insert_pageview(&conn, c_home, c_home, None, home);
    insert_pageview(&conn, nanos(110, 31), c_home, Some(c_home), about);

    // Session D: home only (drops off, exits).
    let d_home = nanos(100, 40);
    insert_pageview(&conn, d_home, d_home, None, home);

    let response =
        build_funnel_data_response(&conn, &data_query(funnel_id), funnel_id).expect("funnel data");
    let dropoff = response.steps[0].dropoff.as_ref().expect("step 0 dropoff");

    // B, C, and D dropped off; A continued to pricing (expected flow).
    assert_eq!(dropoff.total_visitors, 3);

    let blog_row = dropoff
        .transitions
        .iter()
        .find(|row| row.path.as_deref() == Some("/blog"))
        .expect("blog row");
    assert_eq!(blog_row.path_id, Some(blog));
    assert_eq!(blog_row.visitors, 1);

    let about_row = dropoff
        .transitions
        .iter()
        .find(|row| row.path.as_deref() == Some("/about"))
        .expect("about row");
    assert_eq!(about_row.path_id, Some(about));
    assert_eq!(about_row.visitors, 1);

    let exit_row = dropoff
        .transitions
        .iter()
        .find(|row| row.path_id.is_none())
        .expect("exit row");
    assert_eq!(exit_row.visitors, 1);
    assert!((exit_row.share - 1.0 / 3.0).abs() < 1e-9);
}

#[test]
fn splits_dropoff_exit_bucket_by_outbound_click_destination() {
    let conn = open_migrated_db();
    let home = insert_path(&conn, "/");
    let pricing = insert_path(&conn, "/pricing");
    let blog = insert_path(&conn, "/blog");
    let (funnel_id, _step0, _step1) = insert_pageview_funnel(&conn, home, pricing);

    // Session A: home -> pricing (completes both steps, not a drop-off).
    let a_home = nanos(100, 10);
    insert_pageview(&conn, a_home, a_home, None, home);
    insert_pageview(&conn, nanos(110, 11), a_home, Some(a_home), pricing);

    // Session B: home, then an outbound click to stripe.com, then exits.
    let b_home = nanos(100, 20);
    insert_pageview(&conn, b_home, b_home, None, home);
    insert_outbound_click(&conn, nanos(105, 22), b_home, "stripe.com");

    // Session C: home, plain exit (no next page, no outbound click).
    let c_home = nanos(100, 30);
    insert_pageview(&conn, c_home, c_home, None, home);

    // Session D: home -> blog (internal drop-off destination).
    let d_home = nanos(100, 40);
    insert_pageview(&conn, d_home, d_home, None, home);
    insert_pageview(&conn, nanos(110, 41), d_home, Some(d_home), blog);

    let response =
        build_funnel_data_response(&conn, &data_query(funnel_id), funnel_id).expect("funnel data");
    let dropoff = response.steps[0].dropoff.as_ref().expect("step 0 dropoff");

    // B, C, and D dropped off; A continued to pricing (expected flow).
    assert_eq!(dropoff.total_visitors, 3);

    let outbound_row = dropoff
        .transitions
        .iter()
        .find(|row| row.outbound_to.as_deref() == Some("stripe.com"))
        .expect("outbound row");
    assert_eq!(outbound_row.path_id, None);
    assert_eq!(outbound_row.visitors, 1);

    let blog_row = dropoff
        .transitions
        .iter()
        .find(|row| row.path.as_deref() == Some("/blog"))
        .expect("blog row");
    assert_eq!(blog_row.visitors, 1);

    let exit_row = dropoff
        .transitions
        .iter()
        .find(|row| row.path_id.is_none() && row.outbound_to.is_none())
        .expect("exit row");
    assert_eq!(exit_row.visitors, 1);
}

#[test]
fn excludes_navigations_to_next_step_page_for_event_step() {
    let conn = open_migrated_db();
    let home = insert_path(&conn, "/");
    let pricing = insert_path(&conn, "/pricing");
    let docs = insert_path(&conn, "/docs");
    let signup = insert_event_name(&conn, "Signup");

    // Step 0: home page; Step 1: pricing page + Signup event.
    conn.execute("INSERT INTO funnels (name) VALUES ('signup')", [])
        .expect("insert funnel");
    let funnel_id = conn
        .query_row(
            "SELECT id FROM funnels ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("funnel id");
    conn.execute(
        "INSERT INTO funnel_steps (funnel_id, step_order, step_type, path_id)
         VALUES (?1, 0, 'pageview', ?2)",
        params![funnel_id, home],
    )
    .expect("insert step 0");
    conn.execute(
        "INSERT INTO funnel_steps (funnel_id, step_order, step_type, path_id, event_name_id)
         VALUES (?1, 1, 'pageview', ?2, ?3)",
        params![funnel_id, pricing, signup],
    )
    .expect("insert step 1");

    // Session A: home -> pricing, fires Signup on pricing (completes step 1).
    let a_home = nanos(100, 10);
    let a_pricing = nanos(110, 11);
    insert_pageview(&conn, a_home, a_home, None, home);
    insert_pageview(&conn, a_pricing, a_home, Some(a_home), pricing);
    insert_event(&conn, nanos(115, 12), signup, a_pricing);

    // Session B: home -> pricing but never fires Signup. Drop-off, but the
    // next page is the next step's page (pricing), so it is expected flow.
    let b_home = nanos(100, 20);
    insert_pageview(&conn, b_home, b_home, None, home);
    insert_pageview(&conn, nanos(110, 21), b_home, Some(b_home), pricing);

    // Session C: home -> docs. Genuine drop-off to /docs.
    let c_home = nanos(100, 30);
    insert_pageview(&conn, c_home, c_home, None, home);
    insert_pageview(&conn, nanos(110, 31), c_home, Some(c_home), docs);

    let response =
        build_funnel_data_response(&conn, &data_query(funnel_id), funnel_id).expect("funnel data");
    let dropoff = response.steps[0].dropoff.as_ref().expect("step 0 dropoff");

    // Only session C counts; B's navigation to pricing is expected flow.
    assert_eq!(dropoff.total_visitors, 1);
    assert_eq!(dropoff.transitions.len(), 1);
    assert_eq!(dropoff.transitions[0].path_id, Some(docs));
    assert_eq!(dropoff.transitions[0].visitors, 1);
    assert_eq!(dropoff.transitions[0].share, 1.0);
}

#[test]
fn final_step_has_no_dropoff_destinations() {
    let conn = open_migrated_db();
    let home = insert_path(&conn, "/");
    let pricing = insert_path(&conn, "/pricing");
    let (funnel_id, _step0, _step1) = insert_pageview_funnel(&conn, home, pricing);

    let home_ts = nanos(100, 10);
    insert_pageview(&conn, home_ts, home_ts, None, home);
    insert_pageview(&conn, nanos(110, 11), home_ts, Some(home_ts), pricing);

    let response =
        build_funnel_data_response(&conn, &data_query(funnel_id), funnel_id).expect("funnel data");
    assert!(response.steps[0].dropoff.is_some());
    assert!(response.steps[1].dropoff.is_none());
}

/// The project comes from the path, and the dashboard sends no `project_id`.
#[test]
fn funnel_payloads_deserialize_without_a_project_id() {
    let step = r#"{"step_type":"page","path_id":3,"is_landing":false,"payment_types":[]}"#;
    let range = r#""start":1,"end":2,"previous_start":0,"previous_end":1"#;
    serde_json::from_str::<SaveFunnelRequest>(&format!(
        r#"{{"name":"Signup","steps":[{step}],{range}}}"#
    ))
    .expect("save payload");
    serde_json::from_str::<FunnelPreviewRequest>(&format!(r#"{{"steps":[{step}],{range}}}"#))
        .expect("preview payload");
}
