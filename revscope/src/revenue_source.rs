use std::sync::Arc;

use compact_str::CompactString;
use fxhash::{FxHashMap, FxHashSet};
use rusqlite::params_from_iter;
use rusqlite::types::ToSql;
use serde::Serialize;

use crate::auth::{AuthError, map_row_error};
use crate::authorize::AuthorizedProject;
use crate::comparison::{self, Change};
use crate::db::{ProjectConnection, SqlCompactStr, open_project_database_read};
use crate::metrics::{
    self, WilsonInterval, conversion_rate_wilson_interval, conversion_rate_wilson_lower_bound,
};
use crate::page_revenue::page_revenue_cte;
use crate::query_params::parse_date_range_query;
use crate::referrer::{
    ReferrerType, SourceGroup, group_for_referrer_type, source_group_for_landing,
};
use crate::segment_filter::{
    LandingAttribute, SegmentDimension, SegmentFilters, parse_segment_filters,
};
use crate::server_context::ServerContext;
use crate::util::{NANOS_PER_SECOND, seconds_since_epoch};

const TOP_LANDING_PATHS: usize = 5;
const TOP_CAMPAIGN_REFERRERS: usize = 5;

/// Display-ready metrics shared by every revenue-source row and group summary.
///
/// Counts and `revenue` are raw current-period values; rates and intervals are
/// derived from the current period. The `*_change` fields compare against the
/// prior 30 days: additive quantities (revenue, visitors, conversions) are
/// normalized to the current period length, while rate changes (revenue/visitor,
/// conversion rate) are compared directly.
#[derive(Debug, Serialize)]
pub(crate) struct SourceMetrics {
    pub(crate) visitors: i64,
    /// First visits with attributed payment revenue.
    pub(crate) converting_visitors: i64,
    /// Sum of qualifying `payments.amount` (thousandths) attributed to this row/group.
    /// Excludes subscription renewals.
    pub(crate) revenue: i64,
    /// Converting visitors ÷ visitors.
    pub(crate) conversion_rate: f64,
    /// Revenue ÷ visitors (thousandths).
    pub(crate) revenue_per_visitor: f64,
    /// Lower bound of the 95% Wilson interval, used for ranking.
    pub(crate) conversion_lower_bound: f64,
    /// 95% Wilson confidence interval for the conversion rate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) conversion_ci: Option<WilsonInterval>,
    pub(crate) revenue_change: Change,
    pub(crate) visitors_change: Change,
    pub(crate) conversions_change: Change,
    pub(crate) revenue_per_visitor_change: Change,
    pub(crate) conversion_rate_change: Change,
}

/// Landing page path with visit count for a revenue source bucket.
#[derive(Debug, Serialize)]
pub(crate) struct LandingPathCount {
    /// Stable `paths.id` for linking to page/landing-page rows.
    pub(crate) id: i64,
    pub(crate) path: CompactString,
    pub(crate) count: i64,
}

/// One referrer contributing visits to a campaign, ranked by visit count.
#[derive(Debug, Serialize)]
pub(crate) struct CampaignReferrerCount {
    /// Stable `referrers.id`. Absent for direct (no-referrer) visits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) id: Option<i64>,
    /// Normalized referrer URL. Absent for direct visits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) referrer: Option<CompactString>,
    pub(crate) count: i64,
}

/// A campaign revenue source: first visits grouped by the `campaigns` row
/// (the decoded UTM/attribution tuple) regardless of referrer.
#[derive(Debug, Serialize)]
pub(crate) struct RevenueSourceCampaign {
    pub(crate) id: i64,
    pub(crate) source: CompactString,
    pub(crate) medium: CompactString,
    pub(crate) campaign: CompactString,
    /// Ad network the campaign was attributed to (e.g. google_ads, meta_ads).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) network: Option<CompactString>,
    /// Optional human-readable label set by the user in the dashboard.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) display_name: Option<CompactString>,
    /// Channel bucket for this row (always campaigns).
    pub(crate) group: SourceGroup,
    /// Display-ready counts, derived metrics, and period-over-period deltas.
    #[serde(flatten)]
    pub(crate) metrics: SourceMetrics,
    /// Top landing page paths for visits in this source, ordered by visit count.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) landing_paths: Vec<LandingPathCount>,
    /// Top referrers for visits in this campaign, ordered by visit count.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) referrers: Vec<CampaignReferrerCount>,
}

/// A referrer revenue source: first visits with a given referrer (and no
/// campaign), or — when `referrer` is `None` — direct first visits with no
/// referrer and no campaign.
#[derive(Debug, Serialize)]
pub(crate) struct RevenueSourceEntry {
    /// Stable `referrers.id` for non-direct referrer rows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) id: Option<i64>,
    /// Normalized referrer URL. `None` for direct (no referrer) visits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) referrer: Option<CompactString>,
    /// `landings.referrer_type` (search, ai, social, email, campaign, etc).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) referrer_type: Option<ReferrerType>,
    /// Channel bucket for visits in this bucket.
    pub(crate) group: SourceGroup,
    /// Display-ready counts, derived metrics, and period-over-period deltas.
    #[serde(flatten)]
    pub(crate) metrics: SourceMetrics,
    /// Top landing page paths for visits in this source, ordered by visit count.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) landing_paths: Vec<LandingPathCount>,
}

/// A single top-level revenue source. Campaigns are no longer nested under a
/// referrer; both a referrer (or direct) bucket and a campaign bucket are
/// independent top-level sources, serialized with a `kind` discriminator.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum RevenueSource {
    Referrer(RevenueSourceEntry),
    Campaign(RevenueSourceCampaign),
}

/// Display-ready summary metrics + deltas for one channel group (search, ai,
/// social, campaigns, referrers, direct). Mirrors a single grid group-summary
/// hover card; all derived values and changes are computed server-side.
#[derive(Debug, Serialize)]
pub(crate) struct GroupSummary {
    pub(crate) group: SourceGroup,
    #[serde(flatten)]
    pub(crate) metrics: SourceMetrics,
}

/// Per-group summaries for the requested window, with deltas versus the prior
/// 30 days (normalized). Keyed by channel group so the dashboard can render
/// group-summary hover cards without the raw previous period.
#[derive(Debug, Serialize)]
pub(crate) struct GroupSummaries {
    pub(crate) search: GroupSummary,
    pub(crate) ai: GroupSummary,
    pub(crate) social: GroupSummary,
    pub(crate) campaigns: GroupSummary,
    pub(crate) referrers: GroupSummary,
    pub(crate) direct: GroupSummary,
}

/// Per-period payload: ranked entries (each carrying derived metrics and
/// deltas) plus the channel totals the dashboard uses to synthesize per-group
/// "Other" rows.
#[derive(Debug, Serialize)]
pub(crate) struct RevenueSourcePeriod {
    /// Revenue sources across all landings, ranked by revenue: referrer
    /// buckets (referrer set, no campaign), the direct bucket (no referrer, no
    /// campaign), and campaign buckets (campaign set, regardless of referrer).
    pub(crate) entries: Vec<RevenueSource>,
    /// Total first visits in the period.
    pub(crate) total_visitors: i64,
    /// Sum of `payments.amount` attributed to all first-visits in the period.
    pub(crate) total_revenue: i64,
    /// First visits with no referrer (== direct.visitors).
    pub(crate) direct_visitors: i64,
    /// Revenue attributed to first-visits with no referrer (== direct.revenue).
    pub(crate) direct_revenue: i64,
    /// First visits with a referrer (sum across all referrers).
    pub(crate) referrer_visitors: i64,
    /// Revenue attributed to first-visits with a referrer.
    pub(crate) referrer_revenue: i64,
    /// First visits with `landings.campaign` set, regardless of referrer.
    pub(crate) campaign_visitors: i64,
    /// Revenue attributed to first-visits with `landings.campaign` set.
    pub(crate) campaign_revenue: i64,
    /// First visits in the search channel bucket.
    pub(crate) search_visitors: i64,
    pub(crate) search_revenue: i64,
    /// First visits in the AI assistant channel bucket.
    pub(crate) ai_visitors: i64,
    pub(crate) ai_revenue: i64,
    pub(crate) social_visitors: i64,
    pub(crate) social_revenue: i64,
    /// First visits classified as campaigns (paid/UTM channels, not `campaign_visitors`).
    pub(crate) campaigns_visitors: i64,
    pub(crate) campaigns_revenue: i64,
    /// First visits with a referrer in the generic referrers bucket.
    pub(crate) referrers_group_visitors: i64,
    pub(crate) referrers_group_revenue: i64,
}

#[derive(Debug, Serialize)]
pub(crate) struct RevenueSourceResponse {
    /// Ranked entries and channel totals for the requested window. Each entry
    /// carries server-computed derived metrics and period-over-period deltas
    /// (versus the prior 30 days, normalized).
    pub(crate) current: RevenueSourcePeriod,
    /// Per-group summary metrics and deltas for the requested window.
    pub(crate) groups: GroupSummaries,
}

pub(crate) struct RevenueSourceQuery {
    pub(crate) start: i64,
    pub(crate) end: i64,
    pub(crate) segments: SegmentFilters,
}

pub(crate) fn parse_revenue_source_query(target: &str) -> Result<RevenueSourceQuery, &'static str> {
    let (start, end) = parse_date_range_query(target)?;
    Ok(RevenueSourceQuery {
        start,
        end,
        segments: parse_segment_filters(target),
    })
}

pub(crate) fn revenue_source(
    server_context: Arc<ServerContext>,
    project: &AuthorizedProject,
    query: RevenueSourceQuery,
) -> Result<RevenueSourceResponse, AuthError> {
    fetch_revenue_source(server_context, project.id(), &query)
}

fn fetch_revenue_source(
    server_context: Arc<ServerContext>,
    project_id: &str,
    query: &RevenueSourceQuery,
) -> Result<RevenueSourceResponse, AuthError> {
    let conn = open_project_database_read(server_context.as_ref(), project_id)
        .map_err(|error| AuthError::Db(error.to_string()))?;
    build_revenue_source_response(&conn, query)
}

/// Builds the revenue-sources response from an open project database connection.
///
/// Returns the requested window (`current`) with each entry carrying
/// display-ready derived metrics and period-over-period deltas (versus the
/// prior 30 days, normalized to the current period length), plus per-group
/// summaries. The raw previous period is consumed server-side and not returned.
pub(crate) fn build_revenue_source_response(
    conn: &ProjectConnection,
    query: &RevenueSourceQuery,
) -> Result<RevenueSourceResponse, AuthError> {
    build_revenue_source_response_at(conn, query, seconds_since_epoch() as i64)
}

/// Inner builder with an explicit `now` so the normalization factor is
/// deterministic in tests.
fn build_revenue_source_response_at(
    conn: &ProjectConnection,
    query: &RevenueSourceQuery,
    now: i64,
) -> Result<RevenueSourceResponse, AuthError> {
    let previous_start = comparison::previous_period_start(query.start);
    let factor = comparison::normalization_factor(query.start, query.end, now);

    let (current_totals, current_ranked, path_by_id) =
        fetch_revenue_source_raw(conn, query.start, query.end, &query.segments)?;
    let (previous_totals, previous_ranked, _) =
        fetch_revenue_source_raw(conn, previous_start, query.start, &query.segments)?;

    // Index previous buckets by key so current rows can find their match.
    let previous_by_key: FxHashMap<BucketKey, Counts> = previous_ranked
        .iter()
        .map(|(key, bucket)| {
            (
                *key,
                Counts {
                    visitors: bucket.visitors,
                    converting_visitors: bucket.converting_visitors,
                    revenue: bucket.revenue,
                },
            )
        })
        .collect();

    let entries = current_ranked
        .into_iter()
        .map(|(key, bucket)| {
            let current = Counts {
                visitors: bucket.visitors,
                converting_visitors: bucket.converting_visitors,
                revenue: bucket.revenue,
            };
            let previous = previous_by_key.get(&key).copied();
            let metrics = source_metrics(current, previous, factor);
            build_entry(key, bucket, metrics, &path_by_id)
        })
        .collect();

    let current = RevenueSourcePeriod {
        entries,
        total_visitors: current_totals.total_visitors,
        total_revenue: current_totals.total_revenue,
        direct_visitors: current_totals.direct_visitors,
        direct_revenue: current_totals.direct_revenue,
        referrer_visitors: current_totals.referrer_visitors,
        referrer_revenue: current_totals.referrer_revenue,
        campaign_visitors: current_totals.campaign_visitors,
        campaign_revenue: current_totals.campaign_revenue,
        search_visitors: current_totals.search_visitors,
        search_revenue: current_totals.search_revenue,
        ai_visitors: current_totals.ai_visitors,
        ai_revenue: current_totals.ai_revenue,
        social_visitors: current_totals.social_visitors,
        social_revenue: current_totals.social_revenue,
        campaigns_visitors: current_totals.campaigns_visitors,
        campaigns_revenue: current_totals.campaigns_revenue,
        referrers_group_visitors: current_totals.referrers_group_visitors,
        referrers_group_revenue: current_totals.referrers_group_revenue,
    };

    let groups = build_group_summaries(&current_totals, &previous_totals, factor);

    Ok(RevenueSourceResponse { current, groups })
}

/// Computes display-ready metrics and deltas for a single revenue-source row or
/// group summary.
///
/// Change ratios compare against the prior 30 days: additive quantities are
/// normalized by `factor`, rates are compared directly.
fn source_metrics(current: Counts, previous: Option<Counts>, factor: f64) -> SourceMetrics {
    let visitors = current.visitors as f64;
    let converting = current.converting_visitors as f64;
    let revenue = current.revenue as f64;
    let conversion_rate = metrics::conversion_rate(converting, visitors);
    let revenue_per_visitor = metrics::revenue_per_visitor(revenue, visitors);
    let revenue_change =
        Change::breakdown_absolute(revenue, previous.map(|p| p.revenue as f64), factor);
    let visitors_change =
        Change::breakdown_absolute(visitors, previous.map(|p| p.visitors as f64), factor);
    let conversions_change = Change::breakdown_absolute(
        converting,
        previous.map(|p| p.converting_visitors as f64),
        factor,
    );
    let revenue_per_visitor_change = Change::breakdown_rate(
        revenue_per_visitor,
        previous.map(|p| metrics::revenue_per_visitor(p.revenue as f64, p.visitors as f64)),
    );
    let conversion_rate_change = Change::breakdown_rate(
        conversion_rate,
        previous.map(|p| metrics::conversion_rate(p.converting_visitors as f64, p.visitors as f64)),
    );
    SourceMetrics {
        visitors: current.visitors,
        converting_visitors: current.converting_visitors,
        revenue: current.revenue,
        conversion_rate,
        revenue_per_visitor,
        conversion_lower_bound: conversion_rate_wilson_lower_bound(converting, visitors),
        conversion_ci: conversion_rate_wilson_interval(converting, visitors),
        revenue_change,
        visitors_change,
        conversions_change,
        revenue_per_visitor_change,
        conversion_rate_change,
    }
}

/// Builds per-group summaries (current totals + derived metrics + deltas). Each
/// group always has a previous comparison (possibly zero counts).
fn build_group_summaries(
    current_totals: &TotalsRow,
    previous_totals: &TotalsRow,
    factor: f64,
) -> GroupSummaries {
    let summary = |group: SourceGroup| -> GroupSummary {
        let current = current_totals.group_counts(group);
        let previous = previous_totals.group_counts(group);
        GroupSummary {
            group,
            metrics: source_metrics(current, Some(previous), factor),
        }
    };
    GroupSummaries {
        search: summary(SourceGroup::Search),
        ai: summary(SourceGroup::Ai),
        social: summary(SourceGroup::Social),
        campaigns: summary(SourceGroup::Campaigns),
        referrers: summary(SourceGroup::Referrers),
        direct: summary(SourceGroup::Direct),
    }
}

#[derive(Debug, Default)]
struct TotalsRow {
    total_visitors: i64,
    total_revenue: i64,
    direct_visitors: i64,
    direct_revenue: i64,
    direct_converting: i64,
    referrer_visitors: i64,
    referrer_revenue: i64,
    campaign_visitors: i64,
    campaign_revenue: i64,
    search_visitors: i64,
    search_revenue: i64,
    search_converting: i64,
    ai_visitors: i64,
    ai_revenue: i64,
    ai_converting: i64,
    social_visitors: i64,
    social_revenue: i64,
    social_converting: i64,
    campaigns_visitors: i64,
    campaigns_revenue: i64,
    campaigns_converting: i64,
    referrers_group_visitors: i64,
    referrers_group_revenue: i64,
    referrers_group_converting: i64,
}

impl TotalsRow {
    /// Current (raw) counts for one channel group.
    fn group_counts(&self, group: SourceGroup) -> Counts {
        match group {
            SourceGroup::Search => Counts {
                visitors: self.search_visitors,
                converting_visitors: self.search_converting,
                revenue: self.search_revenue,
            },
            SourceGroup::Ai => Counts {
                visitors: self.ai_visitors,
                converting_visitors: self.ai_converting,
                revenue: self.ai_revenue,
            },
            SourceGroup::Social => Counts {
                visitors: self.social_visitors,
                converting_visitors: self.social_converting,
                revenue: self.social_revenue,
            },
            SourceGroup::Campaigns => Counts {
                visitors: self.campaigns_visitors,
                converting_visitors: self.campaigns_converting,
                revenue: self.campaigns_revenue,
            },
            SourceGroup::Referrers => Counts {
                visitors: self.referrers_group_visitors,
                converting_visitors: self.referrers_group_converting,
                revenue: self.referrers_group_revenue,
            },
            SourceGroup::Direct => Counts {
                visitors: self.direct_visitors,
                converting_visitors: self.direct_converting,
                revenue: self.direct_revenue,
            },
        }
    }
}

/// Raw aggregate counts for a bucket or group within one window.
#[derive(Debug, Default, Clone, Copy)]
struct Counts {
    visitors: i64,
    converting_visitors: i64,
    revenue: i64,
}

/// Identifies one of the three top-level buckets a landing maps to. Campaign
/// landings are keyed by `campaigns.id`, plain referrer landings by
/// `referrers.id`, and everything else collapses into the single direct bucket.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum BucketKey {
    Direct,
    Referrer(i64),
    Campaign(i64),
}

/// Per-bucket running aggregate plus the lookup fields (referrer URL / campaign
/// columns) needed to materialize a [`RevenueSource`], captured from the first
/// landing seen in the bucket.
#[derive(Default)]
struct BucketAggregate {
    visitors: i64,
    converting_visitors: i64,
    revenue: i64,
    referrer: Option<SqlCompactStr>,
    referrer_type: Option<ReferrerType>,
    source: Option<SqlCompactStr>,
    medium: Option<SqlCompactStr>,
    campaign: Option<SqlCompactStr>,
    network: Option<SqlCompactStr>,
    display_name: Option<SqlCompactStr>,
    path_counts: FxHashMap<i64, i64>,
    /// Visit counts by referrer id. `None` is direct (no referrer). Campaign buckets only.
    referrer_counts: FxHashMap<Option<i64>, ReferrerVisitCount>,
}

#[derive(Default)]
struct ReferrerVisitCount {
    referrer: Option<SqlCompactStr>,
    count: i64,
}

type RevenueSourceRaw = (
    TotalsRow,
    Vec<(BucketKey, BucketAggregate)>,
    FxHashMap<i64, SqlCompactStr>,
);

/// Fetches all revenue-source totals and entries across first-visit landings in
/// the window.
///
/// The query is intentionally a flat per-landing scan (joined to attributed
/// revenue and the referrer/campaign lookup rows); all bucketing, totals, and
/// ranking are computed in Rust. Path strings are resolved in a follow-up query
/// against the collected set of landing `path_id`s.
///
/// Every landing falls into exactly one bucket:
/// - live campaign set -> a [`RevenueSource::Campaign`] keyed by `campaigns.id`
///   (regardless of referrer); a deleted campaign counts as no campaign,
/// - otherwise referrer set -> a [`RevenueSource::Referrer`] keyed by
///   `referrers.id`,
/// - otherwise -> the single direct [`RevenueSource::Referrer`] (no referrer).
fn fetch_revenue_source_raw(
    conn: &ProjectConnection,
    start: i64,
    end: i64,
    segments: &SegmentFilters,
) -> Result<RevenueSourceRaw, AuthError> {
    let country_filter = segments.country_in_clause("l.country", 3);
    // The revenue-sources view lists sources, so it intentionally ignores the
    // source segment filter (users add sources from this very view).
    let session_filter =
        // The view's channel summaries are its own breakdown by channel, so the
        // channel filter is exempt along with the source filter.
        segments
            .without_attributes(&[LandingAttribute::Channel])
            .session_filter_clause("l.visitor_timestamp_id", Some(SegmentDimension::Source));
    // Same visit window as the landings filter (?1/?2); first subscription charge only.
    let page_revenue = page_revenue_cte(1, 2, true);
    let sql = format!(
        "WITH {page_revenue}
         SELECT
             CASE WHEN c.deleted_at IS NULL THEN l.campaign END AS campaign_id,
             l.referrer AS referrer_id,
             l.referrer_type AS referrer_type,
             COALESCE(rev.amount, 0) AS amount,
             r.referrer AS referrer,
             c.source, c.medium, c.campaign, c.network, c.display_name,
             pv.path_id AS landing_path_id
         FROM landings l
         LEFT JOIN page_revenue rev ON rev.visitor_timestamp_id = l.visitor_timestamp_id
         LEFT JOIN referrers r ON r.id = l.referrer
         LEFT JOIN campaigns c ON c.id = l.campaign
         LEFT JOIN pageviews pv ON pv.timestamp_id = l.visitor_timestamp_id
         WHERE l.visitor_timestamp_id >= ?1
           AND l.visitor_timestamp_id < ?2{country_filter}{session_filter}"
    );
    let start_ns = start * NANOS_PER_SECOND;
    let end_ns = end * NANOS_PER_SECOND;
    let mut sql_params: Vec<&dyn ToSql> = vec![&start_ns, &end_ns];
    segments.append_country_params(&mut sql_params);
    let (totals, ranked, path_ids) = conn
        .with_query(&sql, params_from_iter(sql_params), |mut rows| {
            let mut totals = TotalsRow::default();
            let mut buckets: FxHashMap<BucketKey, BucketAggregate> = FxHashMap::default();
            let mut path_ids: FxHashSet<i64> = FxHashSet::default();

            while let Some(row) = rows.next()? {
                let campaign_id: Option<i64> = row.get(0)?;
                let referrer_id: Option<i64> = row.get(1)?;
                let referrer_type = row
                    .get_ref(2)?
                    .as_str_or_null()?
                    .and_then(ReferrerType::from_stored);
                let amount: i64 = row.get(3)?;
                let landing_path_id: Option<i64> = row.get(10)?;

                let has_campaign = campaign_id.is_some();
                let has_referrer = referrer_id.is_some();
                let converting = i64::from(amount > 0);

                // Totals accumulate across every landing, independent of how the
                // entries are bucketed below.
                totals.total_visitors += 1;
                totals.total_revenue += amount;
                if has_referrer {
                    totals.referrer_visitors += 1;
                    totals.referrer_revenue += amount;
                } else {
                    totals.direct_visitors += 1;
                    totals.direct_revenue += amount;
                    totals.direct_converting += converting;
                }
                if has_campaign {
                    totals.campaign_visitors += 1;
                    totals.campaign_revenue += amount;
                }
                match source_group_for_landing(has_campaign, has_referrer, referrer_type) {
                    SourceGroup::Search => {
                        totals.search_visitors += 1;
                        totals.search_revenue += amount;
                        totals.search_converting += converting;
                    }
                    SourceGroup::Ai => {
                        totals.ai_visitors += 1;
                        totals.ai_revenue += amount;
                        totals.ai_converting += converting;
                    }
                    SourceGroup::Social => {
                        totals.social_visitors += 1;
                        totals.social_revenue += amount;
                        totals.social_converting += converting;
                    }
                    SourceGroup::Campaigns => {
                        totals.campaigns_visitors += 1;
                        totals.campaigns_revenue += amount;
                        totals.campaigns_converting += converting;
                    }
                    SourceGroup::Referrers => {
                        totals.referrers_group_visitors += 1;
                        totals.referrers_group_revenue += amount;
                        totals.referrers_group_converting += converting;
                    }
                    SourceGroup::Direct => {}
                }

                // Assign the landing to its single top-level bucket and fold its
                // counts into the running aggregate.
                let key = if has_campaign {
                    BucketKey::Campaign(campaign_id.unwrap_or_default())
                } else if has_referrer {
                    BucketKey::Referrer(referrer_id.unwrap_or_default())
                } else {
                    BucketKey::Direct
                };

                let bucket = buckets.entry(key).or_default();
                if bucket.visitors == 0 {
                    // First landing in this bucket carries the lookup columns; all
                    // landings in a bucket share the same referrer/campaign row.
                    bucket.referrer_type = referrer_type;
                    match key {
                        BucketKey::Campaign(_) => {
                            bucket.source = row.get(5)?;
                            bucket.medium = row.get(6)?;
                            bucket.campaign = row.get(7)?;
                            bucket.network = row.get(8)?;
                            bucket.display_name = row.get(9)?;
                        }
                        BucketKey::Direct | BucketKey::Referrer(_) => {
                            bucket.referrer = row.get(4)?;
                        }
                    }
                }
                if let BucketKey::Campaign(_) = key {
                    let visits = bucket.referrer_counts.entry(referrer_id).or_default();
                    if visits.count == 0 {
                        visits.referrer = row.get(4)?;
                    }
                    visits.count += 1;
                }
                bucket.visitors += 1;
                bucket.converting_visitors += converting;
                bucket.revenue += amount;
                if let Some(path_id) = landing_path_id {
                    path_ids.insert(path_id);
                    *bucket.path_counts.entry(path_id).or_default() += 1;
                }
            }

            let mut ranked: Vec<(BucketKey, BucketAggregate)> = buckets.into_iter().collect();
            // Mirror the previous SQL ordering: highest revenue first, then most
            // visitors, with a stable tie-break on bucket kind and id.
            ranked.sort_by(|(left_key, left), (right_key, right)| {
                right
                    .revenue
                    .cmp(&left.revenue)
                    .then(right.visitors.cmp(&left.visitors))
                    .then(left_key.cmp(right_key))
            });

            Ok((totals, ranked, path_ids))
        })
        .map_err(map_row_error)?;

    let path_by_id = fetch_paths_by_id(conn, &path_ids)?;
    Ok((totals, ranked, path_by_id))
}

fn fetch_paths_by_id(
    conn: &ProjectConnection,
    ids: &FxHashSet<i64>,
) -> Result<FxHashMap<i64, SqlCompactStr>, AuthError> {
    if ids.is_empty() {
        return Ok(FxHashMap::default());
    }

    let mut id_list: Vec<i64> = ids.iter().copied().collect();
    id_list.sort_unstable();
    let placeholders = (1..=id_list.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!("SELECT id, path FROM paths WHERE id IN ({placeholders})");

    conn.with_query(&sql, params_from_iter(id_list.iter()), |mut rows| {
        let mut path_by_id = FxHashMap::default();
        while let Some(row) = rows.next()? {
            let id: i64 = row.get(0)?;
            let path: SqlCompactStr = row.get(1)?;
            path_by_id.insert(id, path);
        }
        Ok(path_by_id)
    })
    .map_err(map_row_error)
}

/// The channel group a bucket belongs to (for entry grouping and summaries).
fn group_for_bucket(key: BucketKey, bucket: &BucketAggregate) -> SourceGroup {
    match key {
        BucketKey::Campaign(_) => SourceGroup::Campaigns,
        BucketKey::Direct => SourceGroup::Direct,
        BucketKey::Referrer(_) => {
            if bucket.referrer.is_none() {
                SourceGroup::Direct
            } else {
                group_for_referrer_type(bucket.referrer_type)
            }
        }
    }
}

/// Materializes a ranked entry from an aggregated bucket and its computed metrics.
fn build_entry(
    key: BucketKey,
    bucket: BucketAggregate,
    metrics: SourceMetrics,
    path_by_id: &FxHashMap<i64, SqlCompactStr>,
) -> RevenueSource {
    let group = group_for_bucket(key, &bucket);
    let landing_paths = landing_paths_from_counts(bucket.path_counts, path_by_id);
    match key {
        BucketKey::Campaign(id) => {
            let referrers = campaign_referrers_from_counts(bucket.referrer_counts);
            RevenueSource::Campaign(RevenueSourceCampaign {
                id,
                source: compact_str_or_default(bucket.source),
                medium: compact_str_or_default(bucket.medium),
                campaign: compact_str_or_default(bucket.campaign),
                network: compact_opt(bucket.network),
                display_name: compact_opt(bucket.display_name),
                group: SourceGroup::Campaigns,
                metrics,
                landing_paths,
                referrers,
            })
        }
        BucketKey::Direct | BucketKey::Referrer(_) => RevenueSource::Referrer(RevenueSourceEntry {
            id: match key {
                BucketKey::Referrer(id) => Some(id),
                BucketKey::Direct | BucketKey::Campaign(_) => None,
            },
            referrer: compact_opt(bucket.referrer),
            referrer_type: bucket.referrer_type,
            group,
            metrics,
            landing_paths,
        }),
    }
}

fn compact_str_or_default(value: Option<SqlCompactStr>) -> CompactString {
    value.map(Into::into).unwrap_or_default()
}

fn compact_opt(value: Option<SqlCompactStr>) -> Option<CompactString> {
    value.map(Into::into)
}

fn landing_paths_from_counts(
    counts: FxHashMap<i64, i64>,
    path_by_id: &FxHashMap<i64, SqlCompactStr>,
) -> Vec<LandingPathCount> {
    if counts.is_empty() {
        return Vec::new();
    }
    let mut ranked: Vec<(i64, i64)> = counts.into_iter().collect();
    ranked.sort_by(|(left_id, left_count), (right_id, right_count)| {
        right_count.cmp(left_count).then(left_id.cmp(right_id))
    });
    ranked
        .into_iter()
        .take(TOP_LANDING_PATHS)
        .filter_map(|(path_id, count)| {
            path_by_id.get(&path_id).map(|path| LandingPathCount {
                id: path_id,
                path: path.clone().into(),
                count,
            })
        })
        .collect()
}

fn campaign_referrers_from_counts(
    counts: FxHashMap<Option<i64>, ReferrerVisitCount>,
) -> Vec<CampaignReferrerCount> {
    if counts.is_empty() {
        return Vec::new();
    }
    let mut ranked: Vec<(Option<i64>, ReferrerVisitCount)> = counts.into_iter().collect();
    ranked.sort_by(|(left_id, left), (right_id, right)| {
        right.count.cmp(&left.count).then(left_id.cmp(right_id))
    });
    ranked
        .into_iter()
        .take(TOP_CAMPAIGN_REFERRERS)
        .map(|(id, visits)| CampaignReferrerCount {
            id,
            referrer: compact_opt(visits.referrer),
            count: visits.count,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn as_referrer(source: &RevenueSource) -> &RevenueSourceEntry {
        match source {
            RevenueSource::Referrer(entry) => entry,
            other => panic!("expected a referrer source, got {other:?}"),
        }
    }

    fn as_campaign(source: &RevenueSource) -> &RevenueSourceCampaign {
        match source {
            RevenueSource::Campaign(campaign) => campaign,
            other => panic!("expected a campaign source, got {other:?}"),
        }
    }

    fn insert_path_with_value(conn: &ProjectConnection, path: &str) -> i64 {
        conn.execute(
            "INSERT INTO paths (path, hostname) VALUES (?1, 'acme.test')",
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

    fn insert_path(conn: &ProjectConnection) -> i64 {
        insert_path_with_value(conn, "/")
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

    fn insert_campaign(
        conn: &ProjectConnection,
        source: &str,
        medium: &str,
        campaign: &str,
    ) -> i64 {
        conn.execute(
            "INSERT INTO campaigns (source, medium, campaign, first_seen_at)
             VALUES (?1, ?2, ?3, 0)",
            params![source, medium, campaign],
        )
        .expect("insert campaign");
        conn.query_row(
            "SELECT id FROM campaigns
             WHERE source = ?1 AND medium = ?2 AND campaign = ?3",
            params![source, medium, campaign],
            |row| row.get::<_, i64>(0),
        )
        .expect("campaign id")
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_first_visit(
        conn: &ProjectConnection,
        seed: u8,
        viewed_at: i64,
        path_id: i64,
        campaign_id: Option<i64>,
        referrer_id: Option<i64>,
        referrer_type: Option<&str>,
        is_google: i64,
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
             ) VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, NULL, 1.0, NULL, NULL, NULL, 'desktop')",
            params![
                timestamp_id,
                referrer_id,
                campaign_id,
                referrer_type,
                is_google,
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
            .unwrap_or_else(|| timestamp_id(first_page_seed, 0));
        let mut payment = PaymentFixture::new(provider_payment_id, amount, paid_at);
        payment.visitor_timestamp_id = Some(visitor_timestamp_id);
        insert_test_payment(conn, payment);
    }

    fn baseline_query(start: i64, end: i64) -> RevenueSourceQuery {
        RevenueSourceQuery {
            start,
            end,
            segments: SegmentFilters::default(),
        }
    }

    #[test]
    fn rejects_inverted_range() {
        assert!(
            parse_revenue_source_query("/v2/read/projects/1/revenue-sources?start=200&end=100",)
                .is_err()
        );
    }

    #[test]
    fn empty_database_returns_zero_totals() {
        let conn = open_migrated_db();
        let response =
            build_revenue_source_response(&conn, &baseline_query(0, 10_000)).expect("response");
        let current = &response.current;
        assert_eq!(current.total_visitors, 0);
        assert_eq!(current.total_revenue, 0);
        assert_eq!(current.direct_visitors, 0);
        assert_eq!(current.direct_revenue, 0);
        assert_eq!(current.referrer_visitors, 0);
        assert_eq!(current.referrer_revenue, 0);
        assert_eq!(current.campaign_visitors, 0);
        assert_eq!(current.campaign_revenue, 0);
        assert_eq!(current.search_visitors, 0);
        assert_eq!(current.search_revenue, 0);
        assert_eq!(current.ai_visitors, 0);
        assert_eq!(current.ai_revenue, 0);
        assert_eq!(current.social_visitors, 0);
        assert_eq!(current.social_revenue, 0);
        assert_eq!(current.campaigns_visitors, 0);
        assert_eq!(current.campaigns_revenue, 0);
        assert_eq!(current.referrers_group_visitors, 0);
        assert_eq!(current.referrers_group_revenue, 0);
        assert!(current.entries.is_empty());
        // With no data the direct group summary has zero counts and no change.
        assert_eq!(response.groups.direct.metrics.visitors, 0);
        assert_eq!(response.groups.direct.metrics.revenue, 0);
        assert_eq!(
            response.groups.direct.metrics.revenue_change.ratio,
            Some(0.0)
        );
    }

    #[test]
    fn source_segment_filter_does_not_narrow_revenue_sources() {
        let conn = open_migrated_db();
        let path_id = insert_path(&conn);
        let news = insert_referrer(&conn, "news.example.com/");
        // One direct visit and one referrer visit.
        insert_first_visit(&conn, 1, 100, path_id, None, None, None, 0);
        insert_first_visit(&conn, 2, 110, path_id, None, Some(news), None, 0);

        // Filtering by the direct source must not hide the referrer source from
        // the revenue-sources view itself (users add sources from this view).
        let mut query = baseline_query(0, 10_000);
        query.segments = SegmentFilters {
            countries: Vec::new(),
            sources: vec![crate::segment_filter::SourceFilter::Direct],
            pages: Vec::new(),
            landing_pages: Vec::new(),
            events: Vec::new(),
            ..Default::default()
        };
        let response = build_revenue_source_response(&conn, &query).expect("response");
        assert_eq!(response.current.total_visitors, 2);
        assert_eq!(response.current.direct_visitors, 1);
        assert_eq!(response.current.referrer_visitors, 1);
        assert_eq!(response.current.entries.len(), 2);
    }

    #[test]
    fn channel_segment_filter_does_not_narrow_revenue_sources() {
        let conn = open_migrated_db();
        let path_id = insert_path(&conn);
        let news = insert_referrer(&conn, "news.example.com/");
        insert_first_visit(&conn, 1, 100, path_id, None, None, None, 0);
        insert_first_visit(&conn, 2, 110, path_id, None, Some(news), None, 0);

        let mut query = baseline_query(0, 10_000);
        query.segments = crate::segment_filter::parse_segment_filters("?channel=direct");
        let response = build_revenue_source_response(&conn, &query).expect("response");
        assert_eq!(response.current.total_visitors, 2);
        assert_eq!(response.current.referrer_visitors, 1);
    }

    #[test]
    fn ranks_referrer_direct_and_campaign_buckets_together() {
        let conn = open_migrated_db();
        let path_id = insert_path(&conn);
        let google = insert_referrer(&conn, "google.com/search");
        let news = insert_referrer(&conn, "news.example.com/");
        let camp_a = insert_campaign(&conn, "newsletter", "email", "digest");
        let camp_b = insert_campaign(&conn, "", "", "launch");

        // google.com/search: 3 visits (2 with camp_a, 1 with camp_b)
        insert_first_visit(
            &conn,
            1,
            100,
            path_id,
            Some(camp_a),
            Some(google),
            Some("search"),
            1,
        );
        insert_first_visit(
            &conn,
            2,
            110,
            path_id,
            Some(camp_a),
            Some(google),
            Some("search"),
            1,
        );
        insert_first_visit(
            &conn,
            3,
            120,
            path_id,
            Some(camp_b),
            Some(google),
            Some("search"),
            1,
        );
        // news.example.com: 2 visits (no campaign)
        insert_first_visit(&conn, 4, 130, path_id, None, Some(news), None, 0);
        insert_first_visit(&conn, 5, 140, path_id, None, Some(news), None, 0);
        // direct: 4 visits (2 with camp_b, 2 with no campaign)
        insert_first_visit(&conn, 6, 150, path_id, Some(camp_b), None, None, 0);
        insert_first_visit(&conn, 7, 160, path_id, Some(camp_b), None, None, 0);
        insert_first_visit(&conn, 8, 170, path_id, None, None, None, 0);
        insert_first_visit(&conn, 9, 180, path_id, None, None, None, 0);

        // Attribute revenue via payments.first_page_id:
        //  google camp_a: visit seed 1 -> $25, visit seed 2 has no payment
        //  google camp_b: visit seed 3 -> $10 + $5 (two payments on same landing)
        //  news        : visit seed 4 -> $40
        //  direct camp_b: visit seed 6 -> $15
        //  direct bare  : visit seed 9 -> $7
        insert_payment(&conn, 1, 1, 25_000, 200);
        insert_payment(&conn, 2, 3, 10_000, 210);
        insert_payment(&conn, 3, 3, 5_000, 220);
        insert_payment(&conn, 4, 4, 40_000, 230);
        insert_payment(&conn, 5, 6, 15_000, 240);
        insert_payment(&conn, 6, 9, 7_000, 250);
        // Payments without a matching landing must be ignored.
        insert_payment(&conn, 7, 99, 50_000, 260);

        let response =
            build_revenue_source_response(&conn, &baseline_query(0, 10_000)).expect("response");
        let current = &response.current;

        assert_eq!(current.total_visitors, 9);
        assert_eq!(
            current.total_revenue,
            25_000 + 15_000 + 40_000 + 15_000 + 7_000
        );
        assert_eq!(current.direct_visitors, 4);
        assert_eq!(current.direct_revenue, 15_000 + 7_000);
        assert_eq!(current.referrer_visitors, 5);
        assert_eq!(current.referrer_revenue, 25_000 + 15_000 + 40_000);
        assert_eq!(current.campaign_visitors, 5);
        // visits with landings.campaign set: google camp_a (2), google camp_b (1),
        // direct camp_b (2). Revenue across those: 25 + 15 + 15 = 55.
        assert_eq!(current.campaign_revenue, 25_000 + 15_000 + 15_000);
        assert_eq!(current.search_visitors, 0);
        assert_eq!(current.search_revenue, 0);
        assert_eq!(current.ai_visitors, 0);
        assert_eq!(current.ai_revenue, 0);
        assert_eq!(current.social_visitors, 0);
        assert_eq!(current.social_revenue, 0);
        assert_eq!(current.campaigns_visitors, 5);
        assert_eq!(current.campaigns_revenue, 25_000 + 15_000 + 15_000);
        assert_eq!(current.referrers_group_visitors, 2);
        assert_eq!(current.referrers_group_revenue, 40_000);

        // Every landing falls into exactly one top-level bucket, all ranked by
        // revenue together. Campaigns are no longer nested under a referrer, so
        // google (all 3 of its visits carried a campaign) has no referrer row:
        //   news (referrer)   $40k, 2 visitors
        //   camp_b (campaign) $30k, 3 visitors (google seed3 + direct seed6/7)
        //   camp_a (campaign) $25k, 2 visitors (google seed1/2)
        //   direct (referrer) $7k,  2 visitors (seed8/9, no referrer/campaign)
        assert_eq!(current.entries.len(), 4);

        let news_entry = as_referrer(&current.entries[0]);
        assert_eq!(news_entry.referrer.as_deref(), Some("news.example.com/"));
        assert_eq!(news_entry.group, SourceGroup::Referrers);
        assert_eq!(news_entry.metrics.visitors, 2);
        assert_eq!(news_entry.metrics.revenue, 40_000);

        let camp_b_entry = as_campaign(&current.entries[1]);
        assert_eq!(camp_b_entry.id, camp_b);
        assert_eq!(camp_b_entry.campaign, "launch");
        assert_eq!(camp_b_entry.group, SourceGroup::Campaigns);
        assert_eq!(camp_b_entry.metrics.visitors, 3);
        assert_eq!(camp_b_entry.metrics.revenue, 15_000 + 15_000);
        assert_eq!(camp_b_entry.metrics.converting_visitors, 2);

        let camp_a_entry = as_campaign(&current.entries[2]);
        assert_eq!(camp_a_entry.id, camp_a);
        assert_eq!(camp_a_entry.source, "newsletter");
        assert_eq!(camp_a_entry.group, SourceGroup::Campaigns);
        assert_eq!(camp_a_entry.metrics.visitors, 2);
        assert_eq!(camp_a_entry.metrics.revenue, 25_000);

        let direct_entry = as_referrer(&current.entries[3]);
        assert_eq!(direct_entry.referrer, None);
        assert_eq!(direct_entry.group, SourceGroup::Direct);
        assert_eq!(direct_entry.metrics.visitors, 2);
        assert_eq!(direct_entry.metrics.revenue, 7_000);

        // The whole fixture lives inside [0, 10_000), so the prior 30-day
        // window immediately before it has no data: group summaries and
        // individual entries both report no comparison.
        assert!(
            response
                .groups
                .referrers
                .metrics
                .revenue_change
                .ratio
                .is_none()
        );
        assert!(
            response
                .groups
                .campaigns
                .metrics
                .revenue_change
                .ratio
                .is_none()
        );

        for entry in &current.entries {
            let metrics = match entry {
                RevenueSource::Referrer(source) => &source.metrics,
                RevenueSource::Campaign(source) => &source.metrics,
            };
            assert_eq!(metrics.revenue_change, Change::none());
            assert_eq!(metrics.visitors_change, Change::none());
            assert_eq!(metrics.conversions_change, Change::none());
        }
    }

    #[test]
    fn groups_ai_referrers_in_ai_channel() {
        let conn = open_migrated_db();
        let path_id = insert_path(&conn);
        let chatgpt = insert_referrer(&conn, "chatgpt.com");
        let grok = insert_referrer(&conn, "grok.com");

        insert_first_visit(&conn, 1, 100, path_id, None, Some(chatgpt), Some("ai"), 0);
        insert_first_visit(&conn, 2, 110, path_id, None, Some(grok), Some("ai"), 0);
        insert_first_visit(&conn, 3, 120, path_id, None, Some(grok), Some("ai"), 0);

        insert_payment(&conn, 1, 1, 30_000, 200);
        insert_payment(&conn, 2, 2, 20_000, 210);

        let response =
            build_revenue_source_response(&conn, &baseline_query(0, 10_000)).expect("response");
        let current = &response.current;

        assert_eq!(current.ai_visitors, 3);
        assert_eq!(current.ai_revenue, 50_000);
        assert_eq!(current.referrers_group_visitors, 0);
        assert_eq!(current.entries.len(), 2);

        let chatgpt_entry = as_referrer(&current.entries[0]);
        assert_eq!(chatgpt_entry.referrer.as_deref(), Some("chatgpt.com"));
        assert_eq!(chatgpt_entry.referrer_type, Some(ReferrerType::Ai));
        assert_eq!(chatgpt_entry.group, SourceGroup::Ai);
        assert_eq!(chatgpt_entry.metrics.revenue, 30_000);

        let grok_entry = as_referrer(&current.entries[1]);
        assert_eq!(grok_entry.referrer.as_deref(), Some("grok.com"));
        assert_eq!(grok_entry.group, SourceGroup::Ai);
        assert_eq!(grok_entry.metrics.visitors, 2);
        assert_eq!(grok_entry.metrics.revenue, 20_000);
    }

    #[test]
    fn entries_include_landing_paths_ranked_by_visits() {
        let conn = open_migrated_db();
        let root_path = insert_path(&conn);
        let pricing_path = insert_path_with_value(&conn, "/pricing");
        let docs_path = insert_path_with_value(&conn, "/docs");
        let search = insert_referrer(&conn, "google.com/search");
        let campaign = insert_campaign(&conn, "google", "cpc", "brand");

        insert_first_visit(
            &conn,
            1,
            100,
            pricing_path,
            None,
            Some(search),
            Some("search"),
            1,
        );
        insert_first_visit(
            &conn,
            2,
            110,
            pricing_path,
            None,
            Some(search),
            Some("search"),
            1,
        );
        insert_first_visit(
            &conn,
            3,
            120,
            root_path,
            None,
            Some(search),
            Some("search"),
            1,
        );
        insert_first_visit(&conn, 4, 130, docs_path, Some(campaign), None, None, 0);

        let response =
            build_revenue_source_response(&conn, &baseline_query(0, 10_000)).expect("response");

        assert_eq!(response.current.entries.len(), 2);
        let search_entry = as_referrer(&response.current.entries[0]);
        assert_eq!(search_entry.referrer.as_deref(), Some("google.com/search"));
        assert_eq!(
            search_entry
                .landing_paths
                .iter()
                .map(|entry| entry.path.as_str())
                .collect::<Vec<_>>(),
            vec!["/pricing", "/"]
        );
        assert_eq!(search_entry.landing_paths[0].count, 2);
        assert_eq!(search_entry.landing_paths[1].count, 1);

        let campaign_entry = as_campaign(&response.current.entries[1]);
        assert_eq!(campaign_entry.id, campaign);
        assert_eq!(campaign_entry.landing_paths.len(), 1);
        assert_eq!(campaign_entry.landing_paths[0].path, "/docs");
        assert_eq!(campaign_entry.landing_paths[0].count, 1);
    }

    #[test]
    fn landing_paths_limited_to_top_five_by_visits() {
        let conn = open_migrated_db();
        let search = insert_referrer(&conn, "google.com/search");
        let paths: Vec<i64> = (0..6)
            .map(|index| insert_path_with_value(&conn, &format!("/p{index}")))
            .collect();

        for (index, path_id) in paths.iter().enumerate() {
            let visits = (index + 1) as u8;
            for offset in 0..visits {
                let seed = u8::try_from(index * 10 + usize::from(offset)).expect("seed");
                insert_first_visit(
                    &conn,
                    seed,
                    100 + i64::from(offset) + i64::try_from(index * 10).expect("viewed_at"),
                    *path_id,
                    None,
                    Some(search),
                    Some("search"),
                    1,
                );
            }
        }

        let response =
            build_revenue_source_response(&conn, &baseline_query(0, 10_000)).expect("response");
        let search_entry = as_referrer(&response.current.entries[0]);
        assert_eq!(search_entry.landing_paths.len(), TOP_LANDING_PATHS);
        assert_eq!(
            search_entry
                .landing_paths
                .iter()
                .map(|entry| entry.path.as_str())
                .collect::<Vec<_>>(),
            vec!["/p5", "/p4", "/p3", "/p2", "/p1"]
        );
        assert_eq!(
            search_entry
                .landing_paths
                .iter()
                .map(|entry| entry.count)
                .collect::<Vec<_>>(),
            vec![6, 5, 4, 3, 2]
        );
    }

    #[test]
    fn campaign_referrers_ranked_and_limited_to_top_five() {
        let conn = open_migrated_db();
        let path_id = insert_path(&conn);
        let campaign = insert_campaign(&conn, "google", "cpc", "brand");
        let referrers: Vec<i64> = (0..6)
            .map(|index| insert_referrer(&conn, &format!("ref{index}.test/")))
            .collect();

        let mut seed: u8 = 1;
        for offset in 0..7 {
            insert_first_visit(
                &conn,
                seed,
                100 + i64::from(offset),
                path_id,
                Some(campaign),
                None,
                None,
                0,
            );
            seed += 1;
        }
        for (index, referrer_id) in referrers.iter().enumerate() {
            let visits = u8::try_from(index + 1).expect("visits");
            for offset in 0..visits {
                insert_first_visit(
                    &conn,
                    seed,
                    200 + i64::from(offset) + i64::try_from(index * 10).expect("viewed_at"),
                    path_id,
                    Some(campaign),
                    Some(*referrer_id),
                    None,
                    0,
                );
                seed += 1;
            }
        }

        let response =
            build_revenue_source_response(&conn, &baseline_query(0, 10_000)).expect("response");
        let campaign_entry = as_campaign(&response.current.entries[0]);
        assert_eq!(campaign_entry.referrers.len(), TOP_CAMPAIGN_REFERRERS);
        assert_eq!(campaign_entry.referrers[0].id, None);
        assert_eq!(campaign_entry.referrers[0].referrer, None);
        assert_eq!(campaign_entry.referrers[0].count, 7);
        assert_eq!(
            campaign_entry
                .referrers
                .iter()
                .skip(1)
                .map(|entry| entry.referrer.as_deref())
                .collect::<Vec<_>>(),
            vec![
                Some("ref5.test/"),
                Some("ref4.test/"),
                Some("ref3.test/"),
                Some("ref2.test/"),
            ]
        );
        assert_eq!(
            campaign_entry
                .referrers
                .iter()
                .map(|entry| entry.count)
                .collect::<Vec<_>>(),
            vec![7, 6, 5, 4, 3]
        );
    }

    #[test]
    fn returns_all_referrer_buckets_ranked_by_revenue() {
        let conn = open_migrated_db();
        let path_id = insert_path(&conn);
        let mut seed: u8 = 0;
        for (index, weight) in [(0_u8, 5), (1, 4), (2, 3), (3, 2), (4, 1)] {
            let referrer = insert_referrer(&conn, &format!("ref{index}.test/"));
            for _ in 0..weight {
                seed += 1;
                insert_first_visit(
                    &conn,
                    seed,
                    100 + seed as i64,
                    path_id,
                    None,
                    Some(referrer),
                    None,
                    0,
                );
            }
        }

        let response =
            build_revenue_source_response(&conn, &baseline_query(0, 10_000)).expect("response");
        assert_eq!(response.current.entries.len(), 5);
        assert_eq!(
            as_referrer(&response.current.entries[0]).metrics.visitors,
            5
        );
        assert_eq!(
            as_referrer(&response.current.entries[1]).metrics.visitors,
            4
        );
        assert_eq!(
            as_referrer(&response.current.entries[2]).metrics.visitors,
            3
        );
        assert_eq!(
            as_referrer(&response.current.entries[3]).metrics.visitors,
            2
        );
        assert_eq!(
            as_referrer(&response.current.entries[4]).metrics.visitors,
            1
        );
        assert_eq!(response.current.referrer_visitors, 5 + 4 + 3 + 2 + 1);
    }

    #[test]
    fn direct_is_a_top_level_entry_ranked_by_revenue() {
        let conn = open_migrated_db();
        let path_id = insert_path(&conn);
        let referrer = insert_referrer(&conn, "ref.test/");
        insert_first_visit(&conn, 1, 100, path_id, None, Some(referrer), None, 0);
        insert_first_visit(&conn, 2, 110, path_id, None, None, None, 0);
        insert_first_visit(&conn, 3, 120, path_id, None, None, None, 0);
        insert_payment(&conn, 1, 2, 9_000, 200);

        let query = baseline_query(0, 10_000);
        let page = build_revenue_source_response(&conn, &query).expect("first page");
        // Direct ($9k) outranks the zero-revenue referrer and appears as a
        // normal entry whose `referrer` is absent.
        assert_eq!(page.current.entries.len(), 2);
        let direct_entry = as_referrer(&page.current.entries[0]);
        assert_eq!(direct_entry.referrer, None);
        assert_eq!(direct_entry.group, SourceGroup::Direct);
        assert_eq!(direct_entry.metrics.visitors, 2);
        assert_eq!(direct_entry.metrics.revenue, 9_000);

        let referrer_entry = as_referrer(&page.current.entries[1]);
        assert_eq!(referrer_entry.referrer.as_deref(), Some("ref.test/"));
        assert_eq!(referrer_entry.metrics.revenue, 0);

        // Totals still reflect direct counts independently of the entry list.
        assert_eq!(page.current.direct_visitors, 2);
        assert_eq!(page.current.direct_revenue, 9_000);
    }

    #[test]
    fn excludes_pageviews_outside_window() {
        let conn = open_migrated_db();
        let path_id = insert_path(&conn);
        let referrer = insert_referrer(&conn, "ref.test/");
        insert_first_visit(&conn, 1, 50, path_id, None, Some(referrer), None, 0);
        insert_first_visit(&conn, 2, 150, path_id, None, Some(referrer), None, 0);
        insert_first_visit(&conn, 3, 250, path_id, None, Some(referrer), None, 0);
        // Revenue follows the landing's window, not the paid_at window: a
        // payment dated outside [100,200) but tied to the in-window landing
        // (seed 2) is still attributed to this period.
        insert_payment(&conn, 1, 1, 1_000, 60);
        insert_payment(&conn, 2, 2, 7_500, 500);
        insert_payment(&conn, 3, 3, 9_999, 260);

        let query = RevenueSourceQuery {
            start: 100,
            end: 200,
            segments: SegmentFilters::default(),
        };
        // Pin `now` to the window end so the normalization factor is exactly
        // (200 - 100) / 30 days; this keeps the comparison deterministic.
        let response = build_revenue_source_response_at(&conn, &query, 200).expect("response");
        assert_eq!(response.current.total_visitors, 1);
        assert_eq!(response.current.total_revenue, 7_500);
        let entry = as_referrer(&response.current.entries[0]);
        assert_eq!(entry.metrics.visitors, 1);
        assert_eq!(entry.metrics.revenue, 7_500);

        // The prior 30-day window [100 - 30d, 100) contains the seed-1 visit at
        // viewed_at=50, whose $1k payment is attributed regardless of paid_at,
        // so the revenue change is a finite ratio.
        assert!(entry.metrics.revenue_change.ratio.is_some());
    }

    #[test]
    fn ignores_refunds_and_failed_payments() {
        let conn = open_migrated_db();
        let path_id = insert_path(&conn);
        let referrer = insert_referrer(&conn, "ref.test/");
        insert_first_visit(&conn, 1, 100, path_id, None, Some(referrer), None, 0);
        insert_payment(&conn, 1, 1, 20_000, 200);
        // Failed payment is excluded.
        let mut failed_payment = PaymentFixture::new("pay_failed", 5_000, 210);
        failed_payment.visitor_timestamp_id = Some(timestamp_id(1, 100));
        failed_payment.status = "failed".to_string();
        insert_test_payment(&conn, failed_payment);
        // Refund (negative amount) is excluded by payment_type filter.
        let mut refund = PaymentFixture::new("pay_refund", -3_000, 220);
        refund.visitor_timestamp_id = Some(timestamp_id(1, 100));
        refund.payment_type = "refund".to_string();
        insert_test_payment(&conn, refund);

        let response =
            build_revenue_source_response(&conn, &baseline_query(0, 10_000)).expect("response");
        assert_eq!(response.current.total_revenue, 20_000);
        assert_eq!(
            as_referrer(&response.current.entries[0]).metrics.revenue,
            20_000
        );
    }

    #[test]
    fn revenue_excludes_subscription_renewals() {
        let conn = open_migrated_db();
        let path_id = insert_path(&conn);
        let referrer = insert_referrer(&conn, "google.com");
        insert_first_visit(
            &conn,
            1,
            100,
            path_id,
            None,
            Some(referrer),
            Some("search"),
            0,
        );
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

        let response =
            build_revenue_source_response(&conn, &baseline_query(0, 10_000)).expect("response");
        let entry = as_referrer(&response.current.entries[0]);
        assert_eq!(entry.metrics.revenue, 50_000);
        assert_eq!(entry.metrics.converting_visitors, 1);
    }

    #[test]
    fn campaign_entries_carry_network_and_display_name() {
        let conn = open_migrated_db();
        let path_id = insert_path(&conn);
        conn.execute(
            "INSERT INTO campaigns (source, medium, campaign, network, display_name, first_seen_at)
             VALUES ('spring', 'cpc', 'launch', 'google_ads', 'Spring Launch', 0)",
            [],
        )
        .expect("insert campaign");
        let campaign_id = conn
            .query_row("SELECT id FROM campaigns LIMIT 1", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("campaign id");
        insert_first_visit(&conn, 1, 100, path_id, Some(campaign_id), None, None, 0);

        let response =
            build_revenue_source_response(&conn, &baseline_query(0, 10_000)).expect("response");
        let campaign = as_campaign(&response.current.entries[0]);
        assert_eq!(campaign.id, campaign_id);
        assert_eq!(campaign.network.as_deref(), Some("google_ads"));
        assert_eq!(campaign.display_name.as_deref(), Some("Spring Launch"));
        assert_eq!(campaign.group, SourceGroup::Campaigns);
        assert_eq!(campaign.metrics.visitors, 1);
    }

    #[test]
    fn deleted_campaign_visits_fall_back_to_referrer_and_direct() {
        let conn = open_migrated_db();
        let path_id = insert_path(&conn);
        let referrer = insert_referrer(&conn, "ref.test/");
        let campaign_id = insert_campaign(&conn, "spring", "cpc", "launch");
        conn.execute(
            "UPDATE campaigns SET deleted_at = 1 WHERE id = ?1",
            params![campaign_id],
        )
        .expect("delete campaign");
        insert_first_visit(
            &conn,
            1,
            100,
            path_id,
            Some(campaign_id),
            Some(referrer),
            None,
            0,
        );
        insert_first_visit(&conn, 2, 110, path_id, Some(campaign_id), None, None, 0);

        let response =
            build_revenue_source_response(&conn, &baseline_query(0, 10_000)).expect("response");
        assert!(
            response
                .current
                .entries
                .iter()
                .all(|entry| !matches!(entry, RevenueSource::Campaign(_)))
        );
        assert_eq!(response.current.campaign_visitors, 0);
        assert_eq!(response.current.direct_visitors, 1);
        assert_eq!(response.current.referrer_visitors, 1);
    }
}
