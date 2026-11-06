import type { PeriodMetrics } from "../../lib/dashboardApi";

/** Metrics drawn as a single exclusive line series (all use visitors axis color). */
export const LINE_METRIC_KEYS = [
  "visitors",
  "conversion_rate",
  "bounce_rate",
  "session_time",
  "revenue_per_visitor",
] as const satisfies readonly (keyof PeriodMetrics)[];

export type LineMetricKey = (typeof LINE_METRIC_KEYS)[number];

/** Metrics that map to stacked revenue columns (plus the revenue master toggle). */
export const BAR_METRIC_KEYS = [
  "mrr",
  "new_subscriptions_mrr",
  "new_revenue",
  "churn_mrr",
  "refunds_and_chargebacks",
] as const satisfies readonly (keyof PeriodMetrics)[];

export type BarMetricKey = (typeof BAR_METRIC_KEYS)[number];

export type ChartMetricKey = LineMetricKey | BarMetricKey | "total_revenue";

export const LINE_CHART_COLOR = "#2563eb";

/** Matches OverviewChart revenue / line series colors. */
export const METRIC_CHART_COLORS: Record<ChartMetricKey, string> = {
  total_revenue: "#059669",
  visitors: LINE_CHART_COLOR,
  conversion_rate: LINE_CHART_COLOR,
  bounce_rate: LINE_CHART_COLOR,
  session_time: LINE_CHART_COLOR,
  revenue_per_visitor: LINE_CHART_COLOR,
  mrr: "#059669",
  new_subscriptions_mrr: "#34d399",
  new_revenue: "#f59e0b",
  churn_mrr: "#dc2626",
  refunds_and_chargebacks: "#f87171",
};

export type ChartMetricVisibility = {
  /** Active line metric, or none when all line toggles are off. */
  lineMetric: LineMetricKey | null;
  /** Revenue master toggle — stacked bars hidden when false. */
  barEnabled: boolean;
  barSeries: Record<BarMetricKey, boolean>;
};

export const DEFAULT_BAR_SERIES: Record<BarMetricKey, boolean> = {
  mrr: true,
  new_subscriptions_mrr: true,
  new_revenue: true,
  churn_mrr: false,
  refunds_and_chargebacks: false,
};

export function createDefaultChartVisibility(): ChartMetricVisibility {
  return {
    lineMetric: "visitors",
    barEnabled: true,
    barSeries: { ...DEFAULT_BAR_SERIES },
  };
}

export function isLineMetricActive(visibility: ChartMetricVisibility, key: LineMetricKey): boolean {
  return visibility.lineMetric === key;
}

export function isBarSeriesVisible(visibility: ChartMetricVisibility, key: BarMetricKey): boolean {
  return visibility.barEnabled && visibility.barSeries[key];
}

export function isMetricChartActive(
  visibility: ChartMetricVisibility,
  key: ChartMetricKey,
): boolean {
  if (key === "total_revenue") {
    return visibility.barEnabled;
  }
  if ((LINE_METRIC_KEYS as readonly string[]).includes(key)) {
    return isLineMetricActive(visibility, key as LineMetricKey);
  }
  return isBarSeriesVisible(visibility, key as BarMetricKey);
}

export function toggleLineMetric(
  visibility: ChartMetricVisibility,
  key: LineMetricKey,
): ChartMetricVisibility {
  if (visibility.lineMetric === key) {
    return { ...visibility, lineMetric: null };
  }
  return { ...visibility, lineMetric: key };
}

export function toggleRevenueMaster(visibility: ChartMetricVisibility): ChartMetricVisibility {
  if (visibility.barEnabled) {
    return { ...visibility, barEnabled: false };
  }
  return {
    ...visibility,
    barEnabled: true,
    barSeries: { ...DEFAULT_BAR_SERIES },
  };
}

export function toggleBarSeries(
  visibility: ChartMetricVisibility,
  key: BarMetricKey,
): ChartMetricVisibility {
  // With revenue hidden every series reads as off, so a click shows just that one.
  if (!visibility.barEnabled) {
    return {
      ...visibility,
      barEnabled: true,
      barSeries: Object.fromEntries(
        BAR_METRIC_KEYS.map((barKey) => [barKey, barKey === key]),
      ) as Record<BarMetricKey, boolean>,
    };
  }
  return {
    ...visibility,
    barSeries: {
      ...visibility.barSeries,
      [key]: !visibility.barSeries[key],
    },
  };
}

export function toggleChartMetric(
  visibility: ChartMetricVisibility,
  key: ChartMetricKey,
): ChartMetricVisibility {
  if (key === "total_revenue") {
    return toggleRevenueMaster(visibility);
  }
  if ((LINE_METRIC_KEYS as readonly string[]).includes(key)) {
    return toggleLineMetric(visibility, key as LineMetricKey);
  }
  return toggleBarSeries(visibility, key as BarMetricKey);
}
