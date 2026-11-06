//! Search term rows and totals, and their changes against the prior period.

use serde::Serialize;

use crate::comparison::Change;

/// A landing page that received Search Console clicks for a query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct SearchTermLandingPath {
    pub(crate) path_id: i64,
    pub(crate) path: String,
    pub(crate) clicks: i64,
}

/// A single Google Search Console query aggregated over the requested window.
///
/// `clicks`/`impressions`/`position`/`mobile_clicks` come from
/// `search_queries_by_date` (summed across the daily rows in range; position is
/// impression-weighted). `detail_clicks` is `mobile_clicks + desktop_clicks`
/// over the same window (GSC device rows can omit some traffic, so this may
/// differ slightly from `clicks`). `estimated_revenue` projects the search
/// channel's revenue-per-visitor onto this query's clicks.
#[derive(Debug, Serialize)]
pub(crate) struct SearchTermRow {
    pub(crate) query: String,
    /// Paths receiving Search Console clicks for this query in the requested
    /// window, ordered by clicks descending then path ascending.
    pub(crate) landing_paths: Vec<SearchTermLandingPath>,
    /// Impression-weighted average Google position across the window.
    pub(crate) position: f64,
    pub(crate) impressions: i64,
    pub(crate) clicks: i64,
    /// Clicks attributed to mobile/tablet devices in the window.
    pub(crate) mobile_clicks: i64,
    /// `mobile_clicks + desktop_clicks` over the window (mobile-share denominator).
    pub(crate) detail_clicks: i64,
    /// Estimated revenue (thousandths) = this query's share of the search
    /// channel's revenue, weighted by its share of windowed clicks.
    pub(crate) estimated_revenue: i64,
    /// Click-through rate (0..1): `clicks / impressions`.
    pub(crate) ctr: f64,
    /// Mobile share (0..1): `mobile_clicks / detail_clicks`.
    pub(crate) mobile_share: f64,
    /// Site-own typical CTR for this query's rank (0 when position is outside
    /// 1..=10 or that rank has no impressions).
    pub(crate) expected_ctr: f64,
    /// Impressions × max(0, expected CTR − actual CTR).
    pub(crate) lost_clicks: f64,
    /// `lost_clicks` valued at the search channel's revenue per click (thousandths).
    pub(crate) lost_revenue: f64,
    /// True when this query is among the top [`HIGHLIGHT_TOP_N`](super::opportunities::HIGHLIGHT_TOP_N) by
    /// [`Self::lost_clicks`] across every term in the window.
    pub(crate) highest_lost_clicks: bool,
    /// True when this query is among the top [`HIGHLIGHT_TOP_N`](super::opportunities::HIGHLIGHT_TOP_N) by clicks
    /// that landed off the leading path, across every term in the window.
    pub(crate) highest_cannibalization: bool,
    /// Position change vs the prior 30 days (averages compared directly).
    pub(crate) position_change: Change,
    /// Impression change vs the prior 30 days (normalized to current period).
    pub(crate) impressions_change: Change,
    /// Click change vs the prior 30 days (normalized to current period).
    pub(crate) clicks_change: Change,
    /// CTR change vs the prior 30 days (rate, compared directly).
    pub(crate) ctr_change: Change,
    /// Mobile-share change vs the prior 30 days (rate, compared directly).
    pub(crate) mobile_share_change: Change,
    /// Estimated-revenue change vs the prior 30 days (normalized).
    pub(crate) estimated_revenue_change: Change,
}

#[derive(Debug, Serialize)]
pub(crate) struct SearchTermsSummary {
    /// Average Google position weighted by each query's share of clicks.
    pub(crate) position: f64,
    pub(crate) impressions: i64,
    pub(crate) clicks: i64,
    /// Average click-through rate (0..1) weighted by each query's share of clicks.
    pub(crate) ctr: f64,
    /// Average mobile share (0..1) weighted by each query's share of clicks.
    pub(crate) mobile_share: f64,
    pub(crate) estimated_revenue: i64,
    pub(crate) position_change: Change,
    pub(crate) impressions_change: Change,
    pub(crate) clicks_change: Change,
    pub(crate) ctr_change: Change,
    pub(crate) mobile_share_change: Change,
    pub(crate) estimated_revenue_change: Change,
}

/// Terms and totals for the requested window, each carrying server-computed
/// period-over-period deltas (versus the prior 30 days, normalized where
/// additive).
#[derive(Debug, Serialize)]
pub(crate) struct SearchTermsPeriod {
    /// Total search-channel revenue (thousandths) in the window. The per-term
    /// `estimated_revenue` values are this amount split across queries by click
    /// share, so they sum back to this total.
    pub(crate) search_channel_revenue: i64,
    pub(crate) summary: SearchTermsSummary,
    pub(crate) terms: Vec<SearchTermRow>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SearchTermsResponse {
    /// Terms and totals for the requested window, with deltas embedded.
    pub(crate) current: SearchTermsPeriod,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) next_offset: Option<i64>,
    pub(crate) offset: i64,
    pub(crate) limit: i64,
}

/// Click-through rate (0..1): `clicks / impressions`.
pub(super) fn click_through_rate(clicks: i64, impressions: i64) -> f64 {
    if impressions > 0 {
        clicks as f64 / impressions as f64
    } else {
        0.0
    }
}

/// Mobile share (0..1): `mobile_clicks / detail_clicks`.
pub(super) fn mobile_share(mobile_clicks: i64, detail_clicks: i64) -> f64 {
    if detail_clicks > 0 {
        mobile_clicks as f64 / detail_clicks as f64
    } else {
        0.0
    }
}

/// A single query aggregated over a window, before deltas are attached.
pub(super) struct RawTerm {
    pub(super) query_id: i64,
    pub(super) query: String,
    pub(super) landing_paths: Vec<SearchTermLandingPath>,
    pub(super) position: f64,
    pub(super) impressions: i64,
    pub(super) clicks: i64,
    pub(super) mobile_clicks: i64,
    pub(super) detail_clicks: i64,
    pub(super) estimated_revenue: i64,
    pub(super) expected_ctr: f64,
    pub(super) lost_clicks: f64,
    pub(super) lost_revenue: f64,
    pub(super) cannibalized_clicks: i64,
    pub(super) highest_lost_clicks: bool,
    pub(super) highest_cannibalization: bool,
}

/// Channel-level totals for a window, before deltas are attached.
pub(super) struct RawSummary {
    pub(super) position: f64,
    pub(super) impressions: i64,
    pub(super) clicks: i64,
    pub(super) ctr: f64,
    pub(super) mobile_share: f64,
    pub(super) estimated_revenue: i64,
}

/// Terms + totals for a window, before period-over-period deltas are attached.
pub(super) struct RawPeriod {
    pub(super) search_channel_revenue: i64,
    pub(super) summary: RawSummary,
    pub(super) terms: Vec<RawTerm>,
}

/// Builds a display-ready term row by attaching deltas versus the matching
/// prior-period query (if any).
pub(super) fn term_with_changes(
    term: &RawTerm,
    prior: Option<&RawTerm>,
    factor: f64,
) -> SearchTermRow {
    let ctr = click_through_rate(term.clicks, term.impressions);
    let mobile = mobile_share(term.mobile_clicks, term.detail_clicks);
    let position_change = Change::breakdown_rate(term.position, prior.map(|prior| prior.position));
    let impressions_change = Change::breakdown_absolute(
        term.impressions as f64,
        prior.map(|prior| prior.impressions as f64),
        factor,
    );
    let clicks_change = Change::breakdown_absolute(
        term.clicks as f64,
        prior.map(|prior| prior.clicks as f64),
        factor,
    );
    let ctr_change = Change::breakdown_rate(
        ctr,
        prior.map(|prior| click_through_rate(prior.clicks, prior.impressions)),
    );
    let mobile_share_change = Change::breakdown_rate(
        mobile,
        prior.map(|prior| mobile_share(prior.mobile_clicks, prior.detail_clicks)),
    );
    let estimated_revenue_change = Change::breakdown_absolute(
        term.estimated_revenue as f64,
        prior.map(|prior| prior.estimated_revenue as f64),
        factor,
    );
    SearchTermRow {
        query: term.query.clone(),
        landing_paths: term.landing_paths.clone(),
        position: term.position,
        impressions: term.impressions,
        clicks: term.clicks,
        mobile_clicks: term.mobile_clicks,
        detail_clicks: term.detail_clicks,
        estimated_revenue: term.estimated_revenue,
        ctr,
        mobile_share: mobile,
        expected_ctr: term.expected_ctr,
        lost_clicks: term.lost_clicks,
        lost_revenue: term.lost_revenue,
        highest_lost_clicks: term.highest_lost_clicks,
        highest_cannibalization: term.highest_cannibalization,
        position_change,
        impressions_change,
        clicks_change,
        ctr_change,
        mobile_share_change,
        estimated_revenue_change,
    }
}

/// Attaches channel-summary deltas versus the prior period.
pub(super) fn summary_with_changes(
    current: &RawSummary,
    previous: &RawSummary,
    factor: f64,
) -> SearchTermsSummary {
    SearchTermsSummary {
        position: current.position,
        impressions: current.impressions,
        clicks: current.clicks,
        ctr: current.ctr,
        mobile_share: current.mobile_share,
        estimated_revenue: current.estimated_revenue,
        position_change: Change::rate(current.position, previous.position),
        impressions_change: Change::absolute(
            current.impressions as f64,
            previous.impressions as f64,
            factor,
        ),
        clicks_change: Change::absolute(current.clicks as f64, previous.clicks as f64, factor),
        ctr_change: Change::rate(current.ctr, previous.ctr),
        mobile_share_change: Change::rate(current.mobile_share, previous.mobile_share),
        estimated_revenue_change: Change::absolute(
            current.estimated_revenue as f64,
            previous.estimated_revenue as f64,
            factor,
        ),
    }
}
