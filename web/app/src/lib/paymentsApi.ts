import { getApiBaseUrl } from "@revscope/web-shared/apiBaseUrl";
import { projectApiPath } from "@revscope/web-shared/apiPaths";
import { authRequestInit, fetchJson } from "@revscope/web-shared/apiRequest";
import type { AuthSession } from "@revscope/web-shared/authState";
import type { ReferrerType } from "./revenueSourcesApi";
import { appendSegmentParams, EMPTY_SEGMENT_FILTERS, type SegmentFilters } from "./segmentFilters";

export type PaymentReferrer = {
  id: number;
  referrer: string;
  deleted_at?: number;
};

export type PaymentCampaign = {
  id: number;
  source: string;
  medium: string;
  campaign: string;
  network?: string;
  display_name?: string;
  first_seen_at: number;
  deleted_at?: number;
};

export type PaymentLanding = {
  visitor_timestamp_id: number;
  viewed_at: number;
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

export type PaymentEntry = {
  id: number;
  provider: string;
  subscription_id?: number;
  provider_subscription_id?: string;
  interval_unit?: string;
  visitor_timestamp_id?: number;
  provider_payment_id: string;
  provider_order_id?: string;
  provider_invoice_id?: string;
  provider_customer_id?: string;
  customer_email?: string;
  order_email?: string;
  payment_type: string;
  status: string;
  amount: number;
  mrr_amount?: number;
  currency: string;
  fee_amount?: number;
  net_amount?: number;
  paid_at: number;
  created_at: number;
  metadata?: Record<string, unknown>;
  landing?: PaymentLanding;
};

export type ConversionsResponse = {
  conversions: PaymentEntry[];
  next_cursor?: string;
};

import type { SourceGroup } from "./revenueSourcesApi";

export type PaymentsFilter = {
  pathId?: number;
  landingPathId?: number;
  campaignId?: number;
  referrerId?: number;
  direct?: boolean;
};

export type PaymentJourneyActionKind = "source" | "pageview" | "event" | "purchase";

export type PaymentJourneyAction = {
  kind: PaymentJourneyActionKind;
  occurred_at: number;
  title?: string;
  name?: string;
  subtitle?: string;
  hostname?: string;
  path?: string;
  path_id?: number;
  duration_seconds?: number;
  scrolled_percent?: number;
  metadata?: Record<string, unknown>;
  source_group?: SourceGroup;
  goal_id?: number;
};

export type PaymentJourneyResponse = {
  payment: PaymentEntry;
  timeline: PaymentJourneyAction[];
};

/** Page size for the conversions tab and nested conversion drill-downs (matches the API default). */
export const CONVERSIONS_PAGE_SIZE = 100;

async function fetchConversionsPageFromApi(
  projectId: string,
  start: number,
  end: number,
  segments: SegmentFilters,
  session: AuthSession | undefined,
  filters: PaymentsFilter,
  cursor: string | undefined,
  limit: number,
): Promise<ConversionsResponse> {
  const url = new URL(projectApiPath("read", projectId, "conversions"), getApiBaseUrl());
  url.searchParams.set("start", String(start));
  url.searchParams.set("end", String(end));
  url.searchParams.set("limit", String(limit));
  appendSegmentParams(url, segments);
  if (filters.pathId !== undefined) {
    url.searchParams.set("path_id", String(filters.pathId));
  }
  if (filters.landingPathId !== undefined) {
    url.searchParams.set("landing_path_id", String(filters.landingPathId));
  }
  if (filters.campaignId !== undefined) {
    url.searchParams.set("campaign_id", String(filters.campaignId));
  }
  if (filters.referrerId !== undefined) {
    url.searchParams.set("referrer_id", String(filters.referrerId));
  }
  if (filters.direct) {
    url.searchParams.set("direct", "1");
  }
  if (cursor) {
    url.searchParams.set("cursor", cursor);
  }

  return fetchJson<ConversionsResponse>(
    url,
    authRequestInit(session),
    "Unable to load conversions data.",
  );
}

export async function fetchConversions(
  session: AuthSession | undefined,
  projectId: string,
  start: number,
  end: number,
  cursor?: string,
  filters: PaymentsFilter = {},
  segments: SegmentFilters = EMPTY_SEGMENT_FILTERS,
): Promise<ConversionsResponse> {
  return fetchConversionsPageFromApi(
    projectId,
    start,
    end,
    segments,
    session,
    filters,
    cursor,
    CONVERSIONS_PAGE_SIZE,
  );
}

export async function fetchPayments(
  session: AuthSession | undefined,
  projectId: string,
  start: number,
  end: number,
  filters: PaymentsFilter = {},
  segments: SegmentFilters = EMPTY_SEGMENT_FILTERS,
  cursor?: string,
): Promise<ConversionsResponse> {
  return fetchConversionsPageFromApi(
    projectId,
    start,
    end,
    segments,
    session,
    filters,
    cursor,
    CONVERSIONS_PAGE_SIZE,
  );
}

export async function fetchPaymentJourney(
  session: AuthSession | undefined,
  projectId: string,
  paymentId: number,
): Promise<PaymentJourneyResponse> {
  const url = new URL(projectApiPath("read", projectId, "payment-journey"), getApiBaseUrl());
  url.searchParams.set("payment_id", String(paymentId));

  return fetchJson<PaymentJourneyResponse>(
    url,
    authRequestInit(session),
    "Unable to load customer journey.",
  );
}
