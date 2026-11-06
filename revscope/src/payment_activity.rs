use std::sync::Arc;

use serde::Serialize;

use std::collections::BTreeMap;

use crate::auth::{AuthError, map_row_error};
use crate::authorize::AuthorizedProject;
use crate::db::{ProjectConnection, open_project_database_read};
use crate::segment_filter::{SegmentFilters, parse_segment_filters};
use crate::server_context::ServerContext;
use crate::timezone::{self, BucketGranularity, Buckets, ProjectTimezone};
use crate::util::seconds_since_epoch;

const DAY_SECONDS: i64 = 86_400;
/// Trailing window length for the activity graph: 53 weeks so the GitHub-style
/// grid always has a full set of week columns to render. Also wide enough to
/// cover the full year of monthly buckets returned for the revenue tab.
const WINDOW_DAYS: i64 = 53 * 7;
/// Width of each historical "month" bucket used for the runway chart's past
/// revenue/cash trend and for the frontend's revenue growth trend line.
/// Matches the existing 30-day definition of "monthly revenue" used
/// elsewhere on the revenue tab.
const MONTHLY_BUCKET_DAYS: i64 = 30;
/// Number of trailing 30-day buckets returned for the historical revenue
/// series (a year's worth).
const MONTHLY_BUCKET_COUNT: i64 = 12;

/// Per-day payment counts and revenue split by the categories the activity graph toggles.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub(crate) struct PaymentActivityDay {
    /// Calendar day in the project's timezone, formatted as `YYYY-MM-DD`.
    pub(crate) date: String,
    /// Non-subscription (single) payments on this day.
    pub(crate) single: i64,
    /// Revenue from non-subscription payments on this day, in thousandths of a dollar.
    pub(crate) single_revenue: i64,
    /// First-time subscription payments on this day.
    pub(crate) new_subscription: i64,
    /// Revenue from first-time subscription payments on this day, in thousandths of a dollar.
    pub(crate) new_subscription_revenue: i64,
    /// Renewal (recurring) subscription payments on this day.
    pub(crate) recurring: i64,
    /// Revenue from renewal subscription payments on this day, in thousandths of a dollar.
    pub(crate) recurring_revenue: i64,
}

/// One day's totals, before they are trimmed down to the public
/// [`PaymentActivityDay`] shape. Carries `day_start` (the instant the day begins
/// in the project's timezone) so the monthly-history calculation can locate the
/// day without re-parsing the `date` string.
#[derive(Default)]
struct PaymentActivityDayRow {
    day_start: i64,
    single: i64,
    single_revenue: i64,
    new_subscription: i64,
    new_subscription_revenue: i64,
    recurring: i64,
    recurring_revenue: i64,
}

/// The three categories the activity graph toggles, as emitted by the `category`
/// column in [`fetch_payment_activity_days`]. Numeric so classifying a payment
/// costs no allocation; a window can hold a year of them.
const CATEGORY_SINGLE: i64 = 0;
const CATEGORY_NEW_SUBSCRIPTION: i64 = 1;
const CATEGORY_RECURRING: i64 = 2;

impl PaymentActivityDayRow {
    fn total_revenue(&self) -> i64 {
        self.single_revenue + self.new_subscription_revenue + self.recurring_revenue
    }

    fn add(&mut self, category: i64, amount: i64) {
        let (count, revenue) = match category {
            CATEGORY_SINGLE => (&mut self.single, &mut self.single_revenue),
            CATEGORY_NEW_SUBSCRIPTION => (
                &mut self.new_subscription,
                &mut self.new_subscription_revenue,
            ),
            _ => (&mut self.recurring, &mut self.recurring_revenue),
        };
        *count += 1;
        *revenue += amount;
    }

    fn into_public(self, timezone: &ProjectTimezone) -> PaymentActivityDay {
        PaymentActivityDay {
            date: timezone.calendar_date(self.day_start).unwrap_or_default(),
            single: self.single,
            single_revenue: self.single_revenue,
            new_subscription: self.new_subscription,
            new_subscription_revenue: self.new_subscription_revenue,
            recurring: self.recurring,
            recurring_revenue: self.recurring_revenue,
        }
    }
}

/// Total revenue for one trailing 30-day "month" bucket, used to draw the
/// runway chart's historical trend and to estimate past cash balances.
#[derive(Debug, Serialize, PartialEq)]
pub(crate) struct MonthlyRevenue {
    /// 0 = the most recent trailing 30-day window, 1 = the 30 days before
    /// that, etc.
    pub(crate) months_ago: i64,
    /// Total revenue for that window, in thousandths of a dollar.
    pub(crate) revenue: i64,
}

#[derive(Debug, Serialize)]
pub(crate) struct PaymentActivityResponse {
    /// Inclusive lower bound of the window, in Unix seconds.
    pub(crate) start: i64,
    /// Exclusive upper bound of the window, in Unix seconds.
    pub(crate) end: i64,
    /// Days within the window that have at least one payment, ascending by date.
    pub(crate) days: Vec<PaymentActivityDay>,
    /// Trailing 30-day revenue buckets covering up to the last 12 months,
    /// ascending by recency (index 0 is the most recent). The frontend fits
    /// a revenue growth trend against a user-selected number of these.
    pub(crate) monthly_revenue: Vec<MonthlyRevenue>,
}

pub(crate) struct PaymentActivityQuery {
    pub(crate) segments: SegmentFilters,
}

pub(crate) fn parse_payment_activity_query(
    target: &str,
) -> Result<PaymentActivityQuery, &'static str> {
    Ok(PaymentActivityQuery {
        segments: parse_segment_filters(target),
    })
}

pub(crate) fn payment_activity(
    server_context: Arc<ServerContext>,
    project: &AuthorizedProject,
    query: PaymentActivityQuery,
) -> Result<PaymentActivityResponse, AuthError> {
    fetch_payment_activity(server_context, project, &query)
}

fn fetch_payment_activity(
    server_context: Arc<ServerContext>,
    project: &AuthorizedProject,
    query: &PaymentActivityQuery,
) -> Result<PaymentActivityResponse, AuthError> {
    let timezone = timezone::project_timezone(server_context.clone(), project.key());
    let conn = open_project_database_read(server_context.as_ref(), project.id())
        .map_err(|error| AuthError::Db(error.to_string()))?;
    let end = i64::from(seconds_since_epoch());
    let start = end - WINDOW_DAYS * DAY_SECONDS;
    build_payment_activity_response(&conn, start, end, &timezone, &query.segments)
}

/// Builds the full payment-activity response. `start` should cover
/// [`WINDOW_DAYS`], which is wide enough for both the heatmap and the full
/// year of monthly revenue buckets.
///
/// Days are the project timezone's calendar days, so the heatmap squares and the
/// dates labelling them agree with the rest of the dashboard.
///
/// Segment filters only narrow the purchase-activity heatmap (`days`); the
/// "Monthly revenue" figures always reflect unfiltered totals, since they're
/// meant to be an authoritative summary of the whole business regardless of
/// which segment the user is inspecting.
pub(crate) fn build_payment_activity_response(
    conn: &ProjectConnection,
    start: i64,
    end: i64,
    timezone: &ProjectTimezone,
    segments: &SegmentFilters,
) -> Result<PaymentActivityResponse, AuthError> {
    let heatmap_start = (end - WINDOW_DAYS * DAY_SECONDS).max(start);
    let days_bounds = timezone.buckets(start, end, BucketGranularity::Daily);

    let unfiltered_rows =
        fetch_payment_activity_days(conn, start, end, &days_bounds, &SegmentFilters::default())?;
    let monthly_revenue = summarize_monthly_revenue(&unfiltered_rows, &days_bounds);

    let days = if segments.is_empty() {
        unfiltered_rows
            .into_iter()
            .filter(|row| row.day_start >= heatmap_start)
            .map(|row| row.into_public(timezone))
            .collect()
    } else {
        fetch_payment_activity_days(conn, heatmap_start, end, &days_bounds, segments)?
            .into_iter()
            .map(|row| row.into_public(timezone))
            .collect()
    };

    Ok(PaymentActivityResponse {
        start: heatmap_start,
        end,
        days,
        monthly_revenue,
    })
}

/// Reduces the (always unfiltered) queried day rows into the trailing 30-day
/// revenue buckets used for the runway chart's history and the frontend's
/// revenue growth trend.
///
/// A day's age is its distance from today measured in day boundaries rather than
/// in seconds, so a DST transition inside the window cannot shift a payment into
/// the neighbouring 30-day bucket.
fn summarize_monthly_revenue(
    rows: &[PaymentActivityDayRow],
    days_bounds: &Buckets,
) -> Vec<MonthlyRevenue> {
    let Some(today_index) = days_bounds.starts().len().checked_sub(1) else {
        return Vec::new();
    };

    let mut monthly_buckets = [0i64; MONTHLY_BUCKET_COUNT as usize];
    let mut earliest_days_ago: Option<i64> = None;

    for row in rows {
        let Some(index) = days_bounds.index_for_second(row.day_start) else {
            continue;
        };
        let days_ago = (today_index - index) as i64;
        earliest_days_ago = Some(earliest_days_ago.map_or(days_ago, |max: i64| max.max(days_ago)));

        let bucket = days_ago / MONTHLY_BUCKET_DAYS;
        if bucket < MONTHLY_BUCKET_COUNT {
            monthly_buckets[bucket as usize] += row.total_revenue();
        }
    }

    // Only emit buckets that overlap the actual observed history, so a young
    // project doesn't show a year of misleading zero-revenue months.
    let max_months_ago = earliest_days_ago
        .map(|days| (days / MONTHLY_BUCKET_DAYS).min(MONTHLY_BUCKET_COUNT - 1))
        .unwrap_or(-1);
    (0..=max_months_ago)
        .map(|months_ago| MonthlyRevenue {
            months_ago,
            revenue: monthly_buckets[months_ago as usize],
        })
        .collect()
}

/// Classifies each payment in `[start, end)` and folds it into the day of
/// `days_bounds` it falls on, ascending by day.
///
/// Grouping happens here rather than in SQL because a day boundary in a project
/// timezone is not a fixed multiple of 86,400 seconds, so `date(…,'unixepoch')`
/// can only ever produce UTC days.
fn fetch_payment_activity_days(
    conn: &ProjectConnection,
    start: i64,
    end: i64,
    days_bounds: &Buckets,
    segments: &SegmentFilters,
) -> Result<Vec<PaymentActivityDayRow>, AuthError> {
    // A subscription payment is "new" when it is the earliest successful
    // subscription charge for its subscription; later charges are "recurring".
    // The same EXISTS test the conversions endpoint uses is applied per row.
    let country_filter = segments.country_exists_clause("p.visitor_timestamp_id", 3);
    let session_filter = segments.session_filter_clause("p.visitor_timestamp_id", None);
    let sql = format!(
        "SELECT
            p.paid_at,
            p.amount,
            CASE
                WHEN p.payment_type = 'single' THEN {CATEGORY_SINGLE}
                WHEN NOT EXISTS (
                    SELECT 1 FROM payments prior
                    WHERE prior.subscription_id = p.subscription_id
                      AND prior.status = 'ok'
                      AND prior.payment_type = 'subscription'
                      AND prior.paid_at < p.paid_at
                      AND prior.id != p.id
                ) THEN {CATEGORY_NEW_SUBSCRIPTION}
                ELSE {CATEGORY_RECURRING}
            END AS category
         FROM payments p
         WHERE p.status = 'ok'
           AND p.payment_type IN ('single', 'subscription')
           AND p.paid_at >= ?1
           AND p.paid_at < ?2{country_filter}{session_filter}"
    );

    let mut sql_params: Vec<&dyn rusqlite::types::ToSql> = vec![&start, &end];
    segments.append_country_params(&mut sql_params);
    let days = conn
        .with_query(&sql, rusqlite::params_from_iter(sql_params), |mut rows| {
            let mut days: BTreeMap<i64, PaymentActivityDayRow> = BTreeMap::new();
            while let Some(row) = rows.next()? {
                let paid_at: i64 = row.get(0)?;
                let amount: i64 = row.get(1)?;
                let category: i64 = row.get(2)?;
                let Some(day_start) = days_bounds.bucket_for_second(paid_at) else {
                    continue;
                };
                days.entry(day_start)
                    .or_insert_with(|| PaymentActivityDayRow {
                        day_start,
                        ..PaymentActivityDayRow::default()
                    })
                    .add(category, amount);
            }
            Ok(days)
        })
        .map_err(map_row_error)?;

    Ok(days.into_values().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::open_migrated_db;

    use crate::db::ProjectConnection;

    use crate::test_utils::{
        PaymentFixture, SubscriptionFixture, insert_payment as insert_test_payment,
        insert_subscription as insert_test_subscription,
    };
    use rusqlite::params;

    use crate::util::NANOS_PER_SECOND;

    /// Most tests are about classification, not calendars, so they use UTC days.
    fn utc() -> ProjectTimezone {
        ProjectTimezone::utc()
    }

    fn zone(name: &str) -> ProjectTimezone {
        timezone::validate(name).expect("known zone")
    }

    fn insert_subscription(conn: &ProjectConnection, reference: &str) -> i64 {
        insert_test_subscription(conn, SubscriptionFixture::new(reference))
    }

    fn insert_payment(
        conn: &ProjectConnection,
        subscription_id: Option<i64>,
        payment_type: &str,
        reference: &str,
        paid_at: i64,
    ) {
        let mut payment = PaymentFixture::new(reference, 1000, paid_at);
        payment.subscription_id = subscription_id;
        payment.payment_type = payment_type.to_string();
        insert_test_payment(conn, payment);
    }

    fn insert_single_payment(conn: &ProjectConnection, reference: &str, paid_at: i64, amount: i64) {
        insert_test_payment(conn, PaymentFixture::new(reference, amount, paid_at));
    }

    fn insert_single_payment_for_visitor(
        conn: &ProjectConnection,
        reference: &str,
        paid_at: i64,
        amount: i64,
        visitor_timestamp_id: i64,
    ) {
        let mut payment = PaymentFixture::new(reference, amount, paid_at);
        payment.visitor_timestamp_id = Some(visitor_timestamp_id);
        insert_test_payment(conn, payment);
    }

    fn insert_path(conn: &ProjectConnection) -> i64 {
        conn.execute(
            "INSERT INTO paths (path, hostname) VALUES ('/', 'example.com')",
            params![],
        )
        .expect("insert path");
        conn.query_row(
            "SELECT id FROM paths WHERE path = '/' AND hostname = 'example.com'",
            params![],
            |row| row.get(0),
        )
        .expect("path id")
    }

    /// Inserts a landing session (pageview + landings row) with the given
    /// country, returning its `visitor_timestamp_id` for attributing a payment.
    fn insert_landing(
        conn: &ProjectConnection,
        path_id: i64,
        seed: u8,
        viewed_at: i64,
        country: &str,
    ) -> i64 {
        let timestamp_id = viewed_at * NANOS_PER_SECOND + i64::from(seed);
        conn.execute(
            "INSERT INTO pageviews (
                timestamp_id, visitor_timestamp_id, previous_timestamp_id, path_id,
                duration_seconds, scrolled_percent
             ) VALUES (?1, ?1, NULL, ?2, 5, NULL)",
            params![timestamp_id, path_id],
        )
        .expect("insert pageview");
        conn.execute(
            "INSERT INTO landings (
                visitor_timestamp_id, session_duration_seconds, session_page_count,
                referrer, campaign, click_id, referrer_type, is_google, timezone,
                language, country, city, tracker_version, os, os_version, browser_family,
                device, screen_width, screen_height, viewport_width, viewport_height
             ) VALUES (
                ?1, 5, 1,
                NULL, NULL, 'gclid-1', 'search', 1, 'America/Chicago',
                'EN', ?2, 'Austin', 1.0, 'linux', 6.0, 'chrome',
                'desktop', 1920, 1080, 1440, 900
             )",
            params![timestamp_id, country],
        )
        .expect("insert landing");
        timestamp_id
    }

    #[test]
    fn groups_payments_by_day_and_type() {
        let conn = open_migrated_db();
        let subscription_id = insert_subscription(&conn, "sub_1");

        // Day 0 (1970-01-01): one single payment.
        insert_payment(&conn, None, "single", "p_single", 10);
        // Day 0: first subscription charge (new subscription).
        insert_payment(&conn, Some(subscription_id), "subscription", "p_new", 20);
        // Day 1 (1970-01-02): renewal of the same subscription (recurring).
        insert_payment(
            &conn,
            Some(subscription_id),
            "subscription",
            "p_recurring",
            DAY_SECONDS + 30,
        );

        let response = build_payment_activity_response(
            &conn,
            0,
            10 * DAY_SECONDS,
            &utc(),
            &SegmentFilters::default(),
        )
        .expect("build payment activity response");

        assert_eq!(
            response.days,
            vec![
                PaymentActivityDay {
                    date: "1970-01-01".to_string(),
                    single: 1,
                    single_revenue: 1000,
                    new_subscription: 1,
                    new_subscription_revenue: 1000,
                    recurring: 0,
                    recurring_revenue: 0,
                },
                PaymentActivityDay {
                    date: "1970-01-02".to_string(),
                    single: 0,
                    single_revenue: 0,
                    new_subscription: 0,
                    new_subscription_revenue: 0,
                    recurring: 1,
                    recurring_revenue: 1000,
                },
            ]
        );
    }

    #[test]
    fn groups_payments_by_the_project_timezones_calendar_day() {
        let conn = open_migrated_db();
        // 1970-01-02 04:00 UTC is still 1970-01-01 in Chicago (UTC-6), so this
        // payment belongs to the first day, not the second.
        insert_single_payment(&conn, "p_late", DAY_SECONDS + 4 * 3_600, 1_000);
        // 1970-01-02 07:00 UTC has crossed local midnight.
        insert_single_payment(&conn, "p_next", DAY_SECONDS + 7 * 3_600, 2_000);

        let response = build_payment_activity_response(
            &conn,
            0,
            10 * DAY_SECONDS,
            &zone("America/Chicago"),
            &SegmentFilters::default(),
        )
        .expect("build payment activity response");

        let dates: Vec<(&str, i64)> = response
            .days
            .iter()
            .map(|day| (day.date.as_str(), day.single_revenue))
            .collect();
        assert_eq!(dates, vec![("1970-01-01", 1_000), ("1970-01-02", 2_000)]);
    }

    #[test]
    fn trims_monthly_history_to_available_data() {
        let conn = open_migrated_db();
        let end = 800 * DAY_SECONDS;

        // Only 45 days of history exist, so buckets beyond that shouldn't
        // appear even though the growth window covers a much wider range.
        insert_single_payment(&conn, "p_5d", end - 5 * DAY_SECONDS, 5_000);
        insert_single_payment(&conn, "p_45d", end - 45 * DAY_SECONDS, 1_000);

        let response =
            build_payment_activity_response(&conn, 0, end, &utc(), &SegmentFilters::default())
                .expect("build payment activity response");

        assert_eq!(response.monthly_revenue.len(), 2);
        assert_eq!(response.monthly_revenue[0].months_ago, 0);
        assert_eq!(response.monthly_revenue[0].revenue, 5_000);
        assert_eq!(response.monthly_revenue[1].months_ago, 1);
        assert_eq!(response.monthly_revenue[1].revenue, 1_000);
    }

    #[test]
    fn buckets_monthly_revenue_into_trailing_30_day_windows() {
        let conn = open_migrated_db();
        let end = 800 * DAY_SECONDS;

        insert_single_payment(&conn, "p_5d", end - 5 * DAY_SECONDS, 100_000);
        insert_single_payment(&conn, "p_45d", end - 45 * DAY_SECONDS, 40_000);
        insert_single_payment(&conn, "p_100d", end - 100 * DAY_SECONDS, 30_000);
        insert_single_payment(&conn, "p_200d", end - 200 * DAY_SECONDS, 20_000);
        insert_single_payment(&conn, "p_400d", end - 400 * DAY_SECONDS, 10_000);

        let response =
            build_payment_activity_response(&conn, 0, end, &utc(), &SegmentFilters::default())
                .expect("build payment activity response");

        // 12 monthly buckets: index 0 (days 0-29) has the 5-day-old payment,
        // index 1 (30-59) has the 45-day-old one, index 3 (90-119) the
        // 100-day-old one, and index 6 (180-209) the 200-day-old one. The
        // 400-day-old payment falls outside the 12-month history window.
        assert_eq!(response.monthly_revenue.len(), 12);
        let revenue_at = |months_ago: i64| {
            response
                .monthly_revenue
                .iter()
                .find(|entry| entry.months_ago == months_ago)
                .map(|entry| entry.revenue)
                .unwrap_or(0)
        };
        assert_eq!(revenue_at(0), 100_000);
        assert_eq!(revenue_at(1), 40_000);
        assert_eq!(revenue_at(3), 30_000);
        assert_eq!(revenue_at(6), 20_000);
    }

    #[test]
    fn segment_filters_narrow_heatmap_but_not_growth_or_monthly_totals() {
        let conn = open_migrated_db();
        let path_id = insert_path(&conn);
        let end = 800 * DAY_SECONDS;

        // A payment attributed to a US session, which the segment filter keeps.
        let us_visitor = insert_landing(&conn, path_id, 1, end - 5 * DAY_SECONDS, "US");
        insert_single_payment_for_visitor(
            &conn,
            "p_us",
            end - 5 * DAY_SECONDS,
            100_000,
            us_visitor,
        );

        // A payment attributed to a CA session, which the segment filter excludes.
        let ca_visitor = insert_landing(&conn, path_id, 2, end - 10 * DAY_SECONDS, "CA");
        insert_single_payment_for_visitor(
            &conn,
            "p_ca",
            end - 10 * DAY_SECONDS,
            50_000,
            ca_visitor,
        );

        let segments = SegmentFilters {
            countries: vec!["US".to_string()],
            ..SegmentFilters::default()
        };

        let filtered = build_payment_activity_response(&conn, 0, end, &utc(), &segments)
            .expect("build filtered response");
        let unfiltered =
            build_payment_activity_response(&conn, 0, end, &utc(), &SegmentFilters::default())
                .expect("build unfiltered response");

        // The purchase-activity heatmap only reflects the segment-matching payment.
        let filtered_total: i64 = filtered.days.iter().map(|day| day.single_revenue).sum();
        assert_eq!(filtered_total, 100_000);
        let unfiltered_total: i64 = unfiltered.days.iter().map(|day| day.single_revenue).sum();
        assert_eq!(unfiltered_total, 150_000);

        // Monthly revenue always reflects the unfiltered totals.
        assert_eq!(filtered.monthly_revenue, unfiltered.monthly_revenue);
        assert_eq!(filtered.monthly_revenue[0].revenue, 150_000);
    }
}
