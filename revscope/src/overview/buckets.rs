use std::collections::BTreeMap;

use fxhash::FxHashMap;
use rusqlite::params_from_iter;
use serde::Serialize;

use crate::auth::{AuthError, map_row_error};
use crate::db::ProjectConnection;
use crate::ga_import::ImportedDays;
use crate::segment_filter::SegmentFilters;
use crate::timezone::{Buckets, ProjectTimezone};

pub(super) use crate::util::NANOS_PER_SECOND;

#[derive(Debug, Default, Serialize)]
pub(crate) struct OverviewBucket {
    pub(super) start: i64,
    /// Sessions whose landing pageview falls in this bucket. Additive across buckets.
    pub(super) visitors: u32,
    /// The visitors from imported data, who have no purchases to convert into, so
    /// conversion and revenue per visitor are shares of the rest.
    pub(super) imported_visitors: u32,
    /// Distinct buyers with a first purchase (new subscription or single payment) in this
    /// bucket, deduplicated across the period by landing or customer.
    pub(super) converting_visitors: u32,
    /// First purchases in this bucket that could not be linked to a tracked session or
    /// customer identity. Included in [`Self::converting_visitors`].
    pub(super) unattributed_converting_visitors: u32,
    /// Count of renewal subscription payments in this bucket.
    pub(super) recurring_subscriptions_count: u32,
    /// Count of renewal subscription payments billed annually (`interval_unit = year`).
    pub(super) recurring_annual_subscriptions_count: u32,
    /// Count of first-time subscription payments in this bucket.
    pub(super) new_subscriptions_count: u32,
    /// Count of first-time subscription payments billed annually (`interval_unit = year`).
    pub(super) new_annual_subscriptions_count: u32,
    /// Subscriptions canceled or ended with no replacement within 24 hours for the same customer.
    pub(super) churn: u32,
    /// Count of non-subscription (single) payment events in this bucket.
    pub(super) other_payments_count: u32,
    /// Count of refund and chargeback payment events in this bucket.
    pub(super) refunds_chargebacks_count: u32,
    /// Count of sessions landed in this bucket with exactly one pageview.
    pub(super) bounced_sessions: u32,
    pub(super) recurring_subscriptions: i64,
    /// Gross payment revenue from first-time subscription payments. Used for chart stacking.
    pub(super) new_subscriptions: i64,
    /// MRR added by new subscriptions in this bucket, from the recurring price
    /// they signed up on.
    pub(super) new_subscriptions_mrr: i64,
    /// MRR removed by subscriptions that churned in this bucket, from the
    /// recurring price in effect when they churned.
    pub(super) churn_mrr: i64,
    /// Non-subscription (single) payment revenue in this bucket.
    pub(super) other_revenue: i64,
    pub(super) refunds: i64,
    pub(super) chargebacks: i64,
    /// Sum of session durations (seconds) for sessions landed in this bucket.
    pub(super) session_time_total: i64,
}

impl OverviewBucket {
    pub(super) fn empty(start: i64) -> Self {
        Self {
            start,
            ..Self::default()
        }
    }
}

/// Adds imported days to the buckets they start in. GA counts a session as bounced
/// when it wasn't engaged, which is our single page under ten seconds.
pub(super) fn add_imported_session_metrics(
    conn: &ProjectConnection,
    buckets: &mut BTreeMap<i64, OverviewBucket>,
    bucket_bounds: &Buckets,
    timezone: &ProjectTimezone,
    days: &ImportedDays,
) -> Result<(), AuthError> {
    for day in days.daily_totals(conn)? {
        let Some(bucket) = day
            .date
            .and_then(|date| timezone.day_start(date))
            .and_then(|second| bucket_bounds.bucket_for_second(second))
            .and_then(|bucket| buckets.get_mut(&bucket))
        else {
            continue;
        };
        let bounced = day.sessions.saturating_sub(day.engaged_sessions).max(0);
        let sessions = u32::try_from(day.sessions).unwrap_or(u32::MAX);
        bucket.visitors = bucket.visitors.saturating_add(sessions);
        bucket.imported_visitors = bucket.imported_visitors.saturating_add(sessions);
        bucket.bounced_sessions = bucket
            .bounced_sessions
            .saturating_add(u32::try_from(bounced).unwrap_or(u32::MAX));
        bucket.session_time_total += day.engagement_seconds;
    }
    Ok(())
}

#[derive(Debug, Default)]
struct SessionBucketMetrics {
    visitors: u32,
    session_time_total: i64,
    bounced_sessions: u32,
}

pub(super) fn populate_session_metrics(
    conn: &ProjectConnection,
    buckets: &mut BTreeMap<i64, OverviewBucket>,
    bucket_bounds: &Buckets,
    start: i64,
    end: i64,
    segments: &SegmentFilters,
) -> Result<(), AuthError> {
    // It turns out to be much faster to do the aggregation in Rust than in SQL here
    let mut aggregated: FxHashMap<i64, SessionBucketMetrics> = FxHashMap::default();

    let country_filter = segments.country_in_clause("l.country", 3);
    let session_filter = segments.session_filter_clause("l.visitor_timestamp_id", None);
    let start_ns = start * NANOS_PER_SECOND;
    let end_ns = end * NANOS_PER_SECOND;
    let mut sql_params: Vec<&dyn rusqlite::types::ToSql> = vec![&start_ns, &end_ns];
    segments.append_country_params(&mut sql_params);
    conn.with_query(
        &format!(
            "SELECT l.visitor_timestamp_id,
                l.session_duration_seconds,
                l.session_page_count
         FROM landings l
         WHERE l.visitor_timestamp_id >= ?1
           AND l.visitor_timestamp_id < ?2{country_filter}{session_filter}"
        ),
        params_from_iter(sql_params),
        |mut rows| {
            while let Some(row) = rows.next()? {
                let visitor_timestamp_id: i64 = row.get(0)?;
                let session_duration_seconds: i64 = row.get(1)?;
                let session_page_count: i32 = row.get(2)?;
                let Some(bucket) = bucket_bounds.bucket_for_nanosecond(visitor_timestamp_id) else {
                    continue;
                };

                let metrics = aggregated.entry(bucket).or_default();
                metrics.visitors = metrics.visitors.saturating_add(1);
                metrics.session_time_total += session_duration_seconds;
                if session_page_count == 1 && session_duration_seconds < 10 {
                    metrics.bounced_sessions = metrics.bounced_sessions.saturating_add(1);
                }
            }
            Ok(())
        },
    )
    .map_err(map_row_error)?;

    for (bucket, metrics) in aggregated {
        if let Some(entry) = buckets.get_mut(&bucket) {
            entry.visitors = metrics.visitors;
            entry.session_time_total = metrics.session_time_total;
            entry.bounced_sessions = metrics.bounced_sessions;
        }
    }

    Ok(())
}
