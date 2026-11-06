import { getApiBaseUrl } from "@revscope/web-shared/apiBaseUrl";
import { projectApiPath } from "@revscope/web-shared/apiPaths";
import { authRequestInit, fetchJson } from "@revscope/web-shared/apiRequest";
import type { AuthSession } from "@revscope/web-shared/authState";
import type { ConversionInterval } from "./dashboardApi";
import type { MetricChange } from "./metricChange";
import { appendSegmentParams, EMPTY_SEGMENT_FILTERS, type SegmentFilters } from "./segmentFilters";

export const EVENTS_PAGE_SIZE = 200;

export type EventsOrderBy =
  | "event"
  | "page"
  | "visitors"
  | "revenue"
  | "conversions"
  | "conversion";

export type EventsSortDirection = "asc" | "desc";

export type EventsSortState = {
  column: EventsOrderBy;
  direction: EventsSortDirection;
};

export const EVENTS_SORT_COLUMNS = [
  "event",
  "page",
  "visitors",
  "revenue",
  "conversions",
  "conversion",
] as const satisfies readonly EventsOrderBy[];

/** Backend ranks by revenue; mirror that as the initial sort. */
export const DEFAULT_EVENTS_SORT: EventsSortState = {
  column: "revenue",
  direction: "desc",
};

function eventsOrderBy(column: string): EventsOrderBy {
  if ((EVENTS_SORT_COLUMNS as readonly string[]).includes(column)) {
    return column as EventsOrderBy;
  }
  return DEFAULT_EVENTS_SORT.column;
}

export type EventRow = {
  event_name_id: number;
  event_name: string;
  path_id: number;
  path: string;
  hostname: string;
  goal_id?: number;
  visitors: number;
  revenue: number;
  conversions: number;
  conversion_rate: number;
  conversion_lower_bound: number;
  conversion_ci?: ConversionInterval;
  revenue_change: MetricChange;
  visitors_change: MetricChange;
  conversions_change: MetricChange;
  conversion_rate_change: MetricChange;
};

export type EventsResponse = {
  events: EventRow[];
  next_offset?: number;
  offset: number;
  limit: number;
};

export async function fetchEvents(
  session: AuthSession | undefined,
  projectId: string,
  start: number,
  end: number,
  offset: number,
  sort: EventsSortState,
  segments: SegmentFilters = EMPTY_SEGMENT_FILTERS,
): Promise<EventsResponse> {
  const url = new URL(projectApiPath("read", projectId, "events"), getApiBaseUrl());
  url.searchParams.set("start", String(start));
  url.searchParams.set("end", String(end));
  url.searchParams.set("offset", String(offset));
  url.searchParams.set("order_by", eventsOrderBy(sort.column));
  url.searchParams.set("direction", sort.direction);
  appendSegmentParams(url, segments);

  return fetchJson<EventsResponse>(url, authRequestInit(session), "Unable to load events data.");
}
