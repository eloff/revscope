import type { MonthlyRevenue, PaymentActivityResponse } from "../../lib/paymentActivityApi";

/** Revenue amounts from the payment activity API are in thousandths of a dollar. */
const REVENUE_SCALE = 1000;
/** Hard cap on simulated months so runaway/extreme inputs can't loop forever. */
const MAX_MONTHS = 600;
/** Minimum number of revenue-bearing monthly buckets required to fit a trend
 * line; below this a slope is too noisy/underdetermined to report. Also the
 * shortest trailing window selectable for the growth trend, since a
 * single-month window can never produce a trend. */
export const MIN_TREND_WINDOW_MONTHS = 2;
/** Longest trailing window selectable for the growth trend, matching the
 * number of monthly buckets the backend returns. */
export const MAX_TREND_WINDOW_MONTHS = 12;
/** Default trailing window for the growth trend dropdown and fitted rate. */
export const DEFAULT_TREND_WINDOW_MONTHS = 6;

/**
 * Caps the trailing window selectable for the growth trend to the history
 * actually available, and so it never spans a zero-revenue month. The
 * backend only emits buckets that overlap observed history, so a project
 * younger than a year simply has fewer buckets (rather than zero-padding
 * the rest). A zero bucket within that history usually means there's no
 * data for that period (e.g. a gap before the project had any payments)
 * rather than a genuine zero-revenue month, and the month right after such
 * a gap is typically a partial ramp-up period, so the window stops one
 * month short of it.
 */
export function maxTrendWindowMonths(monthlyRevenue: MonthlyRevenue[]): number {
  let firstZeroMonthsAgo: number | null = null;
  for (const bucket of monthlyRevenue) {
    if (
      bucket.revenue === 0 &&
      (firstZeroMonthsAgo === null || bucket.months_ago < firstZeroMonthsAgo)
    ) {
      firstZeroMonthsAgo = bucket.months_ago;
    }
  }
  const zeroCap = firstZeroMonthsAgo === null ? MAX_TREND_WINDOW_MONTHS : firstZeroMonthsAgo - 1;
  return Math.max(
    MIN_TREND_WINDOW_MONTHS,
    Math.min(MAX_TREND_WINDOW_MONTHS, monthlyRevenue.length, zeroCap),
  );
}

/** Selectable options for the growth trend window dropdown, from
 * `MIN_TREND_WINDOW_MONTHS` up to `maxMonths`. */
export function trendWindowOptions(
  maxMonths: number = MAX_TREND_WINDOW_MONTHS,
): { months: number; label: string }[] {
  const clamped = Math.min(MAX_TREND_WINDOW_MONTHS, Math.max(MIN_TREND_WINDOW_MONTHS, maxMonths));
  return Array.from({ length: clamped - MIN_TREND_WINDOW_MONTHS + 1 }, (_, index) => {
    const months = MIN_TREND_WINDOW_MONTHS + index;
    return { months, label: `${months} months` };
  });
}

/**
 * A compounding revenue growth rate estimated by fitting an exponential
 * trend across a trailing window of monthly revenue buckets (ordinary least
 * squares regression of `ln(revenue)` against the bucket's age in months).
 *
 * This is computed client-side (rather than comparing two fixed-length
 * windows, e.g. "last 365 days" vs. "the 365 days before that") because that
 * comparison is undefined whenever the project doesn't yet have a full
 * previous window of history, and because anchoring on a single "first month
 * with revenue" as a baseline is skewed by that month almost always being a
 * partial/ramp-up period. Fitting a trend line across the selected window of
 * monthly buckets smooths over that noise.
 */
export type RevenueGrowthTrend = {
  /**
   * Estimated month-over-month compounding growth rate, as a percent, or
   * `null` when there isn't enough revenue-bearing history in the selected
   * window to fit a trend.
   */
  monthlyPercent: number | null;
  /** The monthly rate compounded over a year, or `null` under the same condition. */
  annualizedPercent: number | null;
  /** Number of revenue-bearing monthly buckets the trend was fit against. */
  monthsOfHistory: number;
};

/**
 * Fits an exponential trend across the revenue-bearing monthly buckets
 * within `windowMonths` (i.e. `months_ago < windowMonths`) via ordinary
 * least squares on `(-months_ago, ln(revenue))`, so growth compounds forward
 * in time. Buckets with zero revenue are excluded (undefined log) rather
 * than treated as a hard floor; with fewer than [`MIN_TREND_WINDOW_MONTHS`]
 * remaining points the slope would be underdetermined or a single-pair
 * comparison, so `null` is reported instead.
 */
export function computeRevenueGrowthTrend(
  monthlyRevenue: MonthlyRevenue[],
  windowMonths: number,
): RevenueGrowthTrend {
  const points = monthlyRevenue
    .filter((bucket) => bucket.months_ago < windowMonths && bucket.revenue > 0)
    .map((bucket) => ({
      x: -bucket.months_ago,
      y: Math.log(bucket.revenue),
    }));
  const monthsOfHistory = points.length;

  if (monthsOfHistory < MIN_TREND_WINDOW_MONTHS) {
    return { monthlyPercent: null, annualizedPercent: null, monthsOfHistory };
  }

  const n = points.length;
  const sumX = points.reduce((sum, point) => sum + point.x, 0);
  const sumY = points.reduce((sum, point) => sum + point.y, 0);
  const sumXX = points.reduce((sum, point) => sum + point.x * point.x, 0);
  const sumXY = points.reduce((sum, point) => sum + point.x * point.y, 0);

  // Bucket ages are distinct integers, so this is zero only in degenerate
  // (e.g. single-point) cases already excluded above.
  const denominator = n * sumXX - sumX * sumX;
  const slope = (n * sumXY - sumX * sumY) / denominator;
  const monthlyMultiplier = Math.exp(slope);
  const monthlyPercent = (monthlyMultiplier - 1) * 100;
  const annualizedPercent = (monthlyMultiplier ** 12 - 1) * 100;

  if (!Number.isFinite(monthlyPercent) || !Number.isFinite(annualizedPercent)) {
    return { monthlyPercent: null, annualizedPercent: null, monthsOfHistory };
  }
  return { monthlyPercent, annualizedPercent, monthsOfHistory };
}

/** Defaults derived from recent payment history, in whole dollars / percent. */
export type RevenueDefaults = {
  /** Total gross revenue over the last 30 days, in dollars. */
  monthlyRevenue: number;
  /**
   * Month-over-month compounding growth rate as a percentage, fitted from
   * the trend across the selected window of monthly revenue buckets. `null`
   * when there isn't enough revenue-bearing history yet to fit a trend.
   */
  growthRatePercent: number | null;
  /** Number of revenue-bearing monthly buckets the trend was fit against. */
  monthsOfHistory: number;
};

/**
 * Read the default monthly revenue (the last 30 days) and fit the growth
 * trend across the trailing `trendWindowMonths` monthly buckets.
 */
export function revenueDefaultsFromActivity(
  activity: PaymentActivityResponse,
  trendWindowMonths: number = DEFAULT_TREND_WINDOW_MONTHS,
): RevenueDefaults {
  const { monthly_revenue } = activity;
  const currentMonth = monthly_revenue.find((entry) => entry.months_ago === 0);
  const monthlyRevenue = (currentMonth?.revenue ?? 0) / REVENUE_SCALE;
  const trend = computeRevenueGrowthTrend(monthly_revenue, trendWindowMonths);

  return {
    monthlyRevenue: Math.round(monthlyRevenue),
    growthRatePercent: trend.monthlyPercent,
    monthsOfHistory: trend.monthsOfHistory,
  };
}

/** A historical (actual revenue, estimated cash) point on the runway chart. */
export type HistoricalCashPoint = {
  /** Months from now; 0 is today, negative values are in the past. */
  month: number;
  /** Estimated cash balance at this point, in dollars. */
  cash: number;
  /** Actual revenue during the trailing 30-day window ending at this point, in dollars. */
  revenue: number;
};

/**
 * Estimate past cash balances by working backwards from the current cash
 * balance: each trailing 30-day bucket's actual revenue plus the current
 * burn rate tells us how much cash must have been on hand one "month"
 * earlier. Returns points ascending by month, ending at `{ month: 0 }`.
 */
export function computeHistoricalCashPoints(
  cash: number,
  burn: number,
  monthlyRevenue: MonthlyRevenue[],
): HistoricalCashPoint[] {
  const sorted = [...monthlyRevenue].sort((a, b) => a.months_ago - b.months_ago);
  const points: HistoricalCashPoint[] = [
    { month: 0, cash, revenue: (sorted[0]?.revenue ?? 0) / REVENUE_SCALE },
  ];
  let currentCash = cash;
  for (const entry of sorted) {
    const entryRevenue = entry.revenue / REVENUE_SCALE;
    const priorCash = currentCash + (burn - entryRevenue);
    points.push({
      month: -(entry.months_ago + 1),
      cash: priorCash,
      revenue: entryRevenue,
    });
    currentCash = priorCash;
  }
  return points.sort((a, b) => a.month - b.month);
}

export type RunwayInputs = {
  /** Current cash balance in dollars. */
  cash: number;
  /** Monthly revenue in dollars (month 0). */
  revenue: number;
  /** Monthly gross burn (total expenses) in dollars. */
  burn: number;
  /** Monthly revenue growth rate as a fraction (e.g. 0.08 for 8%/month). */
  growthRate: number;
};

export type ProjectionPoint = {
  /** Months from now (0 = today). */
  month: number;
  /** Projected cash balance at the end of this month, in dollars. */
  cash: number;
  /** Projected revenue during this month, in dollars. */
  revenue: number;
  /** Net burn during this month (expenses minus revenue), in dollars. */
  netBurn: number;
};

export type RunwayProjection = {
  points: ProjectionPoint[];
  /** Last month plotted (inclusive). */
  horizon: number;
  /** Net burn for month 0 (burn - revenue), in dollars. */
  initialNetBurn: number;
  /** Fractional month where cash first crosses zero, or `null` if it never does. */
  cashZeroMonth: number | null;
  /** First month where monthly revenue covers burn, or `null` if it never does. */
  breakEvenMonth: number | null;
  /** Fractional month where cash recovers above zero after dipping, or `null`. */
  recoveryMonth: number | null;
};

/**
 * Model the cash balance month by month, compounding revenue growth. Returns
 * the per-month curve plus the key crossover events used to annotate the chart.
 *
 * The plotted horizon extends to 12 months past profitability when cash never
 * runs out, or through the recovery point when cash dips below zero and climbs
 * back, so the chart always shows the full story.
 */
export function computeRunwayProjection(inputs: RunwayInputs): RunwayProjection {
  const { cash, revenue, burn, growthRate } = inputs;
  const initialNetBurn = burn - revenue;

  const points: ProjectionPoint[] = [{ month: 0, cash, revenue, netBurn: initialNetBurn }];

  let cashZeroMonth: number | null = cash <= 0 ? 0 : null;
  let breakEvenMonth: number | null = revenue >= burn ? 0 : null;
  let recoveryMonth: number | null = null;
  let previousCash = cash;

  for (let month = 1; month <= MAX_MONTHS; month += 1) {
    const monthRevenue = revenue * (1 + growthRate) ** month;
    const netBurn = burn - monthRevenue;
    const monthCash = previousCash - netBurn;
    points.push({ month, cash: monthCash, revenue: monthRevenue, netBurn });

    if (cashZeroMonth === null && previousCash > 0 && monthCash <= 0) {
      cashZeroMonth = month - 1 + previousCash / (previousCash - monthCash);
    }
    if (breakEvenMonth === null && monthRevenue >= burn) {
      breakEvenMonth = month;
    }
    if (cashZeroMonth !== null && recoveryMonth === null && previousCash <= 0 && monthCash > 0) {
      recoveryMonth = month - 1 + -previousCash / (monthCash - previousCash);
    }

    previousCash = monthCash;

    if (month >= 12) {
      if (breakEvenMonth !== null) {
        // Keep simulating past cash-out until the cash balance climbs back
        // above zero, so a dip-and-recover scenario is shown in full.
        const settledHorizon = month >= breakEvenMonth + 12;
        const settledRecovery = cashZeroMonth === null || recoveryMonth !== null;
        if (settledHorizon && settledRecovery) {
          break;
        }
      } else if (
        // Without positive growth from a non-zero base, revenue can never catch
        // up to burn, so cash will never recover; stop shortly after cash-out.
        (growthRate <= 0 || revenue <= 0) &&
        cashZeroMonth !== null &&
        month >= Math.ceil(cashZeroMonth) + 2
      ) {
        break;
      }
    }
  }

  let horizon = 12;
  if (breakEvenMonth !== null) {
    horizon = Math.max(horizon, breakEvenMonth + 12);
  }
  if (recoveryMonth !== null) {
    horizon = Math.max(horizon, Math.ceil(recoveryMonth) + 3);
  }
  if (cashZeroMonth !== null) {
    horizon = Math.max(horizon, Math.ceil(cashZeroMonth) + 2);
  }
  horizon = Math.max(12, Math.min(horizon, MAX_MONTHS));

  return {
    points: points.slice(0, horizon + 1),
    horizon,
    initialNetBurn,
    cashZeroMonth,
    breakEvenMonth,
    recoveryMonth,
  };
}
