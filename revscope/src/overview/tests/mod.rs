use super::buckets::NANOS_PER_SECOND;
use super::mrr::{CHURN_RESUBSCRIBE_WINDOW_SECONDS, fetch_mrr};
use super::*;
use crate::comparison::DAY_SECONDS;
use crate::subscription_status;
use crate::test_utils::open_migrated_db;

use crate::db::ProjectConnection;

use crate::test_utils::{
    PaymentFixture, SubscriptionFixture, SubscriptionVersionFixture,
    insert_payment as insert_test_payment, insert_subscription as insert_test_subscription,
    insert_subscription_version, visitor_timestamp_id_for_page_id,
};
use rusqlite::OptionalExtension;
use rusqlite::params;

const DAY: i64 = DAY_SECONDS;
const HOUR: i64 = 3_600;
const WEEK_SECONDS: i64 = 7 * DAY;
/// Thousandths of a dollar ($100.00).
const DOLLARS_100: i64 = 100_000;
const DOLLARS_25: i64 = 25_000;
const DOLLARS_30: i64 = 30_000;
const DOLLARS_50: i64 = 50_000;
const DOLLARS_200: i64 = 200_000;
const DOLLARS_1200: i64 = 1_200_000;

/// Most tests are about aggregation, not calendars, so they bucket in UTC.
fn utc() -> ProjectTimezone {
    ProjectTimezone::utc()
}

fn zone(name: &str) -> ProjectTimezone {
    timezone::validate(name).expect("known zone")
}

/// Daily-bucket overview, the default used by the route when `bucket` is omitted.
fn build_overview_response(
    conn: &ProjectConnection,
    start: i64,
    end: i64,
) -> Result<OverviewResponse, AuthError> {
    build_overview_response_with_bucket(
        conn,
        start,
        end,
        BucketGranularity::Daily,
        &utc(),
        &SegmentFilters::default(),
    )
}

fn page_id(seed: u8) -> [u8; 16] {
    [seed; 16]
}

fn timestamp_id(seed: u8, viewed_at: i64) -> i64 {
    viewed_at * NANOS_PER_SECOND + i64::from(seed)
}

fn insert_path(conn: &ProjectConnection, path: &str, hostname: &str) -> i64 {
    conn.execute(
        "INSERT INTO paths (path, hostname) VALUES (?1, ?2)",
        params![path, hostname],
    )
    .expect("insert path");
    conn.query_row("SELECT id FROM paths ORDER BY id DESC LIMIT 1", [], |row| {
        row.get::<_, i64>(0)
    })
    .expect("path id")
}

fn insert_pageview(
    conn: &ProjectConnection,
    id: [u8; 16],
    viewed_at: i64,
    visitor_timestamp_id: i64,
    previous_timestamp_id: Option<i64>,
    path_id: i64,
    duration_seconds: Option<i64>,
) {
    let timestamp_id = timestamp_id(id[0], viewed_at);
    conn.execute(
        "INSERT INTO pageviews (
            timestamp_id, visitor_timestamp_id, previous_timestamp_id, path_id,
            duration_seconds, scrolled_percent
         ) VALUES (?1, ?2, ?3, ?4, ?5, NULL)",
        params![
            timestamp_id,
            visitor_timestamp_id,
            previous_timestamp_id,
            path_id,
            duration_seconds,
        ],
    )
    .expect("insert pageview");
    conn.execute(
        "INSERT INTO client_page_ids_map (visitor_timestamp_id, page_id)
         VALUES (?1, ?2)",
        params![timestamp_id, id.as_slice()],
    )
    .expect("insert page id map");
}

/// Inserts a landing session with `extra_pages` additional pageviews in the same session.
fn insert_session(
    conn: &ProjectConnection,
    path_id: i64,
    landing_seed: u8,
    viewed_at: i64,
    landing_duration_seconds: i64,
    extra_page_durations: &[i64],
) -> [u8; 16] {
    let landing_id = page_id(landing_seed);
    let visitor_timestamp_id = timestamp_id(landing_seed, viewed_at);
    insert_pageview(
        conn,
        landing_id,
        viewed_at,
        visitor_timestamp_id,
        None,
        path_id,
        Some(landing_duration_seconds),
    );

    let mut previous_timestamp_id = visitor_timestamp_id;
    for (index, duration_seconds) in extra_page_durations.iter().enumerate() {
        let page_id_value = page_id(landing_seed.wrapping_add(1 + index as u8));
        let page_viewed_at = viewed_at + index as i64 + 1;
        let page_timestamp_id = timestamp_id(page_id_value[0], page_viewed_at);
        insert_pageview(
            conn,
            page_id_value,
            page_viewed_at,
            visitor_timestamp_id,
            Some(previous_timestamp_id),
            path_id,
            Some(*duration_seconds),
        );
        previous_timestamp_id = page_timestamp_id;
    }

    let session_duration_seconds =
        landing_duration_seconds + extra_page_durations.iter().sum::<i64>();
    let session_page_count = 1 + extra_page_durations.len() as i64;
    conn.execute(
        "INSERT INTO landings (
            visitor_timestamp_id, session_duration_seconds, session_page_count,
            is_google, tracker_version, device
         ) VALUES (?1, ?2, ?3, 0, 1.0, 'desktop')",
        params![
            visitor_timestamp_id,
            session_duration_seconds,
            session_page_count,
        ],
    )
    .expect("insert landing");

    landing_id
}

fn insert_subscription(
    conn: &ProjectConnection,
    provider_subscription_id: &str,
    interval_unit: &str,
    started_at: i64,
    canceled_at: Option<i64>,
    ended_at: Option<i64>,
) -> i64 {
    let period_end = started_at + 30 * DAY;
    let mut subscription = SubscriptionFixture::new(provider_subscription_id);
    subscription.provider_customer_id = Some(format!("cus_{provider_subscription_id}"));
    subscription.customer_email = Some(format!("{provider_subscription_id}@example.com"));
    subscription.interval_unit = interval_unit.to_string();
    subscription.current_period_start = Some(started_at);
    subscription.current_period_end = Some(period_end);
    subscription.started_at = Some(started_at);
    subscription.ended_at = ended_at;
    subscription.canceled_at = canceled_at;
    insert_test_subscription(conn, subscription)
}

fn payment_mrr_amount(amount: i64, payment_type: &str, interval_unit: Option<&str>) -> Option<i64> {
    if payment_type != "subscription" {
        return None;
    }
    let unit = interval_unit?;
    Some(match unit {
        "month" => amount,
        "year" => amount / 12,
        "week" => amount * 52 / 12,
        "day" => ((amount as f64) * 365.25 / 12.0).round() as i64,
        _ => return None,
    })
}

fn subscription_interval_unit(conn: &ProjectConnection, subscription_id: i64) -> Option<String> {
    conn.query_row(
        "SELECT interval_unit FROM subscriptions WHERE id = ?1",
        params![subscription_id],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .expect("subscription interval query")
}

fn insert_payment(
    conn: &ProjectConnection,
    subscription_id: Option<i64>,
    first_page_id: Option<[u8; 16]>,
    provider_payment_id: &str,
    payment_type: &str,
    amount: i64,
    paid_at: i64,
) {
    let interval_unit = match subscription_id {
        Some(sub_id) => subscription_interval_unit(conn, sub_id),
        None => None,
    };
    let mrr_amount = payment_mrr_amount(amount, payment_type, interval_unit.as_deref());
    let visitor_timestamp_id = first_page_id.map(|first_page_id| {
        visitor_timestamp_id_for_page_id(conn, first_page_id.as_slice())
            .expect("mapped landing timestamp")
    });
    let mut payment = PaymentFixture::new(provider_payment_id, amount, paid_at);
    payment.subscription_id = subscription_id;
    payment.interval_unit = interval_unit;
    payment.visitor_timestamp_id = visitor_timestamp_id;
    payment.payment_type = payment_type.to_string();
    payment.mrr_amount = mrr_amount;
    insert_test_payment(conn, payment);
}

fn sum_bucket_sessions(buckets: &[OverviewBucket]) -> (u32, i64, u32) {
    let mut visitors = 0_u32;
    let mut session_time_total = 0_i64;
    let mut bounced_sessions = 0_u32;
    for bucket in buckets {
        visitors = visitors.saturating_add(bucket.visitors);
        session_time_total += bucket.session_time_total;
        bounced_sessions = bounced_sessions.saturating_add(bucket.bounced_sessions);
    }
    (visitors, session_time_total, bounced_sessions)
}

fn sum_bucket_revenue(buckets: &[OverviewBucket]) -> (i64, i64, i64) {
    let mut recurring = 0_i64;
    let mut new_subscriptions = 0_i64;
    let mut other = 0_i64;
    for bucket in buckets {
        recurring += bucket.recurring_subscriptions;
        new_subscriptions += bucket.new_subscriptions;
        other += bucket.other_revenue;
    }
    (recurring, new_subscriptions, other)
}

fn sum_bucket_other_payments_count(buckets: &[OverviewBucket]) -> u32 {
    buckets
        .iter()
        .map(|bucket| bucket.other_payments_count)
        .sum()
}

fn sum_bucket_refunds(buckets: &[OverviewBucket]) -> (i64, i64, u32) {
    let mut refunds = 0_i64;
    let mut chargebacks = 0_i64;
    let mut count = 0_u32;
    for bucket in buckets {
        refunds += bucket.refunds;
        chargebacks += bucket.chargebacks;
        count = count.saturating_add(bucket.refunds_chargebacks_count);
    }
    (refunds, chargebacks, count)
}

fn sum_bucket_converting_visitors(buckets: &[OverviewBucket]) -> u32 {
    buckets
        .iter()
        .map(|bucket| bucket.converting_visitors)
        .sum()
}

fn sum_bucket_unattributed_converting_visitors(buckets: &[OverviewBucket]) -> u32 {
    buckets
        .iter()
        .map(|bucket| bucket.unattributed_converting_visitors)
        .sum()
}

fn sum_bucket_churn(buckets: &[OverviewBucket]) -> u32 {
    buckets.iter().map(|bucket| bucket.churn).sum()
}

/// Returns `(new_subscriptions_mrr, churn_mrr)` summed across buckets.
fn sum_bucket_mrr_changes(buckets: &[OverviewBucket]) -> (i64, i64) {
    let mut new_mrr = 0_i64;
    let mut churn_mrr = 0_i64;
    for bucket in buckets {
        new_mrr += bucket.new_subscriptions_mrr;
        churn_mrr += bucket.churn_mrr;
    }
    (new_mrr, churn_mrr)
}

mod mrr;
mod response;
