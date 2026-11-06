import { getApiBaseUrl } from "@revscope/web-shared/apiBaseUrl";
import { projectApiPath } from "@revscope/web-shared/apiPaths";
import { authRequestInit, fetchJson } from "@revscope/web-shared/apiRequest";
import type { AuthSession } from "@revscope/web-shared/authState";
import type { ConversionInterval } from "./dashboardApi";
import { appendSegmentParams, EMPTY_SEGMENT_FILTERS, type SegmentFilters } from "./segmentFilters";

export const PAGES_PAGE_SIZE = 200;

export type PagesOrderBy =
  | "url"
  | "visitors"
  | "attributed_revenue"
  | "conversion"
  | "bounce"
  | "exit"
  | "session_duration"
  | "page_duration"
  | "scroll";

export type PagesSortDirection = "asc" | "desc";

export type PagesSortState = {
  column: PagesOrderBy;
  direction: PagesSortDirection;
};

export const PAGES_SORT_COLUMNS = [
  "url",
  "visitors",
  "attributed_revenue",
  "conversion",
  "bounce",
  "exit",
  "session_duration",
  "page_duration",
  "scroll",
] as const satisfies readonly PagesOrderBy[];

/** Backend ranks by attributed revenue; mirror that as the initial sort. */
export const DEFAULT_PAGES_SORT: PagesSortState = {
  column: "attributed_revenue",
  direction: "desc",
};

function pagesOrderBy(column: string): PagesOrderBy {
  if ((PAGES_SORT_COLUMNS as readonly string[]).includes(column)) {
    return column as PagesOrderBy;
  }
  return DEFAULT_PAGES_SORT.column;
}

/**
 * A single page (a `paths` row) aggregated over the requested window. Metrics
 * cover every pageview of the path; session-scoped values (`converting_visitors`,
 * bounce/exit/duration) are de-duplicated per session, while `avg_scroll_percent`,
 * `avg_page_duration`, and `attributed_revenue` use individual pageviews.
 */
export type PageRow = {
  /** The visitors from imported data, which transitions can't show. */
  imported_visitors?: number;
  /** Stable `paths.id` for linking to a row. */
  id: number;
  /** Request path (always begins with `/`). */
  path: string;
  /** Hostname the path was served on. */
  hostname: string;
  /** Distinct sessions that viewed this page in the window. */
  visitors: number;
  /** Sessions viewing this page that also have attributed payment revenue. */
  converting_visitors: number;
  /**
   * Revenue weighted by this page's share of total session time.
   * Follows the pageview/landing cohort (not payment date).
   */
  attributed_revenue: number;
  /** Converting visitors / total visitors (0..1). */
  conversion_rate: number;
  /** Lower bound of the 95% Wilson interval, used for ranking. */
  conversion_lower_bound: number;
  /** 95% Wilson confidence interval for the conversion rate (absent when no visitors). */
  conversion_ci?: ConversionInterval;
  /** Share of visitors whose session lasted under 10 seconds (0..1). */
  bounce_rate: number;
  /** Share of visitors whose session ended on this page (0..1). */
  exit_rate: number;
  /** Mean session duration (seconds) across this page's visitors. */
  avg_session_duration: number;
  /** Mean dwell time (seconds) across pageviews that reported one. */
  avg_page_duration: number;
  /** Mean scroll depth (0..100) across pageviews that reported one. */
  avg_scroll_percent: number;
  /** Attributed revenue / tracked visitors (thousandths; 0 when none). */
  revenue_per_visitor: number;
  /** Marginal attributed revenue from lifting visitors by 25%. */
  increase_visitors_25pct: number;
  /** Marginal attributed revenue from lifting the conversion rate by 25%. */
  increase_conversion_25pct: number;
  /** Marginal attributed revenue from cutting the exit rate by 25%. */
  reduce_exits_25pct: number;
  /** Marginal attributed revenue from cutting the bounce rate by 25%. */
  reduce_bounces_25pct: number;
  /** Marginal attributed revenue from extending dwell time by 25%. */
  increase_session_25pct: number;
  /** Leads visitor uplift across all pages. */
  best_visitors_uplift: boolean;
  /** Leads conversion uplift among below-average-conversion pages. */
  best_conversion_uplift: boolean;
  /** Leads exit-reduction uplift among above-average-exit pages. */
  best_exit_uplift: boolean;
  /** Leads bounce-reduction uplift among above-average-bounce pages. */
  best_bounce_uplift: boolean;
  /** Leads session-duration uplift across all pages. */
  best_session_uplift: boolean;
  /** Highest exit rate among pages above the visitor-weighted average. */
  highest_exit_rate: boolean;
  /** Highest bounce rate among pages above the visitor-weighted average. */
  highest_bounce_rate: boolean;
};

export type PagesResponse = {
  /** Pages for the requested window, ranked by the requested `order_by`. */
  pages: PageRow[];
  next_offset?: number;
  offset: number;
  limit: number;
};

export type PageTransitionsVariant = "pages" | "landingPages";

export type PageTransitionRow = {
  /** Destination `paths.id`; absent for the synthetic Exit bucket. */
  path_id?: number;
  /** Destination request path; absent for the synthetic Exit bucket. */
  path?: string;
  /** Destination hostname; absent for the synthetic Exit bucket. */
  hostname?: string;
  /**
   * Outbound destination (hostname or `mailto:` domain) for exits that left via
   * a tracked outbound link. Set only when `path_id` is absent.
   */
  outbound_to?: string;
  /** Visitors whose next page view went to this destination. */
  visitors: number;
  /** Visitors divided by all visitors represented by the selected row. */
  share: number;
};

export type PageTransitionsResponse = {
  /** Visitors who viewed the selected page in this report context. */
  total_visitors: number;
  transitions: PageTransitionRow[];
};

async function fetchPagesFromApi(
  projectId: string,
  start: number,
  end: number,
  offset: number,
  sort: PagesSortState,
  resource: "pages" | "landing-pages",
  segments: SegmentFilters,
  session?: AuthSession,
): Promise<PagesResponse> {
  const url = new URL(projectApiPath("read", projectId, resource), getApiBaseUrl());
  url.searchParams.set("start", String(start));
  url.searchParams.set("end", String(end));
  url.searchParams.set("offset", String(offset));
  url.searchParams.set("order_by", pagesOrderBy(sort.column));
  url.searchParams.set("direction", sort.direction);
  appendSegmentParams(url, segments);

  return fetchJson<PagesResponse>(url, authRequestInit(session), "Unable to load pages data.");
}

export async function fetchPages(
  session: AuthSession | undefined,
  projectId: string,
  start: number,
  end: number,
  offset: number,
  sort: PagesSortState,
  segments: SegmentFilters = EMPTY_SEGMENT_FILTERS,
): Promise<PagesResponse> {
  return fetchPagesFromApi(projectId, start, end, offset, sort, "pages", segments, session);
}

export async function fetchLandingPages(
  session: AuthSession | undefined,
  projectId: string,
  start: number,
  end: number,
  offset: number,
  sort: PagesSortState,
  segments: SegmentFilters = EMPTY_SEGMENT_FILTERS,
): Promise<PagesResponse> {
  return fetchPagesFromApi(projectId, start, end, offset, sort, "landing-pages", segments, session);
}

export async function fetchPageTransitions(
  session: AuthSession | undefined,
  projectId: string,
  start: number,
  end: number,
  pathId: number,
  variant: PageTransitionsVariant,
  segments: SegmentFilters = EMPTY_SEGMENT_FILTERS,
): Promise<PageTransitionsResponse> {
  const url = new URL(projectApiPath("read", projectId, "page-transitions"), getApiBaseUrl());
  url.searchParams.set("start", String(start));
  url.searchParams.set("end", String(end));
  url.searchParams.set("path_id", String(pathId));
  url.searchParams.set("variant", variant === "landingPages" ? "landing_pages" : "pages");
  appendSegmentParams(url, segments);

  return fetchJson<PageTransitionsResponse>(
    url,
    authRequestInit(session),
    "Unable to load page transitions.",
  );
}
