import type {
  SearchTermRow,
  SearchTermsOrderBy,
  SearchTermsPeriod,
  SearchTermsSortState,
} from "../../lib/searchTermsApi";
import { DEFAULT_SEARCH_TERMS_SORT, SEARCH_TERMS_ORDER_BY_COLUMNS } from "../../lib/searchTermsApi";
import {
  buildHoverHighlights,
  type HighlightMetric,
  type HoverHighlight,
} from "./metricHoverDetails";
import type { RankingColumns, TreemapMetric } from "./treemapLogic";

export type {
  SearchTermsOrderBy,
  SearchTermsSortDirection,
  SearchTermsSortState,
} from "../../lib/searchTermsApi";

export { DEFAULT_SEARCH_TERMS_SORT, SEARCH_TERMS_ORDER_BY_COLUMNS };

/** @deprecated Prefer SearchTermsOrderBy — kept for preference migration. */
export type SearchTermsSortColumn = SearchTermsOrderBy;

export const SEARCH_TERMS_SORT_COLUMNS = SEARCH_TERMS_ORDER_BY_COLUMNS;

export const SEARCH_TERMS_OTHER_DATUM_KEY = "__search_terms_other__";

export function searchTermRowKey(term: SearchTermRow): string {
  return `term:${term.query}`;
}

/** Click-through rate (0..1), computed server-side. */
export function clickThroughRate(row: SearchTermRow): number {
  return row.ctr;
}

/** Mobile share of detailed clicks (0..1), computed server-side. */
export function mobileShare(row: SearchTermRow): number {
  return row.mobile_share;
}

export function hasSearchTermsPeriodData(period: SearchTermsPeriod): boolean {
  return period.terms.length > 0 || period.summary.clicks > 0 || period.summary.impressions > 0;
}

export const SEARCH_TERMS_RANKING_COLUMNS: RankingColumns<SearchTermsOrderBy> = {
  revenue: "estimated_revenue",
  traffic: "clicks",
};

/** The ranking the search-terms treemap loads: its metric's column, largest first. */
export function searchTermsTreemapSort(metric: TreemapMetric): SearchTermsSortState {
  return { orderBy: SEARCH_TERMS_RANKING_COLUMNS[metric], direction: "desc" };
}

/** True when the table sort matches the ranking the search-terms treemap uses. */
export function isSearchTermsTreemapSort(
  sort: SearchTermsSortState,
  metric: TreemapMetric,
): boolean {
  const treemapSort = searchTermsTreemapSort(metric);
  return sort.orderBy === treemapSort.orderBy && sort.direction === treemapSort.direction;
}

export function nextSearchTermsSortState(
  current: SearchTermsSortState,
  orderBy: SearchTermsOrderBy,
): SearchTermsSortState {
  if (current.orderBy === orderBy) {
    return {
      orderBy,
      direction: current.direction === "desc" ? "asc" : "desc",
    };
  }
  return { orderBy, direction: "desc" };
}

/** Hover card highlight lines for a row flagged as a top cannibalization or CTR leak. */
export function searchTermRowHighlights(row: SearchTermRow): HoverHighlight[] {
  const metrics: HighlightMetric[] = [];
  if (row.highest_cannibalization) {
    metrics.push("highestCannibalization");
  }
  if (row.highest_lost_clicks) {
    metrics.push("highestLostClicks");
  }
  return buildHoverHighlights(metrics);
}
