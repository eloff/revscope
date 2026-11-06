//! The dashboard overview: revenue, MRR, churn and session metrics bucketed
//! over the selected period, each compared with the 30 days before it.

mod breakdowns;
mod buckets;
mod changes;
mod demo_cache;
mod mrr;
mod revenue;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::Serialize;

use crate::auth::AuthError;
use crate::authorize::AuthorizedProject;
use crate::comparison;
use crate::db::{ProjectConnection, open_project_database_read};
use crate::ga_import::ImportedDays;
use crate::metrics::{
    MrrGrowthRates, WilsonInterval, conversion_rate_wilson_interval, mrr_growth_rates,
};
use crate::query_params::parse_report_query;
use crate::segment_filter::{SegmentFilters, parse_segment_filters};
use crate::server_context::ServerContext;
use crate::timezone::{self, BucketGranularity, ProjectTimezone, parse_bucket_granularity};
use crate::util::seconds_since_epoch;

use breakdowns::{OverviewBreakdowns, build_overview_breakdowns};
use buckets::{OverviewBucket, add_imported_session_metrics, populate_session_metrics};
use changes::{
    MetricChanges, OverviewHoverChanges, OverviewTotals, OverviewTotalsChanges, PeriodMetrics,
};
pub(crate) use demo_cache::{
    DEMO_OVERVIEW_CACHE_ENTRIES, DemoOverviewResponseCache, respond_overview_json,
    take_cached_demo_overview,
};
use mrr::{fetch_mrr, populate_churn_metrics};
use revenue::{RevenuePeriodTotals, populate_revenue_metrics};

#[derive(Debug, Serialize)]
pub(crate) struct OverviewResponse {
    granularity: &'static str,
    /// Monthly recurring revenue as of the period's end timestamp.
    mrr: i64,
    /// Monthly recurring revenue as of the period's start timestamp (= previous period's end).
    previous_mrr: i64,
    /// Monthly and annualized MRR growth normalized from net MRR over the selected period.
    #[serde(skip_serializing_if = "Option::is_none")]
    mrr_growth: Option<MrrGrowthRates>,
    current: RevenuePeriodTotals,
    /// Raw revenue totals for the comparison window (the 30 days before the
    /// selected range). Used only for rate comparisons (average revenue per
    /// conversion, average time to purchase) where the normalization factor
    /// cancels; normalized counts/flows are exposed via [`Self::previous_totals`]
    /// and [`Self::previous_metrics`].
    previous: RevenuePeriodTotals,
    /// Display-ready metric tile values for the selected period.
    current_metrics: PeriodMetrics,
    /// Display-ready metric tile values for the normalized previous period.
    previous_metrics: PeriodMetrics,
    /// Normalized previous-period bucket totals, used for hover-card breakdowns.
    previous_totals: OverviewTotals,
    /// Period-over-period changes for hover-card bucket-total breakdown lines.
    totals_changes: OverviewTotalsChanges,
    /// Period-over-period changes for derived hover-card breakdown lines.
    hover_changes: OverviewHoverChanges,
    /// Period-over-period change ratios for each metric tile.
    changes: MetricChanges,
    /// 95% Wilson CI for the current period conversion rate.
    conversion_rate_ci: Option<WilsonInterval>,
    buckets: Vec<OverviewBucket>,
    /// Visitors and revenue by each filterable dimension, for sessions landed in
    /// the period.
    breakdowns: OverviewBreakdowns,
}

pub(crate) struct OverviewQuery {
    pub(crate) start: i64,
    pub(crate) end: i64,
    pub(crate) bucket: BucketGranularity,
    pub(crate) segments: SegmentFilters,
}

pub(crate) fn parse_overview_query(target: &str) -> Result<OverviewQuery, &'static str> {
    let mut bucket = BucketGranularity::default();
    let range = parse_report_query(target, |key, value| {
        if key == "bucket" {
            bucket = parse_bucket_granularity(value)?;
        }
        Ok(())
    })?;
    let (start, end) = range.validate()?;
    Ok(OverviewQuery {
        start,
        end,
        bucket,
        segments: parse_segment_filters(target),
    })
}

/// Serves overview for a project the router already authorized.
pub(crate) fn overview(
    server_context: Arc<ServerContext>,
    project: &AuthorizedProject,
    query: &OverviewQuery,
) -> Result<OverviewResponse, AuthError> {
    let timezone = timezone::project_timezone(server_context.clone(), project.key());
    let conn = open_project_database_read(server_context.as_ref(), project.id())?;
    build_overview_response_with_bucket(
        &conn,
        query.start,
        query.end,
        query.bucket,
        &timezone,
        &query.segments,
    )
}

/// Builds overview metrics from an open project database connection using the
/// requested bucket size, with bucket boundaries aligned to `timezone`.
pub(crate) fn build_overview_response_with_bucket(
    conn: &ProjectConnection,
    start: i64,
    end: i64,
    bucket: BucketGranularity,
    timezone: &ProjectTimezone,
    segments: &SegmentFilters,
) -> Result<OverviewResponse, AuthError> {
    build_overview_response_at(
        conn,
        start,
        end,
        bucket,
        timezone,
        seconds_since_epoch() as i64,
        segments,
    )
}

/// Inner builder with an explicit `now` so the normalization factor for periods
/// that include today is deterministic in tests.
///
/// The country segment filter scopes visitor-level metrics (session counts,
/// bounce, revenue attributed to a landing). MRR and churn are subscription-level
/// aggregates with no landing/visitor country, so they are intentionally left
/// unfiltered.
fn build_overview_response_at(
    conn: &ProjectConnection,
    start: i64,
    end: i64,
    bucket: BucketGranularity,
    timezone: &ProjectTimezone,
    now: i64,
    segments: &SegmentFilters,
) -> Result<OverviewResponse, AuthError> {
    let granularity = bucket.label();
    // The comparison window is always the 30 days immediately before the range.
    let previous_start = comparison::previous_period_start(start);
    let normalization_factor = comparison::normalization_factor(start, end, now);

    // One boundary list spanning both windows, so a row only has to be located
    // once no matter which window it lands in.
    let bucket_bounds = timezone.buckets(previous_start, end, bucket);
    let mut buckets: BTreeMap<i64, OverviewBucket> = bucket_bounds
        .starts()
        .iter()
        .map(|start| (*start, OverviewBucket::empty(*start)))
        .collect();

    populate_session_metrics(
        conn,
        &mut buckets,
        &bucket_bounds,
        previous_start,
        end,
        segments,
    )?;
    if let Some(days) = ImportedDays::for_report(conn, timezone, previous_start, end, segments)? {
        add_imported_session_metrics(conn, &mut buckets, &bucket_bounds, timezone, &days)?;
    }
    let revenue_metrics =
        populate_revenue_metrics(conn, &mut buckets, &bucket_bounds, start, end, segments)?;
    populate_churn_metrics(conn, &mut buckets, &bucket_bounds, previous_start, end)?;
    let mrr = fetch_mrr(conn, end)?;
    let previous_mrr = fetch_mrr(conn, start)?;

    // The bucket holding `start` is the first of the selected period; everything
    // before it belongs to the comparison window.
    let split_at = bucket_bounds.bucket_for_second(start).unwrap_or(start);
    let mut current_buckets = Vec::new();
    let mut previous_buckets = Vec::new();
    for (bucket_start, bucket) in buckets {
        if bucket_start < split_at {
            previous_buckets.push(bucket);
        } else {
            current_buckets.push(bucket);
        }
    }

    let current_totals = OverviewTotals::from_buckets(&current_buckets);
    let previous_totals_raw = OverviewTotals::from_buckets(&previous_buckets);
    let previous_totals = previous_totals_raw.scaled(normalization_factor);

    let current_metrics =
        PeriodMetrics::from_totals(&current_totals, &revenue_metrics.current, mrr);
    // Previous rates are computed from the raw 30-day window (the factor cancels),
    // then the additive flow metrics are normalized to the current period length.
    let previous_metrics = PeriodMetrics::from_totals(
        &previous_totals_raw,
        &revenue_metrics.previous,
        previous_mrr,
    )
    .scaled_flows(normalization_factor);
    let changes = MetricChanges::between(&current_metrics, &previous_metrics);
    let totals_changes = OverviewTotalsChanges::between(&current_totals, &previous_totals);
    let hover_changes = OverviewHoverChanges::between(
        &current_totals,
        &previous_totals,
        &revenue_metrics.current,
        &revenue_metrics.previous,
    );

    let conversion_rate_ci = conversion_rate_wilson_interval(
        revenue_metrics.current.converting_visitors as f64,
        current_totals.tracked_visitors(),
    );
    let elapsed_seconds = comparison::current_elapsed_seconds(start, end, now);
    let mrr_growth = mrr_growth_rates(mrr, previous_mrr, elapsed_seconds);
    let imported_days = ImportedDays::for_report(conn, timezone, start, end, segments)?;
    let breakdowns = build_overview_breakdowns(conn, start, end, segments, imported_days.as_ref())?;

    Ok(OverviewResponse {
        granularity,
        mrr,
        previous_mrr,
        mrr_growth,
        current: revenue_metrics.current,
        previous: revenue_metrics.previous,
        current_metrics,
        previous_metrics,
        previous_totals,
        totals_changes,
        hover_changes,
        changes,
        conversion_rate_ci,
        buckets: current_buckets,
        breakdowns,
    })
}
