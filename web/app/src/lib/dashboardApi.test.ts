import { describe, expect, it } from "vitest";
import {
  bucketGranularityForRange,
  deriveMetrics,
  formatPercentRatio,
  formatRevenueAmount,
  formatRevenueAmountFixed,
  formatSessionDuration,
  type OverviewBucket,
  type OverviewResponse,
  sumOverviewBuckets,
  visitorAttributedRevenue,
} from "./dashboardApi";

const DAY = 86_400;

function bucket(overrides: Partial<OverviewBucket> = {}): OverviewBucket {
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

describe("bucketGranularityForRange", () => {
  it.each([
    [1, "hourly"],
    [3, "hourly"],
    [3.5, "daily"],
    [34, "daily"],
    [35, "weekly"],
  ])("uses %d days → %s", (days, expected) => {
    expect(bucketGranularityForRange(0, days * DAY)).toBe(expected);
  });
});

describe("sumOverviewBuckets", () => {
  it("sums every field, mapping bucket names onto totals", () => {
    const totals = sumOverviewBuckets([
      bucket({ visitors: 3, recurring_subscriptions: 100, churn: 1, other_revenue: 5 }),
      bucket({ visitors: 4, recurring_subscriptions: 50, churn: 2, refunds: 7, chargebacks: 1 }),
    ]);
    expect(totals).toMatchObject({
      visitors: 7,
      recurring: 150,
      churn_count: 3,
      other: 5,
      refunds: 7,
      chargebacks: 1,
    });
  });

  it("returns zeros for no buckets", () => {
    expect(Object.values(sumOverviewBuckets([])).every((value) => value === 0)).toBe(true);
  });
});

describe("visitorAttributedRevenue", () => {
  it("adds new subscriptions and one-off payments", () => {
    expect(visitorAttributedRevenue({ new_subscriptions: 10, other: 5 })).toBe(15);
  });
});

describe("deriveMetrics", () => {
  it("surfaces the backend's current and previous tiles", () => {
    const current = { visitors: 1 } as OverviewResponse["current_metrics"];
    const previous = { visitors: 2 } as OverviewResponse["previous_metrics"];
    expect(
      deriveMetrics({ current_metrics: current, previous_metrics: previous } as OverviewResponse),
    ).toEqual({ current, previous });
  });
});

describe("formatRevenueAmount", () => {
  it.each([
    [0, "$0"],
    [12_000, "$12"],
    [12_345, "$12.35"],
    [999_000, "$999"],
    [1_500_000, "$1.5k"],
    [-2_500_000, "$-2.5k"],
    [3_210_000_000, "$3.21M"],
  ])("formats %d thousandths as %s", (amount, expected) => {
    expect(formatRevenueAmount(amount)).toBe(expected);
  });

  it("formats with fixed decimals", () => {
    expect(formatRevenueAmountFixed(12_000, 2)).toBe(
      `$${(12).toLocaleString(undefined, { minimumFractionDigits: 2 })}`,
    );
  });
});

describe("formatSessionDuration", () => {
  it.each([
    [-4, "0s"],
    [42.4, "42s"],
    [60, "1m"],
    [95, "1m 35s"],
    [3600, "1h"],
    [3_900, "1h 5m"],
  ])("formats %d seconds as %s", (seconds, expected) => {
    expect(formatSessionDuration(seconds)).toBe(expected);
  });
});

describe("formatPercentRatio", () => {
  it("formats a ratio as a percentage", () => {
    expect(formatPercentRatio(0.1234)).toBe("12.3%");
    expect(formatPercentRatio(0.5, 0)).toBe("50%");
  });
});
