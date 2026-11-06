import { getApiBaseUrl } from "@revscope/web-shared/apiBaseUrl";
import { projectApiPath } from "@revscope/web-shared/apiPaths";
import { authRequestInit, fetchJson } from "@revscope/web-shared/apiRequest";
import type { AuthSession } from "@revscope/web-shared/authState";
import type { MetricChange } from "./metricChange";
import {
  appendSegmentParams,
  EMPTY_SEGMENT_FILTERS,
  type LandingAttribute,
  type SegmentFilters,
} from "./segmentFilters";

export type FunnelStepType = "pageview" | "payment";
export type FunnelPaymentType = "single" | "subscription" | "refund" | "chargeback";

export type FunnelStep = {
  id: number;
  order: number;
  step_type: FunnelStepType;
  event_name_id?: number;
  event_name?: string;
  path_id?: number;
  path?: string;
  hostname?: string;
  is_landing: boolean;
  payment_types: FunnelPaymentType[];
  description?: string;
};

export type Funnel = {
  id: number;
  name: string;
  description?: string;
  steps: FunnelStep[];
};

export type FunnelsResponse = {
  funnels: Funnel[];
};

export type FunnelOption = {
  id: number;
  name: string;
};

export type FunnelEventOption = FunnelOption & {
  path_ids: number[];
};

export type FunnelOptionsResponse = {
  paths: FunnelOption[];
  events: FunnelEventOption[];
};

export type FunnelStepData = {
  step_id: number;
  order: number;
  step_type: FunnelStepType;
  /** Generated label from the step configuration (path, event, or payment type). */
  name: string;
  description?: string;
  path?: string;
  event_name?: string;
  /** Page-anchored path for this step. Set for pageview steps; absent on payment steps. */
  path_id?: number;
  visitors: number;
  previous_visitors: number;
  /** Period-over-period change for `visitors` (previous window normalized to current length). */
  visitors_change: MetricChange;
  /** Share of this step's visitors that didn't reach the next step (0..1). Absent on the final step. */
  dropoff_rate?: number;
  /**
   * Destinations of sessions that left after this step. Set for non-final pageview
   * steps; absent on the final step and on payment steps.
   */
  dropoff?: FunnelStepDropoff;
  /** Total funnel revenue / this step's visitors, in thousandths. Set only when the final step is a payment. */
  revenue_per_visitor?: number;
};

export type FunnelDataResponse = {
  funnel_id: number;
  steps: FunnelStepData[];
  /** Total revenue across visitors who completed the funnel, in thousandths. Set only when the final step is a payment. */
  total_revenue?: number;
};

/** Bootstrap response when `funnel_id` is omitted from the funnel-data route. */
export type FunnelDataBootstrapResponse = {
  funnels: Funnel[];
  /** Data for the first funnel; absent when the project has no funnels. */
  data?: FunnelDataResponse;
};

/** A single drop-off destination for a funnel step. */
export type FunnelDropoffRow = {
  /** Destination `paths.id`; absent for the synthetic Exit bucket. */
  path_id?: number;
  /** Destination request path; absent for the synthetic Exit bucket. */
  path?: string;
  /** Destination hostname; absent for the synthetic Exit bucket. */
  hostname?: string;
  /**
   * Outbound destination (hostname or `mailto:` domain) for drop-offs that left
   * via a tracked outbound link. Set only when `path_id` is absent.
   */
  outbound_to?: string;
  /** Drop-off sessions whose next page view went to this destination. */
  visitors: number;
  /** Visitors divided by all counted drop-off sessions for the step (0..1). */
  share: number;
};

/** Drop-off destinations for a single funnel step (embedded in `FunnelStepData`). */
export type FunnelStepDropoff = {
  /** Sessions that left the funnel after the step and count as drop-off. */
  total_visitors: number;
  transitions: FunnelDropoffRow[];
};

export type SaveFunnelStep = {
  step_type: FunnelStepType;
  event_name_id?: number;
  path_id?: number;
  is_landing: boolean;
  payment_types: FunnelPaymentType[];
  description?: string;
};

export type SaveFunnelRequest = {
  id?: number;
  name: string;
  description?: string;
  steps: SaveFunnelStep[];
  start: number;
  end: number;
  previous_start: number;
  previous_end: number;
};

export type SaveFunnelResponse = {
  funnel: Funnel;
  data: FunnelDataResponse;
};

export type FunnelPreviewRequest = {
  steps: SaveFunnelStep[];
  start: number;
  end: number;
  previous_start: number;
  previous_end: number;
  /** Active global country segment filter (ISO alpha-2 codes), applied server-side. */
  countries?: string[];
  /** Active global revenue-source segment filter values, applied server-side. */
  sources?: string[];
  /** Active global page segment filter (`paths.id`), applied server-side. */
  pages?: number[];
  /** Active global landing-page segment filter (`paths.id`), applied server-side. */
  landing_pages?: number[];
  /** Active global event segment filter (`event_names.id`), applied server-side. */
  events?: number[];
  /** Active global landing attribute filters, applied server-side. */
  attributes?: { dimension: LandingAttribute; value: string }[];
};

export async function fetchFunnels(
  session: AuthSession | undefined,
  projectId: string,
): Promise<FunnelsResponse> {
  const url = new URL(projectApiPath("read", projectId, "funnels"), getApiBaseUrl());
  return fetchJson(url, authRequestInit(session), "Unable to load funnels.");
}

async function fetchFunnelDataFromApi(
  projectId: string,
  funnelId: number,
  start: number,
  end: number,
  previousStart: number,
  previousEnd: number,
  segments: SegmentFilters,
  session?: AuthSession,
): Promise<FunnelDataResponse>;
async function fetchFunnelDataFromApi(
  projectId: string,
  funnelId: undefined,
  start: number,
  end: number,
  previousStart: number,
  previousEnd: number,
  segments: SegmentFilters,
  session?: AuthSession,
): Promise<FunnelDataBootstrapResponse>;
async function fetchFunnelDataFromApi(
  projectId: string,
  funnelId: number | undefined,
  start: number,
  end: number,
  previousStart: number,
  previousEnd: number,
  segments: SegmentFilters,
  session?: AuthSession,
): Promise<FunnelDataResponse | FunnelDataBootstrapResponse> {
  const url = new URL(projectApiPath("read", projectId, "funnel-data"), getApiBaseUrl());
  if (funnelId !== undefined) {
    url.searchParams.set("funnel_id", String(funnelId));
  }
  url.searchParams.set("start", String(start));
  url.searchParams.set("end", String(end));
  url.searchParams.set("previous_start", String(previousStart));
  url.searchParams.set("previous_end", String(previousEnd));
  appendSegmentParams(url, segments);
  return fetchJson(url, authRequestInit(session), "Unable to load funnel data.");
}

export async function fetchFunnelData(
  session: AuthSession | undefined,
  projectId: string,
  funnelId: number,
  start: number,
  end: number,
  previousStart: number,
  previousEnd: number,
  segments: SegmentFilters = EMPTY_SEGMENT_FILTERS,
): Promise<FunnelDataResponse> {
  return fetchFunnelDataFromApi(
    projectId,
    funnelId,
    start,
    end,
    previousStart,
    previousEnd,
    segments,
    session,
  );
}

/** Load the funnel list and data for the first funnel in one request. */
export async function fetchFunnelDataBootstrap(
  session: AuthSession | undefined,
  projectId: string,
  start: number,
  end: number,
  previousStart: number,
  previousEnd: number,
  segments: SegmentFilters = EMPTY_SEGMENT_FILTERS,
): Promise<FunnelDataBootstrapResponse> {
  return fetchFunnelDataFromApi(
    projectId,
    undefined,
    start,
    end,
    previousStart,
    previousEnd,
    segments,
    session,
  );
}

export async function fetchFunnelDataPreview(
  session: AuthSession | undefined,
  projectId: string,
  request: FunnelPreviewRequest,
): Promise<FunnelDataResponse> {
  const url = new URL(projectApiPath("read", projectId, "funnel-preview"), getApiBaseUrl());
  return fetchJson(
    url,
    authRequestInit(session, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(request),
    }),
    "Unable to load funnel preview.",
  );
}

export async function fetchFunnelOptions(
  session: AuthSession | undefined,
  projectId: string,
): Promise<FunnelOptionsResponse> {
  const url = new URL(projectApiPath("read", projectId, "funnel-options"), getApiBaseUrl());
  return fetchJson(url, authRequestInit(session), "Unable to load funnel options.");
}

export async function saveFunnel(
  session: AuthSession,
  projectId: string,
  request: SaveFunnelRequest,
): Promise<SaveFunnelResponse> {
  const url = new URL(projectApiPath("session", projectId, "funnels"), getApiBaseUrl());
  return fetchJson(
    url,
    authRequestInit(session, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(request),
    }),
    "Unable to save funnel.",
  );
}

export async function deleteFunnel(
  session: AuthSession,
  projectId: string,
  funnelId: number,
): Promise<FunnelsResponse> {
  const url = new URL(projectApiPath("session", projectId, "funnels"), getApiBaseUrl());
  url.searchParams.set("funnel_id", String(funnelId));
  return fetchJson(url, authRequestInit(session, { method: "DELETE" }), "Unable to delete funnel.");
}
