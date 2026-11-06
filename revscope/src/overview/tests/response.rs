use super::*;

#[test]
fn parses_overview_query_from_target() {
    let query = parse_overview_query("/v2/read/projects/1/overview?start=1&end=100")
        .expect("query should parse");
    assert_eq!(query.start, 1);
    assert_eq!(query.end, 100);
}

#[test]
fn rejects_invalid_ranges() {
    assert!(parse_overview_query("/v2/read/projects/1/overview?start=10&end=10").is_err());
}

/// Sessions land in the bucket for their *local* day, and the buckets start
/// on local midnight — the whole point of resolving a project timezone.
#[test]
fn daily_buckets_follow_the_project_timezone() {
    let conn = open_migrated_db();
    let path_id = insert_path(&conn, "/", "acme.test");

    // 2026-06-01 05:00 UTC == 2026-06-01 00:00 CDT, a local midnight.
    let start = 1_780_290_000_i64;
    let end = start + 2 * DAY;
    // Three hours before that boundary is still 2026-05-31 locally.
    insert_session(&conn, path_id, 1, start - 3 * HOUR, 10, &[]);
    insert_session(&conn, path_id, 2, start + HOUR, 10, &[]);
    insert_session(&conn, path_id, 3, start + DAY + HOUR, 10, &[]);

    let overview = build_overview_response_with_bucket(
        &conn,
        start,
        end,
        BucketGranularity::Daily,
        &zone("America/Chicago"),
        &SegmentFilters::default(),
    )
    .expect("overview response");

    assert_eq!(overview.buckets.len(), 2);
    assert_eq!(overview.buckets[0].start, start);
    assert_eq!(overview.buckets[1].start, start + DAY);
    // The session three hours early belongs to the previous local day, which
    // is part of the comparison window rather than the selected period.
    assert_eq!(overview.buckets[0].visitors, 1);
    assert_eq!(overview.buckets[1].visitors, 1);
}

#[test]
fn granularity_label_matches_bucket() {
    assert_eq!(BucketGranularity::Hourly.label(), "hour");
    assert_eq!(BucketGranularity::Daily.label(), "day");
    assert_eq!(BucketGranularity::Weekly.label(), "week");
}

#[test]
fn parses_bucket_query_param() {
    let default_query = parse_overview_query("/v2/read/projects/1/overview?start=1&end=100")
        .expect("query should parse");
    assert_eq!(default_query.bucket, BucketGranularity::Daily);

    let weekly_query =
        parse_overview_query("/v2/read/projects/1/overview?start=1&end=100&bucket=weekly")
            .expect("query should parse");
    assert_eq!(weekly_query.bucket, BucketGranularity::Weekly);

    let hourly_query =
        parse_overview_query("/v2/read/projects/1/overview?start=1&end=100&bucket=hourly")
            .expect("query should parse");
    assert_eq!(hourly_query.bucket, BucketGranularity::Hourly);

    assert!(
        parse_overview_query("/v2/read/projects/1/overview?start=1&end=100&bucket=monthly",)
            .is_err()
    );
}

#[test]
fn weekly_buckets_aggregate_revenue_into_seven_day_windows() {
    let conn = open_migrated_db();

    // A Monday, so the selected range opens on a bucket boundary.
    let start = 1_787_529_600_i64;
    let end = start + 28 * DAY;

    let sub_id = insert_subscription(&conn, "sub_weekly", "month", start - 30 * DAY, None, None);
    insert_payment(
        &conn,
        Some(sub_id),
        None,
        "pay_recurring_week",
        "subscription",
        DOLLARS_100,
        start + 2 * DAY,
    );

    let overview = build_overview_response_with_bucket(
        &conn,
        start,
        end,
        BucketGranularity::Weekly,
        &utc(),
        &SegmentFilters::default(),
    )
    .expect("overview response");
    assert_eq!(overview.granularity, "week");
    assert_eq!(overview.buckets[0].start, start);
    for window in overview.buckets.windows(2) {
        assert_eq!(window[1].start - window[0].start, WEEK_SECONDS);
    }
    let (_, new_subscriptions, _) = sum_bucket_revenue(&overview.buckets);
    assert_eq!(new_subscriptions, DOLLARS_100);
}

#[test]
fn overview_churn_mrr_is_zero_for_a_subscription_paused_before_it_canceled() {
    let conn = open_migrated_db();

    let start = 9_500_000_i64;
    let end = start + 30 * DAY;
    let paused_at = start - 10 * DAY;
    let churn_at = start + 5 * DAY;

    let mut subscription = SubscriptionFixture::new("sub_paused_churn");
    subscription.provider_customer_id = Some("cus_paused_churn".to_string());
    subscription.started_at = Some(start - 60 * DAY);
    subscription.plan_amount = Some(DOLLARS_100);
    let sub_id = insert_test_subscription(&conn, subscription);
    insert_payment(
        &conn,
        Some(sub_id),
        None,
        "pay_paused_churn",
        "subscription",
        DOLLARS_100,
        start - 59 * DAY,
    );

    let mut paused = SubscriptionVersionFixture::new(paused_at);
    paused.change_kind = "status_change".to_string();
    paused.status = Some(subscription_status::PAUSED.to_string());
    paused.plan_amount = Some(DOLLARS_100);
    paused.mrr_amount = Some(DOLLARS_100);
    insert_subscription_version(&conn, sub_id, paused);

    let mut canceled = SubscriptionVersionFixture::new(churn_at);
    canceled.change_kind = "canceled".to_string();
    canceled.status = Some(subscription_status::CANCELED.to_string());
    canceled.plan_amount = Some(DOLLARS_100);
    canceled.mrr_amount = Some(DOLLARS_100);
    canceled.canceled_at = Some(churn_at);
    insert_subscription_version(&conn, sub_id, canceled);
    conn.execute(
        "UPDATE subscriptions SET canceled_at = ?2 WHERE id = ?1",
        params![sub_id, churn_at],
    )
    .expect("cancel subscription");

    let overview = build_overview_response(&conn, start, end).expect("overview response");
    let (_, churn_mrr) = sum_bucket_mrr_changes(&overview.buckets);
    assert_eq!(
        overview.previous_mrr, 0,
        "the pause had already taken it out of MRR"
    );
    assert_eq!(
        churn_mrr, 0,
        "cancelling cannot remove MRR that was no longer counted"
    );
    assert_eq!(sum_bucket_churn(&overview.buckets), 1);
}

#[test]
fn overview_has_breakdowns_for_a_full_year() {
    let conn = open_migrated_db();
    conn.execute_batch("INSERT INTO paths (id, path, hostname) VALUES (1, '/', 'acme.test');")
        .expect("path");
    let start = 9_500_000_i64;
    let end = start + 366 * DAY;
    // The last day of a full-year range.
    let session = (end - DAY) * crate::util::NANOS_PER_SECOND;
    crate::test_utils::insert_landing(
        &conn,
        crate::test_utils::LandingFixture {
            visitor_timestamp_id: session,
            path_id: 1,
            country: Some("FR"),
            ..crate::test_utils::LandingFixture::default()
        },
    );
    let overview = build_overview_response(&conn, start, end).expect("overview");
    let json = serde_json::to_value(&overview).expect("serialize");
    assert_eq!(json["breakdowns"]["countries"][0]["value"], "FR");
    assert_eq!(json["breakdowns"]["countries"][0]["visitors"], 1);
}

#[test]
fn overview_churn_mrr_uses_recurring_price_at_churn() {
    let conn = open_migrated_db();

    let start = 9_200_000_i64;
    let end = start + 30 * DAY;
    let churn_at = start + 10 * DAY;

    let mut subscription = SubscriptionFixture::new("sub_priced_churn");
    subscription.provider_customer_id = Some("cus_priced_churn".to_string());
    subscription.started_at = Some(start - 60 * DAY);
    subscription.plan_amount = Some(DOLLARS_50);
    let sub_id = insert_test_subscription(&conn, subscription);

    insert_payment(
        &conn,
        Some(sub_id),
        None,
        "pay_priced_churn",
        "subscription",
        DOLLARS_50,
        start - 59 * DAY,
    );

    let mut upgrade = SubscriptionVersionFixture::new(start - 30 * DAY);
    upgrade.change_kind = "upgrade".to_string();
    upgrade.plan_amount = Some(DOLLARS_200);
    upgrade.mrr_amount = Some(DOLLARS_200);
    insert_subscription_version(&conn, sub_id, upgrade);

    let mut canceled = SubscriptionVersionFixture::new(churn_at);
    canceled.change_kind = "canceled".to_string();
    canceled.status = Some(subscription_status::CANCELED.to_string());
    canceled.plan_amount = Some(DOLLARS_200);
    canceled.mrr_amount = Some(DOLLARS_200);
    canceled.canceled_at = Some(churn_at);
    insert_subscription_version(&conn, sub_id, canceled);
    conn.execute(
        "UPDATE subscriptions SET canceled_at = ?2 WHERE id = ?1",
        params![sub_id, churn_at],
    )
    .expect("cancel subscription");

    let response = build_overview_response(&conn, start, end).expect("overview response");
    let (_, churn_mrr) = sum_bucket_mrr_changes(&response.buckets);
    assert_eq!(churn_mrr, DOLLARS_200);
}

#[test]
fn overview_churn_mrr_is_zero_for_a_trial_that_was_never_charged() {
    let conn = open_migrated_db();

    let start = 9_300_000_i64;
    let end = start + 30 * DAY;
    let churn_at = start + 5 * DAY;

    let mut subscription = SubscriptionFixture::new("sub_trial_churn");
    subscription.provider_customer_id = Some("cus_trial_churn".to_string());
    subscription.started_at = Some(start - 10 * DAY);
    subscription.plan_amount = Some(DOLLARS_100);
    subscription.canceled_at = Some(churn_at);
    let sub_id = insert_test_subscription(&conn, subscription);
    insert_payment(
        &conn,
        Some(sub_id),
        None,
        "pay_trial_invoice",
        "subscription",
        0,
        start - 10 * DAY,
    );

    let response = build_overview_response(&conn, start, end).expect("overview response");
    let (_, churn_mrr) = sum_bucket_mrr_changes(&response.buckets);
    assert_eq!(
        churn_mrr, 0,
        "a trial that never paid takes no MRR with it when it cancels"
    );
    assert_eq!(
        sum_bucket_churn(&response.buckets),
        1,
        "it is still a lost subscription"
    );
}

#[test]
fn overview_new_subscription_mrr_lands_on_the_first_charge_not_the_trial() {
    let conn = open_migrated_db();

    let start = 9_400_000_i64;
    let end = start + 30 * DAY;
    let trial_start = start + DAY;
    let charged_at = start + 15 * DAY;

    let mut subscription = SubscriptionFixture::new("sub_trial_converts");
    subscription.provider_customer_id = Some("cus_trial_converts".to_string());
    subscription.started_at = Some(trial_start);
    subscription.plan_amount = Some(DOLLARS_100);
    let sub_id = insert_test_subscription(&conn, subscription);
    insert_payment(
        &conn,
        Some(sub_id),
        None,
        "pay_trial_invoice",
        "subscription",
        0,
        trial_start,
    );
    insert_payment(
        &conn,
        Some(sub_id),
        None,
        "pay_trial_conversion",
        "subscription",
        DOLLARS_100,
        charged_at,
    );

    let overview = build_overview_response(&conn, start, end).expect("overview response");

    let (new_mrr, churn_mrr) = sum_bucket_mrr_changes(&overview.buckets);
    assert_eq!(new_mrr, DOLLARS_100);
    assert_eq!(
        overview.previous_mrr + new_mrr - churn_mrr,
        overview.mrr,
        "MRR roll-forward should hold when a trial converts mid-period"
    );

    let booked = overview
        .buckets
        .iter()
        .find(|bucket| bucket.new_subscriptions_mrr > 0)
        .expect("bucket with new MRR");
    assert_eq!(booked.start, (charged_at / DAY) * DAY);
    assert_eq!(
        overview
            .buckets
            .iter()
            .map(|bucket| bucket.new_subscriptions_count)
            .sum::<u32>(),
        1,
        "the trial invoice and the charge are one new subscription, not two"
    );
}

#[test]
fn mrr_at_period_start_equals_mrr_at_prior_period_end() {
    let conn = open_migrated_db();

    let prev_start = 1_000_000_i64;
    let start = prev_start + 30 * DAY;
    let end = start + 30 * DAY;

    let stable_sub = insert_subscription(
        &conn,
        "sub_stable",
        "month",
        prev_start - 10 * DAY,
        None,
        None,
    );
    insert_payment(
        &conn,
        Some(stable_sub),
        None,
        "pay_stable_initial",
        "subscription",
        DOLLARS_100,
        prev_start - 9 * DAY,
    );
    insert_payment(
        &conn,
        Some(stable_sub),
        None,
        "pay_stable_renewal",
        "subscription",
        DOLLARS_100,
        start + 15 * DAY,
    );

    let churned_sub = insert_subscription(
        &conn,
        "sub_churned",
        "month",
        prev_start - 20 * DAY,
        Some(start + 10 * DAY),
        None,
    );
    insert_payment(
        &conn,
        Some(churned_sub),
        None,
        "pay_churned_initial",
        "subscription",
        DOLLARS_200,
        prev_start - 19 * DAY,
    );

    let new_sub = insert_subscription(&conn, "sub_new", "month", start + 5 * DAY, None, None);
    insert_payment(
        &conn,
        Some(new_sub),
        None,
        "pay_new_initial",
        "subscription",
        DOLLARS_50,
        start + 5 * DAY,
    );

    let mrr_at_start = fetch_mrr(&conn, start).expect("mrr at period start");
    let prior_period =
        build_overview_response(&conn, prev_start, start).expect("prior period overview");
    assert_eq!(
        mrr_at_start, prior_period.mrr,
        "MRR at current period start should match prior period end snapshot"
    );
    assert_eq!(mrr_at_start, DOLLARS_100 + DOLLARS_200);

    let current = build_overview_response(&conn, start, end).expect("current period overview");
    assert_eq!(current.mrr, DOLLARS_100 + DOLLARS_50);
    assert_eq!(
        current.mrr,
        mrr_at_start - DOLLARS_200 + DOLLARS_50,
        "end MRR should reflect churned sub removed and new sub added (normalized monthly)"
    );
}

#[test]
fn overview_revenue_buckets_track_new_recurring_and_other() {
    let conn = open_migrated_db();

    let start = 2_000_000_i64;
    let end = start + 30 * DAY;
    let prev_start = start - 30 * DAY;

    let sub_id = insert_subscription(&conn, "sub_revenue", "month", prev_start, None, None);
    insert_payment(
        &conn,
        Some(sub_id),
        None,
        "pay_first",
        "subscription",
        DOLLARS_100,
        prev_start + DAY,
    );
    insert_payment(
        &conn,
        Some(sub_id),
        None,
        "pay_recurring",
        "subscription",
        DOLLARS_100,
        start + 2 * DAY,
    );
    insert_payment(
        &conn,
        None,
        None,
        "pay_single",
        "single",
        DOLLARS_50,
        start + 3 * DAY,
    );

    let overview = build_overview_response(&conn, start, end).expect("overview response");
    assert_eq!(overview.granularity, "day");

    // 30-day current period: previous totals are normalized with factor 1.0.
    let prev_recurring = overview.previous_totals.recurring as i64;
    let prev_new = overview.previous_totals.new_subscriptions as i64;
    let prev_other = overview.previous_totals.other as i64;
    assert_eq!(prev_recurring, 0);
    assert_eq!(prev_new, DOLLARS_100);
    assert_eq!(prev_other, 0);

    let (cur_recurring, cur_new, cur_other) = sum_bucket_revenue(&overview.buckets);
    assert_eq!(cur_recurring, DOLLARS_100);
    assert_eq!(cur_new, 0);
    assert_eq!(cur_other, DOLLARS_50);
    assert_eq!(
        overview.current.total_revenue,
        cur_recurring + cur_new + cur_other,
        "period total revenue should match bucket sums"
    );
    assert_eq!(
        overview.current.new_revenue,
        cur_new + cur_other,
        "new revenue excludes recurring payments"
    );
    assert_eq!(
        overview.previous.total_revenue,
        prev_recurring + prev_new + prev_other
    );
    assert_eq!(overview.previous.new_revenue, prev_new + prev_other);
    assert_eq!(sum_bucket_other_payments_count(&overview.buckets), 1);

    let recurring_count: u32 = overview
        .buckets
        .iter()
        .map(|b| b.recurring_subscriptions_count)
        .sum();
    assert_eq!(recurring_count, 1);
    assert_eq!(overview.previous_totals.new_subscriptions_count, 1.0);
}

#[test]
fn overview_recurring_payment_counts_break_out_annual() {
    let conn = open_migrated_db();

    let start = 2_100_000_i64;
    let end = start + 30 * DAY;
    let prev_start = start - 30 * DAY;

    let monthly_sub =
        insert_subscription(&conn, "sub_monthly_renew", "month", prev_start, None, None);
    insert_payment(
        &conn,
        Some(monthly_sub),
        None,
        "pay_monthly_first",
        "subscription",
        DOLLARS_100,
        prev_start + DAY,
    );
    insert_payment(
        &conn,
        Some(monthly_sub),
        None,
        "pay_monthly_renew",
        "subscription",
        DOLLARS_100,
        start + DAY,
    );

    let annual_sub = insert_subscription(&conn, "sub_annual_renew", "year", prev_start, None, None);
    insert_payment(
        &conn,
        Some(annual_sub),
        None,
        "pay_annual_first",
        "subscription",
        DOLLARS_1200,
        prev_start + 2 * DAY,
    );
    insert_payment(
        &conn,
        Some(annual_sub),
        None,
        "pay_annual_renew",
        "subscription",
        DOLLARS_1200,
        start + 2 * DAY,
    );

    let overview = build_overview_response(&conn, start, end).expect("overview response");
    let recurring_count: u32 = overview
        .buckets
        .iter()
        .map(|b| b.recurring_subscriptions_count)
        .sum();
    let recurring_annual_count: u32 = overview
        .buckets
        .iter()
        .map(|b| b.recurring_annual_subscriptions_count)
        .sum();
    assert_eq!(recurring_count, 2);
    assert_eq!(recurring_annual_count, 1);
}

#[test]
fn overview_mrr_is_snapshot_at_period_end_not_start() {
    let conn = open_migrated_db();

    let start = 3_000_000_i64;
    let end = start + 30 * DAY;

    let sub_id = insert_subscription(&conn, "sub_late", "month", start, None, None);
    insert_payment(
        &conn,
        Some(sub_id),
        None,
        "pay_late",
        "subscription",
        DOLLARS_100,
        start + DAY,
    );

    let overview = build_overview_response(&conn, start, end).expect("overview response");
    assert_eq!(fetch_mrr(&conn, start).expect("mrr at start"), 0);
    assert_eq!(overview.mrr, DOLLARS_100);
}

#[test]
fn overview_session_metrics_count_visitors_bounces_and_duration() {
    let conn = open_migrated_db();
    let path_id = insert_path(&conn, "/", "acme.test");

    let start = 4_000_000_i64;
    let end = start + 30 * DAY;

    insert_session(&conn, path_id, 1, start + DAY, 5, &[]);
    insert_session(&conn, path_id, 2, start + 2 * DAY, 30, &[]);
    insert_session(&conn, path_id, 3, start + 3 * DAY, 60, &[120]);

    let overview = build_overview_response(&conn, start, end).expect("overview response");
    let (visitors, session_time_total, bounced_sessions) = sum_bucket_sessions(&overview.buckets);

    assert_eq!(visitors, 3);
    assert_eq!(bounced_sessions, 1);
    assert_eq!(session_time_total, 5 + 30 + 60 + 120);
}

/// Regression test: the source segment filter must actually narrow the
/// visitors/bounce/session-time metrics, not just the SQL string built by
/// `session_filter_clause`. This previously passed unqualified column names
/// into a self-referencing `EXISTS` subquery on `landings`, which made the
/// filter a database-wide existence check decoupled from the outer row
/// (a no-op whenever any matching landing existed anywhere).
#[test]
fn overview_source_segment_filter_narrows_session_metrics() {
    let conn = open_migrated_db();
    let path_id = insert_path(&conn, "/", "acme.test");

    let start = 4_000_000_i64;
    let end = start + 30 * DAY;

    // A direct session (bounced, 5s) and a campaign session (not bounced, 30s).
    insert_session(&conn, path_id, 1, start + DAY, 5, &[]);
    let campaign_session = insert_session(&conn, path_id, 2, start + 2 * DAY, 30, &[]);

    conn.execute(
        "INSERT INTO campaigns (source, medium, campaign, first_seen_at)
         VALUES ('ads', 'cpc', 'launch', 0)",
        [],
    )
    .expect("insert campaign");
    let campaign_id: i64 = conn
        .query_row("SELECT id FROM campaigns LIMIT 1", [], |row| row.get(0))
        .expect("campaign id");
    let campaign_visitor_timestamp_id = timestamp_id(campaign_session[0], start + 2 * DAY);
    conn.execute(
        "UPDATE landings SET campaign = ?1 WHERE visitor_timestamp_id = ?2",
        params![campaign_id, campaign_visitor_timestamp_id],
    )
    .expect("attach campaign");

    let segments = SegmentFilters {
        countries: Vec::new(),
        sources: vec![crate::segment_filter::SourceFilter::Campaign(campaign_id)],
        pages: Vec::new(),
        landing_pages: Vec::new(),
        events: Vec::new(),
        ..Default::default()
    };
    let overview = build_overview_response_with_bucket(
        &conn,
        start,
        end,
        BucketGranularity::Daily,
        &utc(),
        &segments,
    )
    .expect("overview response");
    let (visitors, session_time_total, bounced_sessions) = sum_bucket_sessions(&overview.buckets);

    // Only the campaign session should count: the direct session must be
    // excluded, proving the source filter actually narrows the outer row
    // rather than degenerating into a database-wide existence check.
    assert_eq!(visitors, 1);
    assert_eq!(session_time_total, 30);
    assert_eq!(bounced_sessions, 0);
}

#[test]
fn overview_normalizes_previous_thirty_days_to_short_current_period() {
    let conn = open_migrated_db();
    let path_id = insert_path(&conn, "/", "acme.test");

    // 3-day current period; the comparison window is the prior 30 days.
    let start = 5_500_000_i64;
    let end = start + 3 * DAY;
    let prev_start = start - 30 * DAY;

    // 30 visitors spread across the 30-day comparison window.
    for index in 0..30_u8 {
        insert_session(
            &conn,
            path_id,
            index,
            prev_start + i64::from(index) * DAY,
            10,
            &[],
        );
    }
    // 2 visitors in the 3-day current period.
    insert_session(&conn, path_id, 200, start + DAY, 10, &[]);
    insert_session(&conn, path_id, 201, start + 2 * DAY, 10, &[]);

    // `now` at or beyond `end` => elapsed = 3 days, factor = 3/30 = 0.1.
    let overview = build_overview_response_at(
        &conn,
        start,
        end,
        BucketGranularity::Daily,
        &utc(),
        end,
        &SegmentFilters::default(),
    )
    .expect("overview response");

    // 30 previous visitors normalized to the 3-day window: 30 * 0.1 = 3.
    assert!((overview.previous_totals.visitors - 3.0).abs() < 1e-9);
    assert!((overview.previous_metrics.visitors - 3.0).abs() < 1e-9);
    assert_eq!(overview.current_metrics.visitors, 2.0);
    // Change vs normalized previous: (2 - 3) / 3.
    assert!(
        (overview.changes.visitors.ratio.expect("ratio") - (-1.0 / 3.0)).abs() < 1e-9,
        "visitors change should compare against the normalized previous period"
    );
}

#[test]
fn overview_session_metrics_split_across_current_and_previous_periods() {
    let conn = open_migrated_db();
    let path_id = insert_path(&conn, "/", "acme.test");

    let start = 5_000_000_i64;
    let end = start + 30 * DAY;
    let prev_start = start - 30 * DAY;

    insert_session(&conn, path_id, 10, prev_start + DAY, 40, &[]);
    insert_session(&conn, path_id, 11, start + DAY, 50, &[]);

    let overview = build_overview_response(&conn, start, end).expect("overview response");

    let (cur_visitors, cur_time, _) = sum_bucket_sessions(&overview.buckets);
    assert_eq!(overview.previous_totals.visitors, 1.0);
    assert_eq!(cur_visitors, 1);
    assert_eq!(cur_time, 50);
}

#[test]
fn overview_converting_visitors_counts_attributed_sessions_and_unattributed_buyers() {
    let conn = open_migrated_db();
    let path_id = insert_path(&conn, "/", "acme.test");

    let start = 6_000_000_i64;
    let end = start + 30 * DAY;

    let session_a = insert_session(&conn, path_id, 20, start + DAY, 10, &[]);
    let session_b = insert_session(&conn, path_id, 21, start + 2 * DAY, 10, &[]);
    insert_session(&conn, path_id, 22, start + 3 * DAY, 10, &[]);

    insert_payment(
        &conn,
        None,
        Some(session_a),
        "pay_single_a",
        "single",
        DOLLARS_50,
        start + DAY,
    );
    insert_payment(
        &conn,
        None,
        Some(session_a),
        "pay_single_a_dup",
        "single",
        DOLLARS_25,
        start + DAY + 60,
    );
    insert_payment(
        &conn,
        None,
        Some(session_b),
        "pay_single_b",
        "single",
        DOLLARS_50,
        start + 2 * DAY,
    );

    insert_payment(
        &conn,
        None,
        None,
        "pay_single_unattributed",
        "single",
        DOLLARS_25,
        start + 4 * DAY,
    );

    let overview = build_overview_response(&conn, start, end).expect("overview response");
    assert_eq!(sum_bucket_converting_visitors(&overview.buckets), 3);
    assert_eq!(
        sum_bucket_unattributed_converting_visitors(&overview.buckets),
        1
    );
}

#[test]
fn overview_avg_time_to_purchase_averages_attributed_first_purchases_by_period() {
    let conn = open_migrated_db();
    let path_id = insert_path(&conn, "/", "acme.test");

    let start = 6_050_000_i64;
    let end = start + 30 * DAY;
    let prev_start = start - 30 * DAY;

    let prev_session = insert_session(&conn, path_id, 50, prev_start + DAY, 10, &[]);
    insert_payment(
        &conn,
        None,
        Some(prev_session),
        "pay_prev",
        "single",
        DOLLARS_50,
        prev_start + DAY + 3_600,
    );

    let current_session_a = insert_session(&conn, path_id, 51, start + DAY, 10, &[]);
    let current_session_b = insert_session(&conn, path_id, 52, start + 2 * DAY, 10, &[]);
    insert_payment(
        &conn,
        None,
        Some(current_session_a),
        "pay_current_a",
        "single",
        DOLLARS_50,
        start + DAY + 3_600,
    );
    insert_payment(
        &conn,
        None,
        Some(current_session_b),
        "pay_current_b",
        "single",
        DOLLARS_50,
        start + 2 * DAY + 7_200,
    );
    insert_payment(
        &conn,
        None,
        None,
        "pay_unattributed",
        "single",
        DOLLARS_25,
        start + 3 * DAY,
    );

    let overview = build_overview_response_at(
        &conn,
        start,
        end,
        BucketGranularity::Daily,
        &utc(),
        end,
        &SegmentFilters::default(),
    )
    .expect("overview response");
    assert_eq!(overview.previous.avg_time_to_purchase, 3_600.0);
    assert_eq!(overview.current.avg_time_to_purchase, 5_400.0);
}

#[test]
fn overview_converting_visitors_excludes_recurring_subscription_payments() {
    let conn = open_migrated_db();
    let path_id = insert_path(&conn, "/", "acme.test");

    let start = 6_100_000_i64;
    let end = start + 30 * DAY;

    let session = insert_session(&conn, path_id, 30, start + DAY, 10, &[]);
    let sub_id = insert_subscription(&conn, "sub_renewal", "month", start + DAY, None, None);
    insert_payment(
        &conn,
        Some(sub_id),
        Some(session),
        "pay_sub_new",
        "subscription",
        DOLLARS_50,
        start + DAY,
    );
    insert_payment(
        &conn,
        Some(sub_id),
        Some(session),
        "pay_sub_renewal",
        "subscription",
        DOLLARS_50,
        start + 2 * DAY,
    );

    let overview = build_overview_response(&conn, start, end).expect("overview response");
    assert_eq!(sum_bucket_converting_visitors(&overview.buckets), 1);
}

#[test]
fn overview_converting_visitors_deduplicates_buyers_across_buckets_in_period() {
    let conn = open_migrated_db();
    let path_id = insert_path(&conn, "/", "acme.test");

    let start = 6_200_000_i64;
    let end = start + 30 * DAY;

    let session = insert_session(&conn, path_id, 40, start + DAY, 10, &[]);
    insert_payment(
        &conn,
        None,
        Some(session),
        "pay_single_day1",
        "single",
        DOLLARS_50,
        start + DAY,
    );
    insert_payment(
        &conn,
        None,
        Some(session),
        "pay_single_day5",
        "single",
        DOLLARS_25,
        start + 5 * DAY,
    );

    let overview = build_overview_response(&conn, start, end).expect("overview response");
    assert_eq!(sum_bucket_converting_visitors(&overview.buckets), 1);

    let first_payment_bucket = (start + DAY) / DAY * DAY;
    let second_payment_bucket = (start + 5 * DAY) / DAY * DAY;
    let first_bucket = overview
        .buckets
        .iter()
        .find(|bucket| bucket.start == first_payment_bucket)
        .expect("bucket for first payment");
    let second_bucket = overview
        .buckets
        .iter()
        .find(|bucket| bucket.start == second_payment_bucket)
        .expect("bucket for second payment");
    assert_eq!(first_bucket.converting_visitors, 1);
    assert_eq!(second_bucket.converting_visitors, 0);
}

#[test]
fn overview_refunds_and_chargebacks_sum_in_revenue_buckets() {
    let conn = open_migrated_db();

    let start = 7_000_000_i64;
    let end = start + 30 * DAY;

    insert_payment(
        &conn,
        None,
        None,
        "pay_refund",
        "refund",
        -DOLLARS_50,
        start + DAY,
    );
    insert_payment(
        &conn,
        None,
        None,
        "pay_chargeback",
        "chargeback",
        -DOLLARS_30,
        start + 2 * DAY,
    );

    let overview = build_overview_response(&conn, start, end).expect("overview response");
    let (refunds, chargebacks, count) = sum_bucket_refunds(&overview.buckets);
    assert_eq!(refunds, -DOLLARS_50);
    assert_eq!(chargebacks, -DOLLARS_30);
    assert_eq!(count, 2);
}

#[test]
fn overview_churn_counts_canceled_subscriptions_without_resubscribe() {
    let conn = open_migrated_db();

    let start = 8_000_000_i64;
    let end = start + 30 * DAY;
    let churn_at = start + 5 * DAY;

    insert_subscription(
        &conn,
        "sub_churn",
        "month",
        start - 10 * DAY,
        Some(churn_at),
        None,
    );

    let overview = build_overview_response(&conn, start, end).expect("overview response");
    assert_eq!(sum_bucket_churn(&overview.buckets), 1);
    assert_eq!(overview.previous_totals.churn_count, 0.0);
}

#[test]
fn overview_churn_ignores_resubscribe_within_24_hours() {
    let conn = open_migrated_db();

    let start = 8_100_000_i64;
    let end = start + 30 * DAY;
    let churn_at = start + 5 * DAY;

    insert_subscription(
        &conn,
        "sub_old",
        "month",
        start - 10 * DAY,
        Some(churn_at),
        None,
    );
    conn.execute(
        "UPDATE subscriptions SET provider_customer_id = 'cus_shared', customer_email = 'shared@example.com'
         WHERE provider_subscription_id = 'sub_old'",
        [],
    )
    .expect("set shared customer on churned sub");
    insert_subscription(
        &conn,
        "sub_replacement",
        "month",
        churn_at + 12 * HOUR,
        None,
        None,
    );
    conn.execute(
        "UPDATE subscriptions SET provider_customer_id = 'cus_shared', customer_email = 'shared@example.com'
         WHERE provider_subscription_id = 'sub_replacement'",
        [],
    )
    .expect("set shared customer on replacement sub");

    let overview = build_overview_response(&conn, start, end).expect("overview response");
    assert_eq!(sum_bucket_churn(&overview.buckets), 0);
}

#[test]
fn overview_churn_counts_resubscribe_after_24_hours() {
    let conn = open_migrated_db();

    let start = 8_200_000_i64;
    let end = start + 30 * DAY;
    let churn_at = start + 5 * DAY;

    insert_subscription(
        &conn,
        "sub_old_late",
        "month",
        start - 10 * DAY,
        Some(churn_at),
        None,
    );
    conn.execute(
        "UPDATE subscriptions SET provider_customer_id = 'cus_late', customer_email = 'late@example.com'
         WHERE provider_subscription_id = 'sub_old_late'",
        [],
    )
    .expect("set shared customer on churned sub");
    insert_subscription(
        &conn,
        "sub_replacement_late",
        "month",
        churn_at + CHURN_RESUBSCRIBE_WINDOW_SECONDS + HOUR,
        None,
        None,
    );
    conn.execute(
        "UPDATE subscriptions SET provider_customer_id = 'cus_late', customer_email = 'late@example.com'
         WHERE provider_subscription_id = 'sub_replacement_late'",
        [],
    )
    .expect("set shared customer on late replacement sub");

    let overview = build_overview_response(&conn, start, end).expect("overview response");
    assert_eq!(sum_bucket_churn(&overview.buckets), 1);
}

#[test]
fn overview_mrr_rolls_forward_through_new_business_and_churn() {
    let conn = open_migrated_db();

    let prev_start = 9_000_000_i64;
    let start = prev_start + 30 * DAY;
    let end = start + 30 * DAY;

    let stable_sub = insert_subscription(
        &conn,
        "sub_stable",
        "month",
        prev_start - 10 * DAY,
        None,
        None,
    );
    insert_payment(
        &conn,
        Some(stable_sub),
        None,
        "pay_stable_initial",
        "subscription",
        DOLLARS_100,
        prev_start - 9 * DAY,
    );
    insert_payment(
        &conn,
        Some(stable_sub),
        None,
        "pay_stable_renewal",
        "subscription",
        DOLLARS_100,
        start + 15 * DAY,
    );

    let churned_sub = insert_subscription(
        &conn,
        "sub_churned",
        "month",
        prev_start - 20 * DAY,
        Some(start + 10 * DAY),
        None,
    );
    insert_payment(
        &conn,
        Some(churned_sub),
        None,
        "pay_churned_initial",
        "subscription",
        DOLLARS_200,
        prev_start - 19 * DAY,
    );

    let new_sub = insert_subscription(&conn, "sub_new", "month", start + 5 * DAY, None, None);
    insert_payment(
        &conn,
        Some(new_sub),
        None,
        "pay_new",
        "subscription",
        DOLLARS_50,
        start + 5 * DAY,
    );

    let overview = build_overview_response(&conn, start, end).expect("overview response");

    assert_eq!(overview.previous_mrr, DOLLARS_100 + DOLLARS_200);
    assert_eq!(overview.mrr, DOLLARS_100 + DOLLARS_50);

    let (new_mrr, churn_mrr) = sum_bucket_mrr_changes(&overview.buckets);
    assert_eq!(new_mrr, DOLLARS_50);
    assert_eq!(churn_mrr, DOLLARS_200);
    let new_subs_count: u32 = overview
        .buckets
        .iter()
        .map(|b| b.new_subscriptions_count)
        .sum();
    assert_eq!(new_subs_count, 1);

    assert_eq!(
        overview.previous_mrr + new_mrr - churn_mrr,
        overview.mrr,
        "previous_mrr + new_subscriptions_mrr - churn_mrr should equal current mrr"
    );
    let growth = overview
        .mrr_growth
        .expect("mrr growth for non-zero start mrr");
    let period_growth =
        (overview.mrr - overview.previous_mrr) as f64 / overview.previous_mrr as f64;
    assert!(
        (growth.monthly - period_growth).abs() < 1e-9,
        "30-day period monthly growth should match period growth"
    );
    let expected_annualized = (1.0 + period_growth).powf(365.0 / 30.0) - 1.0;
    assert!(
        (growth.annualized - expected_annualized).abs() < 1e-9,
        "annualized growth should compound period growth over 365 days"
    );
}

#[test]
fn overview_new_subscriptions_mrr_normalizes_annual_payments() {
    let conn = open_migrated_db();

    let start = 9_100_000_i64;
    let end = start + 30 * DAY;

    let annual_sub = insert_subscription(&conn, "sub_annual_new", "year", start + DAY, None, None);
    insert_payment(
        &conn,
        Some(annual_sub),
        None,
        "pay_annual_first",
        "subscription",
        DOLLARS_1200,
        start + DAY,
    );

    let overview = build_overview_response(&conn, start, end).expect("overview response");

    let (new_mrr, _churn_mrr) = sum_bucket_mrr_changes(&overview.buckets);
    assert_eq!(
        new_mrr,
        DOLLARS_1200 / 12,
        "annual first payment contributes monthly-normalized MRR, not the full payment amount"
    );
    assert_eq!(
        overview.previous_mrr + new_mrr,
        overview.mrr,
        "MRR roll-forward should hold for the annual new-business case too"
    );
    let bucket = overview
        .buckets
        .iter()
        .find(|b| b.new_subscriptions_count > 0)
        .expect("bucket with new subscription");
    assert_eq!(bucket.new_subscriptions_count, 1);
    assert_eq!(bucket.new_annual_subscriptions_count, 1);
}

#[test]
fn overview_churn_mrr_uses_last_pre_churn_payment() {
    let conn = open_migrated_db();

    let start = 9_200_000_i64;
    let end = start + 30 * DAY;
    let churn_at = start + 10 * DAY;

    let sub = insert_subscription(
        &conn,
        "sub_upgrade_then_churn",
        "month",
        start - 60 * DAY,
        Some(churn_at),
        None,
    );
    insert_payment(
        &conn,
        Some(sub),
        None,
        "pay_initial",
        "subscription",
        DOLLARS_50,
        start - 59 * DAY,
    );
    insert_payment(
        &conn,
        Some(sub),
        None,
        "pay_upgrade",
        "subscription",
        DOLLARS_100,
        start - 30 * DAY,
    );

    let overview = build_overview_response(&conn, start, end).expect("overview response");

    let (_new_mrr, churn_mrr) = sum_bucket_mrr_changes(&overview.buckets);
    assert_eq!(
        churn_mrr, DOLLARS_100,
        "churn_mrr should reflect the latest pre-churn payment, not the original signup amount"
    );
}

#[test]
fn overview_churn_buckets_by_ended_at() {
    let conn = open_migrated_db();

    let start = 8_300_000_i64;
    let end = start + 30 * DAY;
    let ended_at = start + 7 * DAY;

    insert_subscription(
        &conn,
        "sub_ended",
        "month",
        start - 10 * DAY,
        None,
        Some(ended_at),
    );

    let overview = build_overview_response(&conn, start, end).expect("overview response");
    assert_eq!(sum_bucket_churn(&overview.buckets), 1);
}

#[test]
fn overview_churn_waits_for_a_scheduled_cancellation_to_take_effect() {
    let conn = open_migrated_db();

    let start = 8_400_000_i64;
    let cancel_at = start + 20 * DAY;

    let mut subscription = SubscriptionFixture::new("sub_scheduled");
    subscription.provider_customer_id = Some("cus_scheduled".to_string());
    subscription.started_at = Some(start - 10 * DAY);
    subscription.canceled_at = Some(start + 5 * DAY);
    subscription.cancel_at = Some(cancel_at);
    insert_test_subscription(&conn, subscription);

    let requested = build_overview_response(&conn, start, cancel_at).expect("overview response");
    assert_eq!(
        sum_bucket_churn(&requested.buckets),
        0,
        "a cancellation that has not taken effect is not churn yet"
    );

    let in_effect =
        build_overview_response(&conn, start, cancel_at + DAY).expect("overview response");
    assert_eq!(sum_bucket_churn(&in_effect.buckets), 1);
}
