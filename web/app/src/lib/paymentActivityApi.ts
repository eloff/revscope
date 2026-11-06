import { getApiBaseUrl } from "@revscope/web-shared/apiBaseUrl";
import { projectApiPath } from "@revscope/web-shared/apiPaths";
import { authRequestInit, fetchJson } from "@revscope/web-shared/apiRequest";
import type { AuthSession } from "@revscope/web-shared/authState";
import { appendSegmentParams, EMPTY_SEGMENT_FILTERS, type SegmentFilters } from "./segmentFilters";

/** Per-day payment counts and revenue split by category, for the activity graph. */
export type PaymentActivityDay = {
  /** Calendar day in the project's timezone, formatted as `YYYY-MM-DD`. */
  date: string;
  /** Non-subscription (single) payments on this day. */
  single: number;
  /** Revenue from non-subscription payments on this day, in thousandths of a dollar. */
  single_revenue: number;
  /** First-time subscription payments on this day. */
  new_subscription: number;
  /** Revenue from first-time subscription payments on this day, in thousandths of a dollar. */
  new_subscription_revenue: number;
  /** Renewal (recurring) subscription payments on this day. */
  recurring: number;
  /** Revenue from renewal subscription payments on this day, in thousandths of a dollar. */
  recurring_revenue: number;
};

/** Total revenue for one trailing 30-day "month" bucket. */
export type MonthlyRevenue = {
  /** 0 = the most recent trailing 30-day window, 1 = the 30 days before that, etc. */
  months_ago: number;
  /** Total revenue for that window, in thousandths of a dollar. */
  revenue: number;
};

export type PaymentActivityResponse = {
  /** Inclusive lower bound of the window, in Unix seconds. */
  start: number;
  /** Exclusive upper bound of the window, in Unix seconds. */
  end: number;
  /** Days within the window that have at least one payment, ascending by date. */
  days: PaymentActivityDay[];
  /**
   * Trailing 30-day revenue buckets covering up to the last 12 months,
   * ascending by recency (index 0 is the most recent). The revenue growth
   * trend is fit client-side from this data (see `runwayProjection.ts`).
   */
  monthly_revenue: MonthlyRevenue[];
};

export async function fetchPaymentActivity(
  session: AuthSession | undefined,
  projectId: string,
  segments: SegmentFilters = EMPTY_SEGMENT_FILTERS,
): Promise<PaymentActivityResponse> {
  const url = new URL(projectApiPath("read", projectId, "payment-activity"), getApiBaseUrl());
  appendSegmentParams(url, segments);
  return fetchJson<PaymentActivityResponse>(
    url,
    authRequestInit(session),
    "Unable to load payment activity.",
  );
}
