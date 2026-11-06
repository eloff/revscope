import type {
  BreakdownRow,
  BucketTotals,
  OverviewBreakdowns,
  OverviewBucket,
  OverviewResponse,
  PeriodMetrics,
} from "../src/lib/dashboardApi";
import type { MetricChange } from "../src/lib/metricChange";
import type { PaymentEntry, PaymentLanding } from "../src/lib/paymentsApi";
import type { VisitorEntry } from "../src/lib/visitorsApi";

const NO_CHANGE: MetricChange = { ratio: null };

/** Every key of `T` mapped to "no comparison"; tests override the changes they assert. */
function noChanges<K extends string>(keys: readonly K[]): Record<K, MetricChange> {
  return Object.fromEntries(keys.map((key) => [key, NO_CHANGE])) as Record<K, MetricChange>;
}

export function testBucket(overrides: Partial<OverviewBucket> = {}): OverviewBucket {
  return {
    start: 0,
    visitors: 0,
    imported_visitors: 0,
    converting_visitors: 0,
    unattributed_converting_visitors: 0,
    recurring_subscriptions: 0,
    recurring_subscriptions_count: 0,
    recurring_annual_subscriptions_count: 0,
    new_subscriptions: 0,
    new_subscriptions_mrr: 0,
    new_subscriptions_count: 0,
    new_annual_subscriptions_count: 0,
    churn: 0,
    churn_mrr: 0,
    other_revenue: 0,
    other_payments_count: 0,
    refunds: 0,
    chargebacks: 0,
    refunds_chargebacks_count: 0,
    session_time_total: 0,
    bounced_sessions: 0,
    ...overrides,
  };
}

export function testPeriodMetrics(overrides: Partial<PeriodMetrics> = {}): PeriodMetrics {
  return {
    total_revenue: 0,
    mrr: 0,
    new_subscriptions_mrr: 0,
    churn_mrr: 0,
    churn_count: 0,
    new_revenue: 0,
    visitors: 0,
    refunds_and_chargebacks: 0,
    revenue_per_visitor: 0,
    conversion_rate: 0,
    session_time: 0,
    bounce_rate: 0,
    ...overrides,
  };
}

const PERIOD_METRIC_KEYS = Object.keys(testPeriodMetrics()) as (keyof PeriodMetrics)[];
const BUCKET_TOTAL_KEYS: (keyof BucketTotals)[] = [
  "visitors",
  "converting_visitors",
  "unattributed_converting_visitors",
  "recurring",
  "recurring_subscriptions_count",
  "recurring_annual_subscriptions_count",
  "new_subscriptions",
  "new_subscriptions_mrr",
  "new_subscriptions_count",
  "new_annual_subscriptions_count",
  "churn_count",
  "churn_mrr",
  "other",
  "other_payments_count",
  "refunds",
  "chargebacks",
  "session_time_total",
  "bounced_sessions",
  "refunds_chargebacks_count",
];
const HOVER_CHANGE_KEYS = [
  "converting_visitors",
  "net_after_refunds",
  "avg_payment",
  "mrr_div_gross_payment",
  "avg_mrr_per_churn",
  "total_refund_outflows",
  "conversion_share",
  "engaged_visitors",
  "avg_revenue_per_conversion",
  "avg_time_to_purchase",
] as const;

const emptyTotals = {
  total_revenue: 0,
  new_revenue: 0,
  converting_visitors: 0,
  avg_time_to_purchase: 0,
};

/** A valid `/overview` payload with no data; pass the fields a test cares about. */
export function testOverview(overrides: Partial<OverviewResponse> = {}): OverviewResponse {
  return {
    granularity: "day",
    mrr: 0,
    previous_mrr: 0,
    current: { ...emptyTotals },
    previous: { ...emptyTotals },
    current_metrics: testPeriodMetrics(),
    previous_metrics: testPeriodMetrics(),
    previous_totals: Object.fromEntries(BUCKET_TOTAL_KEYS.map((key) => [key, 0])) as BucketTotals,
    totals_changes: noChanges(BUCKET_TOTAL_KEYS),
    hover_changes: noChanges(HOVER_CHANGE_KEYS),
    changes: noChanges(PERIOD_METRIC_KEYS),
    buckets: [],
    breakdowns: emptyBreakdowns(),
    ...overrides,
  };
}

export function emptyBreakdowns(): OverviewBreakdowns {
  return {
    sources: [],
    channels: [],
    campaigns: [],
    countries: [],
    cities: [],
    pages: [],
    landing_pages: [],
    browsers: [],
    os: [],
    devices: [],
  };
}

export function testBreakdownRow(overrides: Partial<BreakdownRow> = {}): BreakdownRow {
  return {
    value: "US",
    label: "United States",
    visitors: 10,
    converting_visitors: 1,
    revenue: 0,
    ...overrides,
  };
}

export function testLanding(overrides: Partial<PaymentLanding> = {}): PaymentLanding {
  return {
    visitor_timestamp_id: 1,
    viewed_at: 1_700_000_000,
    session_duration_seconds: 120,
    session_page_count: 3,
    is_google: false,
    tracker_version: 1,
    device: "desktop",
    screen_width: 1920,
    screen_height: 1080,
    viewport_width: 1440,
    viewport_height: 900,
    ...overrides,
  } as PaymentLanding;
}

export function testPayment(overrides: Partial<PaymentEntry> = {}): PaymentEntry {
  return {
    id: 1,
    provider: "stripe",
    provider_payment_id: "pi_1",
    payment_type: "charge",
    status: "succeeded",
    amount: 49_000,
    currency: "usd",
    paid_at: 1_700_000_000 + 3600,
    created_at: 1_700_000_000 + 3600,
    ...overrides,
  };
}

export function testVisitor(overrides: Partial<VisitorEntry> = {}): VisitorEntry {
  return {
    visitor_timestamp_id: "0123456789abcdef",
    viewed_at: 1_700_000_000,
    last_view_at: 1_700_000_000 + 600,
    session_duration_seconds: 95,
    session_page_count: 4,
    is_google: false,
    tracker_version: 1,
    device: "desktop",
    screen_width: 1920,
    screen_height: 1080,
    viewport_width: 1440,
    viewport_height: 900,
    ...overrides,
  };
}
