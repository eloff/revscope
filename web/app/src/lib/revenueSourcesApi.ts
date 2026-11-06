import { getApiBaseUrl } from "@revscope/web-shared/apiBaseUrl";
import { projectApiPath } from "@revscope/web-shared/apiPaths";
import { authRequestInit, fetchJson } from "@revscope/web-shared/apiRequest";
import type { AuthSession } from "@revscope/web-shared/authState";
import { appendSegmentParams, EMPTY_SEGMENT_FILTERS, type SegmentFilters } from "./segmentFilters";

/** Marketing-channel bucket for revenue-sources totals and UI grouping. */
export type SourceGroup = "search" | "ai" | "social" | "campaigns" | "referrers" | "direct";

/** Values stored on `landings.referrer_type` (mirrors revscope `referrer.rs`). */
export type ReferrerType = "ai" | "search" | "social" | "email" | "campaign" | "affiliate";

/** Landing page path with visit count for a revenue source bucket. */
export type LandingPathCount = {
  /** Stable `paths.id` for linking to pages/landing-pages rows. */
  id: number;
  path: string;
  count: number;
};

/** One referrer contributing visits to a campaign, ranked by visit count. */
export type CampaignReferrerCount = {
  /** Stable `referrers.id`. Absent for direct (no-referrer) visits. */
  id?: number;
  /** Normalized referrer URL. Absent for direct visits. */
  referrer?: string;
  count: number;
};

import type { MetricChange } from "./metricChange";

export type { MetricChange } from "./metricChange";

/** 95% Wilson score confidence interval for a conversion rate (0..1 bounds). */
export type ConversionInterval = {
  lower: number;
  upper: number;
};

/**
 * Display-ready metrics shared by every revenue-source row and group summary,
 * computed server-side. Counts and `revenue` are raw current-period values;
 * rates and the interval are derived from the current period; the `*_change`
 * fields compare against the prior 30 days (additive quantities normalized to
 * the current period, rates compared directly).
 */
export type SourceMetrics = {
  visitors: number;
  /** First visits in this bucket with attributed payment revenue. */
  converting_visitors: number;
  /** Sum of qualifying payment amounts (thousandths); excludes subscription renewals. */
  revenue: number;
  /** `converting_visitors` divided by `visitors`. */
  conversion_rate: number;
  /** Revenue divided by visitors (thousandths). */
  revenue_per_visitor: number;
  /** Lower bound of the 95% Wilson interval, used for ranking. */
  conversion_lower_bound: number;
  /** 95% Wilson confidence interval for the conversion rate (absent when no visitors). */
  conversion_ci?: ConversionInterval;
  revenue_change: MetricChange;
  visitors_change: MetricChange;
  conversions_change: MetricChange;
  revenue_per_visitor_change: MetricChange;
  conversion_rate_change: MetricChange;
};

/**
 * A campaign revenue source: first visits grouped by the `campaigns` row
 * (decoded UTM/attribution tuple), regardless of referrer. Top-level, no
 * longer nested under a referrer.
 */
export type RevenueSourceCampaign = {
  kind: "campaign";
  /** Stable per-project id from the `campaigns` table. */
  id: number;
  source: string;
  medium: string;
  campaign: string;
  /** Ad network the campaign was attributed to (absent when unknown). */
  network?: string;
  /** Optional human-readable label set by the user in the dashboard. */
  display_name?: string;
  /** Channel bucket for this row (always `campaigns`). */
  group: SourceGroup;
  /** Top landing page paths for visits in this source, ordered by visit count. */
  landing_paths?: LandingPathCount[];
  /** Top referrers for visits in this campaign, ordered by visit count (at most five). */
  referrers?: CampaignReferrerCount[];
} & SourceMetrics;

/**
 * A referrer revenue source: first visits with a given referrer (and no
 * campaign). When `referrer` is absent it is the direct bucket (no referrer,
 * no campaign).
 */
export type RevenueSourceEntry = {
  kind: "referrer";
  /** Stable `referrers.id`; absent for direct rows. */
  id?: number;
  /** Normalized referrer URL; absent for direct (no-referrer) visits. */
  referrer?: string;
  /** Classification from `landings.referrer_type` at ingest time. */
  referrer_type?: ReferrerType;
  /** Channel bucket for this row. */
  group: SourceGroup;
  /** Top landing page paths for visits in this source, ordered by visit count. */
  landing_paths?: LandingPathCount[];
} & SourceMetrics;

/**
 * A single top-level revenue source, discriminated by `kind`. Campaigns,
 * referrers, and the direct bucket all compete in one revenue-ranked list.
 */
export type RevenueSource = RevenueSourceEntry | RevenueSourceCampaign;

/** Per-group summary metrics and deltas for one channel group. */
export type GroupSummary = {
  group: SourceGroup;
} & SourceMetrics;

/** Per-group summaries keyed by channel group. */
export type GroupSummaries = Record<SourceGroup, GroupSummary>;

/**
 * Per-period payload: ranked entries (each carrying server-computed derived
 * metrics and deltas) plus the channel totals the dashboard uses to synthesize
 * per-group "Other" rows.
 */
export type RevenueSourcePeriod = {
  /** Revenue sources across all landings, ranked by revenue. */
  entries: RevenueSource[];
  total_visitors: number;
  total_revenue: number;
  direct_visitors: number;
  direct_revenue: number;
  referrer_visitors: number;
  referrer_revenue: number;
  campaign_visitors: number;
  campaign_revenue: number;
  search_visitors: number;
  search_revenue: number;
  ai_visitors: number;
  ai_revenue: number;
  social_visitors: number;
  social_revenue: number;
  campaigns_visitors: number;
  campaigns_revenue: number;
  referrers_group_visitors: number;
  referrers_group_revenue: number;
};

export type RevenueSourceResponse = {
  /** Ranked entries and channel totals for the requested window. */
  current: RevenueSourcePeriod;
  /** Per-group summary metrics and deltas (versus the prior 30 days). */
  groups: GroupSummaries;
};

export async function fetchRevenueSources(
  session: AuthSession | undefined,
  projectId: string,
  start: number,
  end: number,
  segments: SegmentFilters = EMPTY_SEGMENT_FILTERS,
): Promise<RevenueSourceResponse> {
  const url = new URL(projectApiPath("read", projectId, "revenue-sources"), getApiBaseUrl());
  url.searchParams.set("start", String(start));
  url.searchParams.set("end", String(end));
  appendSegmentParams(url, segments);

  return fetchJson<RevenueSourceResponse>(
    url,
    authRequestInit(session),
    "Unable to load sources data.",
  );
}
