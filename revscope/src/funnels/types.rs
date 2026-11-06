use serde::{Deserialize, Serialize};

use crate::comparison;
use crate::segment_filter::{LandingAttributeParam, SegmentFilters};

#[derive(Debug, Serialize)]
pub(crate) struct Funnel {
    pub(crate) id: i64,
    pub(crate) name: String,
    pub(crate) description: Option<String>,
    pub(crate) steps: Vec<FunnelStep>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct FunnelStep {
    pub(crate) id: i64,
    pub(crate) order: i64,
    pub(crate) step_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) event_name_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) event_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) path_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) hostname: Option<String>,
    pub(crate) is_landing: bool,
    pub(crate) payment_types: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) description: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct FunnelsResponse {
    pub(crate) funnels: Vec<Funnel>,
}

#[derive(Debug, Serialize)]
pub(crate) struct FunnelOption {
    pub(crate) id: i64,
    pub(crate) name: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct FunnelEventOption {
    pub(crate) id: i64,
    pub(crate) name: String,
    pub(crate) path_ids: Vec<i64>,
}

#[derive(Debug, Serialize)]
pub(crate) struct FunnelOptionsResponse {
    pub(crate) paths: Vec<FunnelOption>,
    pub(crate) events: Vec<FunnelEventOption>,
}

/// A single drop-off destination for a funnel step: where sessions that left the
/// funnel after this step went next, or the synthetic exit bucket.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct FunnelDropoffRow {
    /// Destination `paths.id`; absent for the synthetic exit bucket.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) path_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) hostname: Option<String>,
    /// Outbound destination (hostname or `mailto:` domain) for drop-offs that
    /// left via a tracked outbound link. Set only when `path_id` is absent (a
    /// real exit re-attributed from the synthetic exit bucket).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) outbound_to: Option<String>,
    /// Drop-off sessions whose next page view went to this destination.
    pub(crate) visitors: i64,
    /// `visitors` divided by all counted drop-off sessions for the step (0..1).
    pub(crate) share: f64,
}

/// Where sessions that reached a step but did not reach the next step went next.
/// Navigations to the next step's page are excluded (expected funnel flow).
#[derive(Clone, Debug, Serialize)]
pub(crate) struct FunnelStepDropoff {
    pub(crate) total_visitors: i64,
    pub(crate) transitions: Vec<FunnelDropoffRow>,
}

#[derive(Debug, Serialize)]
pub(crate) struct FunnelStepData {
    pub(crate) step_id: i64,
    pub(crate) order: i64,
    pub(crate) step_type: String,
    /// Generated label from the step configuration (path, event, or payment type).
    pub(crate) name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) event_name: Option<String>,
    /// Page-anchored path for this step. Set for pageview steps (with or without
    /// an event); payment steps have none. Used by the dashboard to decide when
    /// drop-off destinations apply.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) path_id: Option<i64>,
    pub(crate) visitors: i64,
    pub(crate) previous_visitors: i64,
    /// Period-over-period change for [`Self::visitors`], with the previous
    /// absolute count normalized to the current period length.
    pub(crate) visitors_change: comparison::Change,
    /// Share of this step's visitors that did not reach the next step (0..1).
    /// `None` for the final step, which has no following step.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) dropoff_rate: Option<f64>,
    /// Destinations of sessions that left after this step. Set for non-final
    /// pageview steps; omitted on the final step and on payment steps.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) dropoff: Option<FunnelStepDropoff>,
    /// Total funnel revenue divided by this step's visitors, in integer
    /// thousandths of the currency unit. Only set when the final step is a
    /// payment step (so the funnel has a revenue total to attribute).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) revenue_per_visitor: Option<i64>,
}

#[derive(Debug, Serialize)]
pub(crate) struct FunnelDataResponse {
    pub(crate) funnel_id: i64,
    pub(crate) steps: Vec<FunnelStepData>,
    /// Total revenue across visitors who completed the whole funnel, in integer
    /// thousandths of the currency unit. Only set when the final step is a
    /// payment step.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) total_revenue: Option<i64>,
}

/// Bootstrap payload when `funnel_id` is omitted from `/v2/read/projects/{id}/funnel-data`:
/// the funnel list plus computed data for the first funnel (if any).
#[derive(Debug, Serialize)]
pub(crate) struct FunnelDataBootstrapResponse {
    pub(crate) funnels: Vec<Funnel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) data: Option<FunnelDataResponse>,
}

/// `/v2/read/projects/{id}/funnel-data` response: either data for an explicit funnel, or the
/// bootstrap shape that includes the funnel list.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(crate) enum FunnelDataRouteResponse {
    Data(FunnelDataResponse),
    Bootstrap(FunnelDataBootstrapResponse),
}

#[derive(Debug, Serialize)]
pub(crate) struct SaveFunnelResponse {
    pub(crate) funnel: Funnel,
    pub(crate) data: FunnelDataResponse,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SaveFunnelRequest {
    pub(crate) id: Option<i64>,
    pub(crate) name: String,
    pub(crate) description: Option<String>,
    pub(crate) steps: Vec<SaveFunnelStepRequest>,
    #[serde(default)]
    pub(crate) start: Option<i64>,
    #[serde(default)]
    pub(crate) end: Option<i64>,
    #[serde(default)]
    pub(crate) previous_start: Option<i64>,
    #[serde(default)]
    pub(crate) previous_end: Option<i64>,
}

/// Stateless preview request: the editor sends the funnel steps it currently
/// has (without persisting them) and gets back per-step visitor counts for the
/// selected date range.
#[derive(Debug, Deserialize)]
pub(crate) struct FunnelPreviewRequest {
    pub(crate) steps: Vec<SaveFunnelStepRequest>,
    #[serde(default)]
    pub(crate) start: Option<i64>,
    #[serde(default)]
    pub(crate) end: Option<i64>,
    #[serde(default)]
    pub(crate) previous_start: Option<i64>,
    #[serde(default)]
    pub(crate) previous_end: Option<i64>,
    /// Active global country segment filter (ISO alpha-2). Optional.
    #[serde(default)]
    pub(crate) countries: Vec<String>,
    /// Active global revenue-source segment filter (`campaign-<id>`,
    /// `referrer-<id>`, `direct`). Optional.
    #[serde(default)]
    pub(crate) sources: Vec<String>,
    /// Active global page segment filter (`paths.id`). Optional.
    #[serde(default)]
    pub(crate) pages: Vec<i64>,
    /// Active global landing-page segment filter (`paths.id`). Optional.
    #[serde(default)]
    pub(crate) landing_pages: Vec<i64>,
    /// Active global event segment filter (`event_names.id`). Optional.
    #[serde(default)]
    pub(crate) events: Vec<i64>,
    /// Active global landing attribute filters (channel, region, city, browser,
    /// OS, device). Optional.
    #[serde(default)]
    pub(crate) attributes: Vec<LandingAttributeParam>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SaveFunnelStepRequest {
    pub(crate) step_type: String,
    pub(crate) event_name_id: Option<i64>,
    pub(crate) path_id: Option<i64>,
    #[serde(default)]
    pub(crate) is_landing: bool,
    #[serde(default)]
    pub(crate) payment_types: Vec<String>,
    pub(crate) description: Option<String>,
}

pub(crate) struct FunnelDataQuery {
    /// When omitted, the route returns the funnel list and data for the first
    /// funnel (lowest id). When set, only that funnel's data is returned.
    pub(crate) funnel_id: Option<i64>,
    pub(crate) start: i64,
    pub(crate) end: i64,
    pub(crate) previous_start: i64,
    pub(crate) previous_end: i64,
    pub(crate) segments: SegmentFilters,
}

pub(crate) struct DeleteFunnelQuery {
    pub(crate) funnel_id: i64,
}

/// Payment types as `funnel_steps.payment_types` stores them: comma-separated,
/// with `None` meaning any type.
pub(super) fn payment_types_to_text(payment_types: &[String]) -> Option<String> {
    (!payment_types.is_empty()).then(|| payment_types.join(","))
}
