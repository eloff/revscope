import { getApiBaseUrl } from "@revscope/web-shared/apiBaseUrl";
import { projectApiPath } from "@revscope/web-shared/apiPaths";
import { authRequestInit, fetchJson } from "@revscope/web-shared/apiRequest";
import type { AuthSession } from "@revscope/web-shared/authState";
import type { PaymentCampaign, PaymentJourneyAction, PaymentReferrer } from "./paymentsApi";
import type { ReferrerType } from "./revenueSourcesApi";
import { appendSegmentParams, EMPTY_SEGMENT_FILTERS, type SegmentFilters } from "./segmentFilters";

export const VISITORS_PAGE_SIZE = 100;

export type VisitorOrderBy =
  | "source"
  | "last_view_at"
  | "active_duration"
  | "session_age"
  | "pages_viewed";

export type VisitorSortDirection = "asc" | "desc";

export type VisitorSortState = {
  orderBy: VisitorOrderBy;
  direction: VisitorSortDirection;
};

export type VisitorSessionFilter =
  | { kind: "pageview"; pathId: number }
  | { kind: "landing_page"; pathId: number }
  | { kind: "event"; eventNameId: number; pathId: number };

export const VISITOR_ORDER_BY_COLUMNS = [
  "source",
  "last_view_at",
  "active_duration",
  "session_age",
  "pages_viewed",
] as const satisfies readonly VisitorOrderBy[];

export const DEFAULT_VISITOR_SORT: VisitorSortState = {
  orderBy: "last_view_at",
  direction: "desc",
};

export type VisitorEntry = {
  /** 16-char lowercase hex encoding of the nanosecond session timestamp id. */
  visitor_timestamp_id: string;
  viewed_at: number;
  last_view_at: number;
  session_duration_seconds: number;
  session_page_count: number;
  click_id?: string;
  referrer_type?: ReferrerType;
  is_google: boolean;
  timezone?: string;
  language?: string;
  country?: string;
  city?: string;
  tracker_version: number;
  os?: string;
  os_version?: number;
  browser_family?: string;
  device: string;
  screen_width: number;
  screen_height: number;
  viewport_width: number;
  viewport_height: number;
  referrer?: PaymentReferrer;
  campaign?: PaymentCampaign;
};

export type VisitorsFilterInfo = {
  kind: "pageview" | "landing_page" | "event";
  path: string;
  hostname: string;
  event_name?: string;
};

export type VisitorsResponse = {
  visitors: VisitorEntry[];
  next_offset?: number;
  offset: number;
  limit: number;
  /** Resolved description of the active session filter, absent when unfiltered. */
  filter?: VisitorsFilterInfo;
};

export type VisitorJourneyResponse = {
  visitor: VisitorEntry;
  timeline: PaymentJourneyAction[];
};

export async function fetchVisitors(
  session: AuthSession | undefined,
  projectId: string,
  start: number,
  end: number,
  offset: number,
  sort: VisitorSortState,
  filter?: VisitorSessionFilter | null,
  segments: SegmentFilters = EMPTY_SEGMENT_FILTERS,
): Promise<VisitorsResponse> {
  const url = new URL(projectApiPath("read", projectId, "visitors"), getApiBaseUrl());
  url.searchParams.set("start", String(start));
  url.searchParams.set("end", String(end));
  url.searchParams.set("offset", String(offset));
  url.searchParams.set("order_by", sort.orderBy);
  url.searchParams.set("direction", sort.direction);
  setVisitorFilterParams(url.searchParams, filter);
  appendSegmentParams(url, segments);

  return fetchJson<VisitorsResponse>(
    url,
    authRequestInit(session),
    "Unable to load visitors data.",
  );
}

export function visitorSessionFilterKey(filter?: VisitorSessionFilter | null): string {
  if (!filter) {
    return "all";
  }
  if (filter.kind === "event") {
    return `event:${filter.eventNameId}:path:${filter.pathId}`;
  }
  return `${filter.kind}:path:${filter.pathId}`;
}

/**
 * Human-readable label for the active visitors filter. Uses the resolved
 * `info` from the backend when available, falling back to a generic label
 * (e.g. while the first page is still loading) so the chip can appear
 * immediately on navigation.
 */
export function describeVisitorSessionFilter(
  filter: VisitorSessionFilter,
  info?: VisitorsFilterInfo,
): string {
  const page = info ? `${info.hostname}${info.path}` : "the selected page";
  switch (filter.kind) {
    case "pageview":
      return `Visitors who viewed ${page}`;
    case "landing_page":
      return `Visitors who landed on ${page}`;
    case "event":
      return `Visitors who triggered ${info?.event_name ?? "an event"} on ${page}`;
  }
}

export function setVisitorFilterParams(
  params: URLSearchParams,
  filter?: VisitorSessionFilter | null,
): void {
  params.delete("filter");
  params.delete("path_id");
  params.delete("event_name_id");
  if (!filter) {
    return;
  }
  params.set("filter", filter.kind);
  params.set("path_id", String(filter.pathId));
  if (filter.kind === "event") {
    params.set("event_name_id", String(filter.eventNameId));
  }
}

export async function fetchVisitorJourney(
  session: AuthSession | undefined,
  projectId: string,
  visitorTimestampId: string,
): Promise<VisitorJourneyResponse> {
  const url = new URL(projectApiPath("read", projectId, "visitor-journey"), getApiBaseUrl());
  url.searchParams.set("visitor_timestamp_id", visitorTimestampId);

  return fetchJson<VisitorJourneyResponse>(
    url,
    authRequestInit(session),
    "Unable to load visitor journey.",
  );
}
