import { getApiBaseUrl } from "@revscope/web-shared/apiBaseUrl";
import { projectApiPath } from "@revscope/web-shared/apiPaths";
import type { AuthSession } from "@revscope/web-shared/authState";

export { DEMO_PROJECT_ID } from "@revscope/web-shared/projectSelection";

import { authRequestInit, fetchJson } from "@revscope/web-shared/apiRequest";
import { appendSegmentParams, EMPTY_SEGMENT_FILTERS, type SegmentFilters } from "./segmentFilters";

export type OverviewBucket = {
  start: number;
  visitors: number;
  /** The visitors from imported data, who have no purchases to convert into. */
  imported_visitors: number;
  /** Distinct buyers with a first purchase (new subscription or single payment) in this bucket. */
  converting_visitors: number;
  /** First purchases in this bucket not linked to a tracked session or customer identity. */
  unattributed_converting_visitors: number;
  recurring_subscriptions: number;
  /** Count of renewal subscription payments in this bucket. */
  recurring_subscriptions_count: number;
  /** Count of renewal subscription payments billed annually. */
  recurring_annual_subscriptions_count: number;
  /** Gross payment revenue from first-time subscription payments (used by the stacked chart). */
  new_subscriptions: number;
  /** MRR added by new subscriptions in this bucket (first payment normalized to monthly). */
  new_subscriptions_mrr: number;
  /** Count of first-time subscription payments in this bucket. */
  new_subscriptions_count: number;
  /** Count of first-time subscription payments billed annually. */
  new_annual_subscriptions_count: number;
  churn: number;
  /** MRR removed by churned subscriptions in this bucket (last pre-churn payment normalized to monthly). */
  churn_mrr: number;
  /** Non-subscription (single) payment revenue in this bucket. */
  other_revenue: number;
  /** Count of non-subscription (single) payment events in this bucket. */
  other_payments_count: number;
  refunds: number;
  chargebacks: number;
  refunds_chargebacks_count: number;
  session_time_total: number;
  bounced_sessions: number;
};

export type BucketGranularity = "hourly" | "daily" | "weekly";

export type RevenuePeriodTotals = {
  /** Gross payment revenue (recurring + new subscriptions + other). */
  total_revenue: number;
  /** Gross payment revenue from new subscriptions and non-subscription payments. */
  new_revenue: number;
  /** Distinct buyers with a first purchase in the period. */
  converting_visitors: number;
  /** Average seconds from landing to purchase for attributed first purchases. */
  avg_time_to_purchase: number;
};

import type { MetricChange } from "./metricChange";

export type { MetricChange } from "./metricChange";

export type MetricChanges = Record<keyof PeriodMetrics, MetricChange>;

/** 95% Wilson score confidence interval for a conversion rate (0..1 bounds). */
export type ConversionInterval = {
  lower: number;
  upper: number;
};

export type MrrGrowthRates = {
  monthly: number;
  annualized: number;
};

export type OverviewResponse = {
  granularity: "hour" | "day" | "week";
  /** MRR snapshot at the period end. */
  mrr: number;
  /** MRR snapshot at the period start (= the previous period's end). */
  previous_mrr: number;
  /** Monthly and annualized MRR growth normalized from net MRR over the selected period. */
  mrr_growth?: MrrGrowthRates;
  current: RevenuePeriodTotals;
  /**
   * Raw revenue totals for the comparison window (the 30 days before the
   * selected range). Used only for rate comparisons (average revenue per
   * conversion, average time to purchase) where the normalization factor
   * cancels.
   */
  previous: RevenuePeriodTotals;
  /** Display-ready metric tile values for the selected period. */
  current_metrics: PeriodMetrics;
  /** Display-ready metric tile values for the normalized previous period. */
  previous_metrics: PeriodMetrics;
  /** Normalized previous-period bucket totals, for hover-card breakdowns. */
  previous_totals: BucketTotals;
  /** Period-over-period changes for hover-card bucket-total breakdown lines. */
  totals_changes: BucketTotalsChanges;
  /** Period-over-period changes for derived hover-card breakdown lines. */
  hover_changes: OverviewHoverChanges;
  /** Period-over-period change ratios for each metric tile. */
  changes: MetricChanges;
  /** 95% Wilson CI for the current period conversion rate. */
  conversion_rate_ci?: ConversionInterval;
  buckets: OverviewBucket[];
  /** Visitors and revenue by each filterable dimension, for sessions landed in the period. */
  breakdowns: OverviewBreakdowns;
};

/**
 * One value of a dimension. Each breakdown ignores its own filter, so its rows
 * stay the alternatives to what is selected.
 */
export type BreakdownRow = {
  /** The visitors from imported data, who have no revenue to convert into. */
  imported_visitors?: number;
  /**
   * The segment filter value selecting this row: a country code, a `source`
   * param, a `paths.id`, or a landing attribute value.
   */
  value: string;
  label: string;
  /** Present on campaign source rows, which are labeled client-side. */
  campaign?: {
    source: string;
    medium: string;
    campaign: string;
    network?: string;
    display_name?: string;
  };
  visitors: number;
  /** Visitors with attributed revenue. */
  converting_visitors: number;
  /** Revenue those visitors went on to pay, renewals excluded (thousandths). */
  revenue: number;
};

/**
 * The top rows by visitors, plus any others in the top by revenue, ordered by
 * visitors.
 */
export type OverviewBreakdowns = {
  sources: BreakdownRow[];
  channels: BreakdownRow[];
  /** The campaign rows of `sources`, ranked among themselves. */
  campaigns: BreakdownRow[];
  countries: BreakdownRow[];
  cities: BreakdownRow[];
  pages: BreakdownRow[];
  landing_pages: BreakdownRow[];
  browsers: BreakdownRow[];
  os: BreakdownRow[];
  devices: BreakdownRow[];
};

export type PeriodMetrics = {
  total_revenue: number;
  mrr: number;
  /** MRR added by new subscriptions in this period (monthly-normalized). */
  new_subscriptions_mrr: number;
  /** MRR removed by churned subscriptions in this period (monthly-normalized). */
  churn_mrr: number;
  /** Count of churned subscriptions in this period. */
  churn_count: number;
  new_revenue: number;
  visitors: number;
  refunds_and_chargebacks: number;
  revenue_per_visitor: number;
  conversion_rate: number;
  session_time: number;
  bounce_rate: number;
};

export type MetricsResponse = {
  current: PeriodMetrics;
  previous: PeriodMetrics;
};

const REVENUE_SCALE = 1000;

const DAY_SECONDS = 86_400;
/** Hourly buckets for ranges up to 3 days; weekly beyond 34 days. */
const HOURLY_BUCKET_MAX_DAYS = 3;
const WEEKLY_BUCKET_THRESHOLD_DAYS = 34;

export function bucketGranularityForRange(start: number, end: number): BucketGranularity {
  const rangeSeconds = end - start;
  if (rangeSeconds > WEEKLY_BUCKET_THRESHOLD_DAYS * DAY_SECONDS) {
    return "weekly";
  }
  if (rangeSeconds <= HOURLY_BUCKET_MAX_DAYS * DAY_SECONDS) {
    return "hourly";
  }
  return "daily";
}

export async function fetchOverview(
  session: AuthSession | undefined,
  projectId: string,
  start: number,
  end: number,
  segments: SegmentFilters = EMPTY_SEGMENT_FILTERS,
  bucket: BucketGranularity = bucketGranularityForRange(start, end),
): Promise<OverviewResponse> {
  const url = new URL(projectApiPath("read", projectId, "overview"), getApiBaseUrl());
  url.searchParams.set("start", String(start));
  url.searchParams.set("end", String(end));
  url.searchParams.set("bucket", bucket);
  appendSegmentParams(url, segments);

  return fetchJson<OverviewResponse>(
    url,
    authRequestInit(session),
    "Unable to load overview data.",
  );
}

export type BucketTotals = {
  visitors: number;
  /** The visitors from imported data, who have no purchases to convert into. */
  imported_visitors: number;
  /** Distinct buyers with a first purchase (new subscription or single payment) in this bucket. */
  converting_visitors: number;
  /** First purchases not linked to a tracked session or customer identity. */
  unattributed_converting_visitors: number;
  recurring: number;
  /** Count of renewal subscription payments in the period. */
  recurring_subscriptions_count: number;
  /** Count of renewal subscription payments billed annually. */
  recurring_annual_subscriptions_count: number;
  /** Gross first-time subscription payment revenue. */
  new_subscriptions: number;
  /** MRR added by new subscriptions in the period (monthly-normalized). */
  new_subscriptions_mrr: number;
  /** Count of first-time subscription payments in the period. */
  new_subscriptions_count: number;
  /** Count of first-time subscription payments billed annually. */
  new_annual_subscriptions_count: number;
  /** Count of churned subscriptions in the period. */
  churn_count: number;
  /** MRR removed by churned subscriptions in the period (monthly-normalized). */
  churn_mrr: number;
  /** Non-subscription (single) payment revenue. */
  other: number;
  /** Count of non-subscription (single) payment events. */
  other_payments_count: number;
  refunds: number;
  chargebacks: number;
  session_time_total: number;
  bounced_sessions: number;
  refunds_chargebacks_count: number;
};

export type BucketTotalsChanges = {
  [K in keyof Omit<BucketTotals, "imported_visitors">]: MetricChange;
};

/** Period-over-period changes for derived overview hover-card breakdown lines. */
export type OverviewHoverChanges = {
  converting_visitors: MetricChange;
  net_after_refunds: MetricChange;
  avg_payment: MetricChange;
  mrr_div_gross_payment: MetricChange;
  avg_mrr_per_churn: MetricChange;
  total_refund_outflows: MetricChange;
  conversion_share: MetricChange;
  engaged_visitors: MetricChange;
  avg_revenue_per_conversion: MetricChange;
  avg_time_to_purchase: MetricChange;
};

export function sumOverviewBuckets(buckets: OverviewBucket[]): BucketTotals {
  const totals: BucketTotals = {
    visitors: 0,
    imported_visitors: 0,
    converting_visitors: 0,
    unattributed_converting_visitors: 0,
    recurring: 0,
    recurring_subscriptions_count: 0,
    recurring_annual_subscriptions_count: 0,
    new_subscriptions: 0,
    new_subscriptions_mrr: 0,
    new_subscriptions_count: 0,
    new_annual_subscriptions_count: 0,
    churn_count: 0,
    churn_mrr: 0,
    other: 0,
    other_payments_count: 0,
    refunds: 0,
    chargebacks: 0,
    session_time_total: 0,
    bounced_sessions: 0,
    refunds_chargebacks_count: 0,
  };
  for (const bucket of buckets) {
    totals.visitors += bucket.visitors;
    totals.imported_visitors += bucket.imported_visitors;
    totals.converting_visitors += bucket.converting_visitors;
    totals.unattributed_converting_visitors += bucket.unattributed_converting_visitors;
    totals.recurring += bucket.recurring_subscriptions;
    totals.recurring_subscriptions_count += bucket.recurring_subscriptions_count;
    totals.recurring_annual_subscriptions_count += bucket.recurring_annual_subscriptions_count;
    totals.new_subscriptions += bucket.new_subscriptions;
    totals.new_subscriptions_mrr += bucket.new_subscriptions_mrr;
    totals.new_subscriptions_count += bucket.new_subscriptions_count;
    totals.new_annual_subscriptions_count += bucket.new_annual_subscriptions_count;
    totals.churn_count += bucket.churn;
    totals.churn_mrr += bucket.churn_mrr;
    totals.other += bucket.other_revenue;
    totals.other_payments_count += bucket.other_payments_count;
    totals.refunds += bucket.refunds;
    totals.chargebacks += bucket.chargebacks;
    totals.session_time_total += bucket.session_time_total;
    totals.bounced_sessions += bucket.bounced_sessions;
    totals.refunds_chargebacks_count += bucket.refunds_chargebacks_count;
  }
  return totals;
}

/** Gross revenue from new subscriptions and non-subscription payments (excludes recurring). */
export function visitorAttributedRevenue(
  parts: Pick<BucketTotals, "new_subscriptions" | "other">,
): number {
  return parts.new_subscriptions + parts.other;
}

/**
 * Current/previous metric tile values.
 *
 * Both are now computed and normalized on the backend (the previous period is
 * the prior 30 days, with additive flows scaled to the selected period length).
 * This adapter simply surfaces them in the existing `MetricsResponse` shape.
 */
export function deriveMetrics(response: OverviewResponse): MetricsResponse {
  return {
    current: response.current_metrics,
    previous: response.previous_metrics,
  };
}

/** Revenue amounts from the API are stored in thousandths of a dollar. */
export function formatRevenueAmount(amount: number): string {
  const dollars = amount / REVENUE_SCALE;
  const abs = Math.abs(dollars);
  if (abs >= 1_000_000) {
    return `$${(dollars / 1_000_000).toFixed(2)}M`;
  }
  if (abs >= 1000) {
    return `$${(dollars / 1000).toFixed(1)}k`;
  }
  if (Number.isInteger(dollars)) {
    return `$${dollars.toLocaleString()}`;
  }
  return `$${dollars.toFixed(2)}`;
}

export function formatRevenueAmountFixed(amount: number, decimals: number): string {
  return `$${(amount / REVENUE_SCALE).toLocaleString(undefined, {
    minimumFractionDigits: decimals,
    maximumFractionDigits: decimals,
  })}`;
}

export function formatSessionDuration(seconds: number): string {
  const total = Math.max(0, Math.round(seconds));
  if (total < 60) {
    return `${total}s`;
  }
  const minutes = Math.floor(total / 60);
  const remainder = total % 60;
  if (minutes < 60) {
    return remainder > 0 ? `${minutes}m ${remainder}s` : `${minutes}m`;
  }
  const hours = Math.floor(minutes / 60);
  const mins = minutes % 60;
  return mins > 0 ? `${hours}h ${mins}m` : `${hours}h`;
}

export function formatPercentRatio(ratio: number, decimals = 1): string {
  return `${(ratio * 100).toFixed(decimals)}%`;
}
