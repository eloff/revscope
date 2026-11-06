import { describe, expect, it } from "vitest";
import type { MonthlyRevenue } from "../../lib/paymentActivityApi";
import {
  computeHistoricalCashPoints,
  computeRevenueGrowthTrend,
  computeRunwayProjection,
  maxTrendWindowMonths,
  revenueDefaultsFromActivity,
  trendWindowOptions,
} from "./runwayProjection";

/** Monthly buckets, most recent first, from dollar amounts. */
function buckets(dollars: number[]): MonthlyRevenue[] {
  return dollars.map((amount, monthsAgo) => ({ months_ago: monthsAgo, revenue: amount * 1000 }));
}

describe("computeRevenueGrowthTrend", () => {
  it("recovers a steady compounding rate", () => {
    // 10% a month: each month is 1.1x the one before it.
    const history = buckets([1331, 1210, 1100, 1000]);
    const trend = computeRevenueGrowthTrend(history, 4);
    expect(trend.monthsOfHistory).toBe(4);
    expect(trend.monthlyPercent).toBeCloseTo(10, 6);
    expect(trend.annualizedPercent).toBeCloseTo((1.1 ** 12 - 1) * 100, 4);
  });

  it("reports shrinking revenue as negative growth", () => {
    const trend = computeRevenueGrowthTrend(buckets([500, 1000]), 2);
    expect(trend.monthlyPercent).toBeCloseTo(-50, 6);
  });

  it("only fits the buckets inside the window and skips zero months", () => {
    const history = buckets([1100, 1000, 0, 1]);
    expect(computeRevenueGrowthTrend(history, 4).monthsOfHistory).toBe(3);
    expect(computeRevenueGrowthTrend(history, 2).monthlyPercent).toBeCloseTo(10, 6);
  });

  it("needs two revenue-bearing months to fit a trend", () => {
    expect(computeRevenueGrowthTrend(buckets([1000]), 6)).toEqual({
      monthlyPercent: null,
      annualizedPercent: null,
      monthsOfHistory: 1,
    });
    expect(computeRevenueGrowthTrend(buckets([1000, 0]), 6).monthlyPercent).toBeNull();
  });
});

describe("trend window", () => {
  it("stops one month short of a gap in the history", () => {
    expect(maxTrendWindowMonths(buckets([5, 5, 5, 5, 0, 5, 5]))).toBe(3);
    expect(maxTrendWindowMonths(buckets(Array(12).fill(5)))).toBe(12);
    expect(maxTrendWindowMonths(buckets([5, 5, 5]))).toBe(3);
  });

  it("never offers fewer than two months", () => {
    expect(maxTrendWindowMonths(buckets([5, 0]))).toBe(2);
    expect(maxTrendWindowMonths([])).toBe(2);
    expect(trendWindowOptions(1).map((option) => option.months)).toEqual([2]);
    expect(trendWindowOptions(4)).toEqual([
      { months: 2, label: "2 months" },
      { months: 3, label: "3 months" },
      { months: 4, label: "4 months" },
    ]);
  });
});

describe("revenueDefaultsFromActivity", () => {
  it("takes this month's revenue in whole dollars", () => {
    const defaults = revenueDefaultsFromActivity(
      { start: 0, end: 0, days: [], monthly_revenue: buckets([1210.4, 1100, 1000]) },
      3,
    );
    expect(defaults.monthlyRevenue).toBe(1210);
    expect(defaults.monthsOfHistory).toBe(3);
    expect(defaults.growthRatePercent).toBeCloseTo(10, 1);
  });
});

describe("computeHistoricalCashPoints", () => {
  it("works backwards from today's cash using each month's revenue and the burn", () => {
    const points = computeHistoricalCashPoints(10_000, 3_000, buckets([1_000, 2_000]));
    expect(points).toEqual([
      { month: -2, cash: 13_000, revenue: 2_000 },
      { month: -1, cash: 12_000, revenue: 1_000 },
      { month: 0, cash: 10_000, revenue: 1_000 },
    ]);
  });
});

describe("computeRunwayProjection", () => {
  it("finds the fractional month cash runs out with flat revenue", () => {
    const projection = computeRunwayProjection({
      cash: 10_000,
      revenue: 1_000,
      burn: 5_000,
      growthRate: 0,
    });
    expect(projection.initialNetBurn).toBe(4_000);
    expect(projection.cashZeroMonth).toBeCloseTo(2.5, 10);
    expect(projection.breakEvenMonth).toBeNull();
    expect(projection.recoveryMonth).toBeNull();
    expect(projection.horizon).toBe(12);
    expect(projection.points).toHaveLength(13);
    expect(projection.points[1]).toEqual({ month: 1, cash: 6_000, revenue: 1_000, netBurn: 4_000 });
  });

  it("is profitable from the start when revenue covers burn", () => {
    const projection = computeRunwayProjection({
      cash: 1_000,
      revenue: 5_000,
      burn: 5_000,
      growthRate: 0.05,
    });
    expect(projection.breakEvenMonth).toBe(0);
    expect(projection.cashZeroMonth).toBeNull();
    expect(projection.horizon).toBe(12);
  });

  it("extends the horizon to show a dip below zero and the recovery", () => {
    const projection = computeRunwayProjection({
      cash: 1_000,
      revenue: 1_000,
      burn: 3_000,
      growthRate: 0.2,
    });
    const { cashZeroMonth, breakEvenMonth, recoveryMonth, horizon, points } = projection;
    expect(cashZeroMonth).not.toBeNull();
    expect(breakEvenMonth).not.toBeNull();
    expect(recoveryMonth).not.toBeNull();
    expect(recoveryMonth as number).toBeGreaterThan(cashZeroMonth as number);
    expect(horizon).toBe(
      Math.max((breakEvenMonth as number) + 12, Math.ceil(recoveryMonth as number) + 3),
    );
    expect(points).toHaveLength(horizon + 1);
    expect(points[Math.ceil(recoveryMonth as number)].cash).toBeGreaterThan(0);
  });

  it("starts out of cash when there is none", () => {
    expect(
      computeRunwayProjection({ cash: 0, revenue: 0, burn: 1_000, growthRate: 0 }).cashZeroMonth,
    ).toBe(0);
  });

  it("caps the simulation for growth that never catches up", () => {
    const projection = computeRunwayProjection({
      cash: 1e12,
      revenue: 1,
      burn: 1e9,
      growthRate: 0.0001,
    });
    expect(projection.horizon).toBeLessThanOrEqual(600);
    expect(projection.points.length).toBe(projection.horizon + 1);
  });
});
