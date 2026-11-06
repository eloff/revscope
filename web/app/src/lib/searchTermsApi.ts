import { getApiBaseUrl } from "@revscope/web-shared/apiBaseUrl";
import { projectApiPath } from "@revscope/web-shared/apiPaths";
import { authRequestInit, fetchJson } from "@revscope/web-shared/apiRequest";
import type { AuthSession } from "@revscope/web-shared/authState";
import type { MetricChange } from "./metricChange";
import { appendSegmentParams, EMPTY_SEGMENT_FILTERS, type SegmentFilters } from "./segmentFilters";

export type { MetricChange } from "./metricChange";

export const SEARCH_TERMS_PAGE_SIZE = 100;

export type SearchTermsOrderBy =
  | "query"
  | "landing_paths"
  | "position"
  | "impressions"
  | "clicks"
  | "ctr"
  | "mobile"
  | "estimated_revenue";

export type SearchTermsSortDirection = "asc" | "desc";

export type SearchTermsSortState = {
  orderBy: SearchTermsOrderBy;
  direction: SearchTermsSortDirection;
};

export const SEARCH_TERMS_ORDER_BY_COLUMNS = [
  "query",
  "landing_paths",
  "position",
  "impressions",
  "clicks",
  "ctr",
  "mobile",
  "estimated_revenue",
] as const satisfies readonly SearchTermsOrderBy[];

/** Backend ranks by estimated revenue (≡ clicks); mirror that as the initial sort. */
export const DEFAULT_SEARCH_TERMS_SORT: SearchTermsSortState = {
  orderBy: "estimated_revenue",
  direction: "desc",
};

/** A landing page that received Search Console clicks for a query. */
export type SearchTermLandingPath = {
  path_id: number;
  path: string;
  clicks: number;
};

/**
 * A single Google Search Console query aggregated over the requested window.
 * `clicks`/`impressions`/`position`/`mobile_clicks` are windowed GSC daily
 * totals; `detail_clicks` is `mobile_clicks + desktop_clicks` (mobile-share
 * denominator). `estimated_revenue` projects the search channel's
 * revenue-per-visitor onto this query's clicks. The `*_change` fields are
 * server-computed deltas versus the prior 30 days (additive quantities
 * normalized to the current period; rates and position compared directly).
 */
export type SearchTermRow = {
  query: string;
  /** Paths receiving Search Console clicks for this query in the window, ordered by clicks. */
  landing_paths: SearchTermLandingPath[];
  /** Impression-weighted average Google position across the window. */
  position: number;
  impressions: number;
  clicks: number;
  /** Clicks attributed to mobile/tablet devices in the window. */
  mobile_clicks: number;
  /** `mobile_clicks + desktop_clicks` over the window (mobile-share denominator). */
  detail_clicks: number;
  /** Estimated revenue (thousandths) for this query over the window. */
  estimated_revenue: number;
  /** Click-through rate (0..1): `clicks / impressions`. */
  ctr: number;
  /** Mobile share (0..1): `mobile_clicks / detail_clicks`. */
  mobile_share: number;
  /** Site-own typical CTR for this query's rank (0 when outside ranks 1–10). */
  expected_ctr: number;
  /** Impressions × max(0, expected CTR − actual CTR). */
  lost_clicks: number;
  /** `lost_clicks` valued at channel revenue per click (thousandths). */
  lost_revenue: number;
  /** Among the three queries with the most lost clicks in the window. */
  highest_lost_clicks: boolean;
  /** Among the three queries with the most clicks off the leading landing path. */
  highest_cannibalization: boolean;
  position_change: MetricChange;
  impressions_change: MetricChange;
  clicks_change: MetricChange;
  ctr_change: MetricChange;
  mobile_share_change: MetricChange;
  estimated_revenue_change: MetricChange;
};

export type SearchTermsSummary = {
  /** Average Google position weighted by each query's share of clicks. */
  position: number;
  impressions: number;
  clicks: number;
  /** Average click-through rate (0..1) weighted by each query's share of clicks. */
  ctr: number;
  /** Average mobile share (0..1) weighted by each query's share of clicks. */
  mobile_share: number;
  /** Total estimated revenue (thousandths) over the window. */
  estimated_revenue: number;
  position_change: MetricChange;
  impressions_change: MetricChange;
  clicks_change: MetricChange;
  ctr_change: MetricChange;
  mobile_share_change: MetricChange;
  estimated_revenue_change: MetricChange;
};

export type SearchTermsPeriod = {
  /** Total estimated Google search-term revenue (thousandths) over the window. */
  search_channel_revenue: number;
  summary: SearchTermsSummary;
  terms: SearchTermRow[];
};

export type SearchTermsResponse = {
  /** Terms and totals for the requested window, with deltas embedded. */
  current: SearchTermsPeriod;
  next_offset?: number;
  offset: number;
  limit: number;
};

export async function fetchSearchTerms(
  session: AuthSession | undefined,
  projectId: string,
  start: number,
  end: number,
  offset: number,
  sort: SearchTermsSortState,
  segments: SegmentFilters = EMPTY_SEGMENT_FILTERS,
): Promise<SearchTermsResponse> {
  const url = new URL(projectApiPath("read", projectId, "search-terms"), getApiBaseUrl());
  url.searchParams.set("start", String(start));
  url.searchParams.set("end", String(end));
  url.searchParams.set("offset", String(offset));
  url.searchParams.set("order_by", sort.orderBy);
  url.searchParams.set("direction", sort.direction);
  appendSegmentParams(url, segments);

  return fetchJson<SearchTermsResponse>(
    url,
    authRequestInit(session),
    "Unable to load search terms data.",
  );
}
