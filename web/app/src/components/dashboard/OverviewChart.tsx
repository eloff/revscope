/// <reference types="solid-js" />

import { createEffect, createSignal, onCleanup, onMount, Show } from "solid-js";
import type { OverviewBucket, OverviewResponse } from "../../lib/dashboardApi";
import {
  formatRevenueAmountFixed,
  formatSessionDuration,
  visitorAttributedRevenue,
} from "../../lib/dashboardApi";
import { useDashboardPreferences } from "../../lib/dashboardPreferences";
import { formatBucketLabel, isCurrentDay, rangeEndIncludesCurrentDay } from "../../lib/dateRange";
import type { ImportedRange } from "../../lib/importedCoverageApi";
import ApexCharts, { type ApexOptions } from "./apexcharts";
import { chartMount } from "./chart.css";
import {
  type BarMetricKey,
  type ChartMetricVisibility,
  isBarSeriesVisible,
  LINE_CHART_COLOR,
  type LineMetricKey,
} from "./chartMetrics";
import { useDashboardState } from "./dashboardPeriodContext";
import { importedBucketSpans } from "./importedCoverage";
import { errorMessage, message } from "./messages.css";
import { niceLineAxisBounds, niceRevenueAxisBounds } from "./overviewChartAxis";

type OverviewChartProps = {
  data: OverviewResponse | undefined;
  loading: boolean;
  error: string;
  /** Fixed series in place of the viewer's tile toggles. */
  visibility?: ChartMetricVisibility;
  /** Zone the data was bucketed in, when it isn't the selected project's. */
  timeZone?: string;
  /** End of the period the data covers, when it isn't the selected range. */
  rangeEnd?: number;
  height?: number;
  /** Imports to shade, for the project the data is from. */
  importedRanges?: ImportedRange[];
};

const DEFAULT_CHART_HEIGHT = 420;
const IMPORTED_SHADE_COLOR = "#9ca3af";

const LINE_METRIC_LABELS: Record<LineMetricKey, string> = {
  visitors: "Visitors",
  conversion_rate: "Conversion",
  bounce_rate: "Bounce rate",
  session_time: "Session time",
  revenue_per_visitor: "Revenue / visitor",
};

/** Incomplete current day depresses the visitors line; revenue bars stay as-is. */
function visitorsForBucket(
  data: OverviewResponse,
  rangeEnd: number | undefined,
  timeZone: string,
  bucketStart: number,
  visitors: number,
): number | null {
  if (
    data.granularity !== "day" ||
    rangeEnd === undefined ||
    !rangeEndIncludesCurrentDay(timeZone, rangeEnd) ||
    !isCurrentDay(timeZone, bucketStart)
  ) {
    return visitors;
  }
  return null;
}

const REVENUE_SCALE = 1000;

/** ApexCharts defaults axis/tooltip text to 12px; scale up for dashboard legibility. */
const CHART_FONT_SIZE = 18;
/** Phone-width charts can't carry the larger text without labels colliding. */
const NARROW_CHART_FONT_SIZE = 11;
const NARROW_CHART_WIDTH = 560;

const MAX_X_AXIS_LABELS = 20;
const MAX_HOURLY_X_AXIS_LABELS = 10;

/** Labels rotated -35deg take up roughly this multiple of their font size horizontally. */
const X_AXIS_LABEL_WIDTH_RATIO = 3.2;

/** Each y-axis (value labels plus rotated title) eats about this much of the chart width. */
const Y_AXIS_GUTTER_RATIO = 6.5;

/** Bucket indices at which to show an x-axis label (fixed step, at most maxLabels). */
function sparseAxisLabelIndices(count: number, maxLabels: number): Set<number> {
  if (count <= 0) {
    return new Set();
  }
  if (count <= maxLabels) {
    return new Set(Array.from({ length: count }, (_, index) => index));
  }
  const step = Math.max(1, Math.ceil((count - 1) / (maxLabels - 1)));
  const indices = new Set<number>();
  for (let index = 0; index < count; index += step) {
    indices.add(index);
  }
  return indices;
}

function chartFontSize(chartWidth: number): number {
  return chartWidth > 0 && chartWidth < NARROW_CHART_WIDTH
    ? NARROW_CHART_FONT_SIZE
    : CHART_FONT_SIZE;
}

/** Plotting area left once the y-axes have taken their gutters. */
function plotWidth(chartWidth: number, fontSize: number, yAxisCount: number): number {
  return chartWidth - yAxisCount * fontSize * Y_AXIS_GUTTER_RATIO;
}

/** Label budget the plotting area can actually fit, capped by granularity. */
function maxXAxisLabelsForGranularity(
  granularity: OverviewResponse["granularity"],
  availableWidth: number,
  fontSize: number,
): number {
  const cap = granularity === "hour" ? MAX_HOURLY_X_AXIS_LABELS : MAX_X_AXIS_LABELS;
  if (availableWidth <= 0) {
    return cap;
  }
  const fits = Math.floor(availableWidth / (fontSize * X_AXIS_LABEL_WIDTH_RATIO));
  return Math.max(2, Math.min(cap, fits));
}

function chartAxisLabelStyle(fontSize: number) {
  return { fontSize: `${fontSize}px` };
}

function chartAxisTitleStyle(fontSize: number) {
  return { fontSize: `${fontSize}px`, fontWeight: 600 };
}

function formatRevenueAxis(value: number): string {
  const dollars = Math.round(value / REVENUE_SCALE);
  if (Math.abs(dollars) >= 1000) {
    return `$${(dollars / 1000).toFixed(1)}k`;
  }
  return `$${dollars}`;
}

function bucketChurnMrr(bucket: OverviewBucket): number {
  return -bucket.churn_mrr;
}

function bucketTotalRevenue(bucket: OverviewBucket): number {
  return (
    bucket.recurring_subscriptions +
    bucket.new_subscriptions +
    bucket.other_revenue +
    bucketChurnMrr(bucket) +
    bucket.refunds +
    bucket.chargebacks
  );
}

type BarSeriesPart = {
  key: BarMetricKey | "chargebacks_split";
  value: number;
};

function barSeriesParts(
  bucket: OverviewBucket,
  visibility: ChartMetricVisibility,
): BarSeriesPart[] {
  const parts: BarSeriesPart[] = [];
  if (isBarSeriesVisible(visibility, "mrr")) {
    parts.push({ key: "mrr", value: bucket.recurring_subscriptions });
  }
  if (isBarSeriesVisible(visibility, "new_subscriptions_mrr")) {
    parts.push({
      key: "new_subscriptions_mrr",
      value: bucket.new_subscriptions,
    });
  }
  if (isBarSeriesVisible(visibility, "new_revenue")) {
    parts.push({ key: "new_revenue", value: bucket.other_revenue });
  }
  if (isBarSeriesVisible(visibility, "churn_mrr")) {
    parts.push({ key: "churn_mrr", value: bucketChurnMrr(bucket) });
  }
  if (isBarSeriesVisible(visibility, "refunds_and_chargebacks")) {
    parts.push({ key: "refunds_and_chargebacks", value: bucket.refunds });
    parts.push({ key: "chargebacks_split", value: bucket.chargebacks });
  }
  return parts;
}

/** Stacked column min/max per bucket for visible bar series only. */
function bucketStackedRevenueExtents(
  bucket: OverviewBucket,
  visibility: ChartMetricVisibility,
): {
  min: number;
  max: number;
} {
  const parts = barSeriesParts(bucket, visibility).map((part) => part.value);
  if (parts.length === 0) {
    return { min: 0, max: 0 };
  }
  let cumulative = 0;
  let min = 0;
  let max = 0;
  for (const part of parts) {
    cumulative += part;
    min = Math.min(min, cumulative);
    max = Math.max(max, cumulative);
  }
  return { min, max };
}

function bucketNegativeRevenue(bucket: OverviewBucket, visibility: ChartMetricVisibility): number {
  let total = 0;
  if (isBarSeriesVisible(visibility, "churn_mrr")) {
    total += bucketChurnMrr(bucket);
  }
  if (isBarSeriesVisible(visibility, "refunds_and_chargebacks")) {
    total += bucket.refunds + bucket.chargebacks;
  }
  return total;
}

function bucketLineValue(bucket: OverviewBucket, lineMetric: LineMetricKey): number | null {
  if (bucket.visitors === 0) {
    return null;
  }
  const trackedVisitors = bucket.visitors - bucket.imported_visitors;
  switch (lineMetric) {
    case "visitors":
      return bucket.visitors;
    case "conversion_rate":
      return trackedVisitors > 0 ? bucket.converting_visitors / trackedVisitors : null;
    case "bounce_rate":
      return bucket.bounced_sessions / bucket.visitors;
    case "session_time":
      return bucket.session_time_total / bucket.visitors;
    case "revenue_per_visitor":
      return trackedVisitors > 0
        ? visitorAttributedRevenue({
            new_subscriptions: bucket.new_subscriptions,
            other: bucket.other_revenue,
          }) / trackedVisitors
        : null;
  }
}

function lineMetricMax(
  data: OverviewResponse,
  rangeEnd: number | undefined,
  timeZone: string,
  lineMetric: LineMetricKey,
): number {
  let max = 0;
  for (const bucket of data.buckets) {
    const raw = bucketLineValue(bucket, lineMetric);
    const value =
      lineMetric === "visitors"
        ? visitorsForBucket(data, rangeEnd, timeZone, bucket.start, raw ?? 0)
        : raw;
    if (value != null && !Number.isNaN(value)) {
      max = Math.max(max, value);
    }
  }
  return max;
}

function computeAxisBounds(
  data: OverviewResponse,
  rangeEnd: number | undefined,
  timeZone: string,
  visibility: ChartMetricVisibility,
): {
  lineMin: number;
  lineMax: number;
  revenueMin: number;
  revenueMax: number;
  revenueStep: number;
  hasLine: boolean;
  hasBars: boolean;
} {
  const hasLine = visibility.lineMetric !== null;
  const hasBars = visibility.barEnabled;

  let lineMax = 0;
  if (hasLine && visibility.lineMetric) {
    lineMax = lineMetricMax(data, rangeEnd, timeZone, visibility.lineMetric);
  }

  let revenueMin = 0;
  let revenueMax = 0;
  if (hasBars) {
    for (const bucket of data.buckets) {
      const { min, max } = bucketStackedRevenueExtents(bucket, visibility);
      const negativeRevenue = bucketNegativeRevenue(bucket, visibility);
      revenueMin = Math.min(revenueMin, min, negativeRevenue);
      revenueMax = Math.max(revenueMax, max);
    }
  }

  const revenue = hasBars
    ? niceRevenueAxisBounds(revenueMin, revenueMax)
    : {
        min: 0,
        max: 0,
        step: REVENUE_SCALE,
        stepDollars: 1,
      };

  const line = hasLine ? niceLineAxisBounds(lineMax) : { min: 0, max: 1, step: 1 };

  return {
    lineMin: line.min,
    lineMax: line.max,
    revenueMin: revenue.min,
    revenueMax: revenue.max,
    revenueStep: revenue.step,
    hasLine,
    hasBars,
  };
}

function formatRatePercent(value: number): string {
  const pct = value * 100;
  if (pct === 0) {
    return "0%";
  }
  if (Math.abs(pct) < 1) {
    return `${pct.toFixed(2)}%`;
  }
  if (Math.abs(pct) < 10) {
    return `${pct.toFixed(1)}%`;
  }
  return `${pct.toFixed(0)}%`;
}

function formatLineAxisValue(lineMetric: LineMetricKey, value: number): string {
  if (value < 0) {
    return "";
  }
  switch (lineMetric) {
    case "visitors":
      return Math.round(value).toString();
    case "conversion_rate":
    case "bounce_rate":
      return formatRatePercent(value);
    case "session_time":
      return formatSessionDuration(value);
    case "revenue_per_visitor":
      return formatRevenueAmountFixed(value, 2);
  }
}

function formatPaymentCount(total: number): string {
  if (total <= 0) {
    return "";
  }
  return total.toLocaleString();
}

function revenueSeriesPaymentCount(seriesName: string, bucket: OverviewBucket): string {
  switch (seriesName) {
    case "Recurring subscriptions":
      return formatPaymentCount(bucket.recurring_subscriptions_count);
    case "New subscriptions":
      return formatPaymentCount(bucket.new_subscriptions_count);
    case "New revenue":
      return formatPaymentCount(bucket.other_payments_count);
    default:
      return "";
  }
}

function formatLineTooltipValue(
  lineMetric: LineMetricKey,
  value: number | null | undefined,
): string {
  if (value == null || Number.isNaN(value)) {
    return "—";
  }
  switch (lineMetric) {
    case "visitors":
      return `${Math.round(value)} visitors`;
    case "conversion_rate":
      return `${(value * 100).toFixed(1)}% conversion`;
    case "bounce_rate":
      return `${(value * 100).toFixed(1)}% bounce rate`;
    case "session_time":
      return formatSessionDuration(value);
    case "revenue_per_visitor":
      return `${formatRevenueAmountFixed(value, 3)} / visitor`;
  }
}

function buildOverviewTooltipHtml(
  dataPointIndex: number,
  bucketLabels: string[],
  bucket: OverviewBucket | undefined,
  w: {
    globals: {
      series: (number | null)[][];
      seriesNames: string[];
      colors: string[];
    };
  },
  totals: number[],
  lineMetric: LineMetricKey | null,
  showRevenueTotal: boolean,
  fontSize: number,
): string {
  const title = bucketLabels[dataPointIndex] ?? "";
  const rows: string[] = [];

  if (lineMetric !== "visitors" && bucket) {
    rows.push(
      `<div class="apexcharts-tooltip-series-group" style="order: 0; display: flex;">` +
        `<span class="apexcharts-tooltip-marker" style="background-color: ${LINE_CHART_COLOR};"></span>` +
        `<div class="apexcharts-tooltip-text">` +
        `<div class="apexcharts-tooltip-y-group">` +
        `<span class="apexcharts-tooltip-text-y-label">Visitors: </span>` +
        `<span class="apexcharts-tooltip-text-y-value">${formatLineTooltipValue("visitors", bucket.visitors)}</span>` +
        `</div></div></div>`,
    );
  }

  rows.push(
    ...w.globals.seriesNames.map((name, seriesIndex) => {
      const value = w.globals.series[seriesIndex]?.[dataPointIndex];
      const isLineSeries = lineMetric !== null && seriesIndex === 0;
      let formatted = isLineSeries
        ? formatLineTooltipValue(lineMetric, value)
        : formatRevenueAxis(typeof value === "number" ? value : 0);
      if (!isLineSeries && bucket) {
        const paymentCount = revenueSeriesPaymentCount(name, bucket);
        if (paymentCount) {
          formatted = `${formatted} (${paymentCount})`;
        }
      }
      const color = w.globals.colors[seriesIndex] ?? "#888";
      return (
        `<div class="apexcharts-tooltip-series-group" style="order: ${seriesIndex + 1}; display: flex;">` +
        `<span class="apexcharts-tooltip-marker" style="background-color: ${color};"></span>` +
        `<div class="apexcharts-tooltip-text">` +
        `<div class="apexcharts-tooltip-y-group">` +
        `<span class="apexcharts-tooltip-text-y-label">${name}: </span>` +
        `<span class="apexcharts-tooltip-text-y-value">${formatted}</span>` +
        `</div></div></div>`
      );
    }),
  );

  if (showRevenueTotal) {
    const total = totals[dataPointIndex] ?? 0;
    rows.push(
      `<div class="apexcharts-tooltip-series-group" style="order: 99; display: flex; margin-top: 4px; padding-top: 4px; border-top: 1px solid rgba(0,0,0,0.1);">` +
        `<span class="apexcharts-tooltip-marker" style="visibility: hidden;"></span>` +
        `<div class="apexcharts-tooltip-text">` +
        `<div class="apexcharts-tooltip-y-group">` +
        `<span class="apexcharts-tooltip-text-y-label" style="font-weight: 600;">Total revenue: </span>` +
        `<span class="apexcharts-tooltip-text-y-value" style="font-weight: 600;">${formatRevenueAxis(total)}</span>` +
        `</div></div></div>`,
    );
  }

  return (
    `<div class="apexcharts-tooltip-title" style="font-size: ${fontSize}px;">${title}</div>` +
    rows.join("")
  );
}

function buildChartOptions(
  data: OverviewResponse,
  rangeEnd: number | undefined,
  timeZone: string,
  visibility: ChartMetricVisibility,
  chartWidth: number,
  height: number,
  importedRanges: ImportedRange[] = [],
): ApexOptions {
  const fontSize = chartFontSize(chartWidth);
  const labels = data.buckets.map((bucket) =>
    formatBucketLabel(bucket.start, data.granularity, timeZone),
  );
  const revenueTotals = data.buckets.map(bucketTotalRevenue);
  const axisBounds = computeAxisBounds(data, rangeEnd, timeZone, visibility);
  const lineMetric = visibility.lineMetric;
  const yAxisCount = (axisBounds.hasLine && lineMetric ? 1 : 0) + (axisBounds.hasBars ? 1 : 0);
  const visibleLabelIndices = sparseAxisLabelIndices(
    labels.length,
    maxXAxisLabelsForGranularity(
      data.granularity,
      plotWidth(chartWidth, fontSize, yAxisCount),
      fontSize,
    ),
  );
  // Apex finds an annotation's position by its category's text, so a hidden label
  // has to be invisible and still unique rather than empty.
  const axisLabels = labels.map((label, index) =>
    visibleLabelIndices.has(index) ? label : "\u200b".repeat(index + 1),
  );
  const importedAnnotations = importedBucketSpans(
    data.buckets.map((bucket) => bucket.start),
    importedRanges,
    timeZone,
  ).map(({ first, last }) => ({
    x: axisLabels[first],
    x2: first === last ? undefined : axisLabels[last],
    fillColor: IMPORTED_SHADE_COLOR,
    borderColor: IMPORTED_SHADE_COLOR,
    opacity: 0.15,
    label: {
      text: "Imported from Google Analytics",
      orientation: "horizontal",
      textAnchor: "start",
      borderColor: "transparent",
      style: { background: "transparent", color: "#6b7280", fontSize: `${fontSize}px` },
    },
  }));

  type OverviewSeries = {
    name: string;
    type: "line" | "column";
    data: (number | null)[];
  };

  const series: OverviewSeries[] = [];
  const colors: string[] = [];
  const strokeWidths: number[] = [];

  if (lineMetric) {
    const lineName = LINE_METRIC_LABELS[lineMetric];
    series.push({
      name: lineName,
      type: "line",
      data: data.buckets.map((bucket) => {
        const value = bucketLineValue(bucket, lineMetric);
        if (lineMetric === "visitors") {
          return visitorsForBucket(data, rangeEnd, timeZone, bucket.start, value ?? 0);
        }
        return value;
      }),
    });
    colors.push(LINE_CHART_COLOR);
    strokeWidths.push(3);
  }

  if (visibility.barEnabled) {
    if (isBarSeriesVisible(visibility, "mrr")) {
      series.push({
        name: "Recurring subscriptions",
        type: "column",
        data: data.buckets.map((b) => b.recurring_subscriptions),
      });
      colors.push("#059669");
      strokeWidths.push(0);
    }
    if (isBarSeriesVisible(visibility, "new_subscriptions_mrr")) {
      series.push({
        name: "New subscriptions",
        type: "column",
        data: data.buckets.map((b) => b.new_subscriptions),
      });
      colors.push("#34d399");
      strokeWidths.push(0);
    }
    if (isBarSeriesVisible(visibility, "new_revenue")) {
      series.push({
        name: "New revenue",
        type: "column",
        data: data.buckets.map((b) => b.other_revenue),
      });
      colors.push("#f59e0b");
      strokeWidths.push(0);
    }
    if (isBarSeriesVisible(visibility, "churn_mrr")) {
      series.push({
        name: "Churned MRR",
        type: "column",
        data: data.buckets.map((b) => bucketChurnMrr(b)),
      });
      colors.push("#dc2626");
      strokeWidths.push(0);
    }
    if (isBarSeriesVisible(visibility, "refunds_and_chargebacks")) {
      series.push({
        name: "Refunds",
        type: "column",
        data: data.buckets.map((b) => b.refunds),
      });
      colors.push("#f87171");
      strokeWidths.push(0);
      series.push({
        name: "Chargebacks",
        type: "column",
        data: data.buckets.map((b) => b.chargebacks),
      });
      colors.push("#9333ea");
      strokeWidths.push(0);
    }
  }

  const yaxis: ApexOptions["yaxis"] = [];
  if (axisBounds.hasLine && lineMetric) {
    yaxis.push({
      seriesName: LINE_METRIC_LABELS[lineMetric],
      min: axisBounds.lineMin,
      max: axisBounds.lineMax,
      title: {
        text: LINE_METRIC_LABELS[lineMetric],
        style: chartAxisTitleStyle(fontSize),
      },
      labels: {
        formatter: (value) => formatLineAxisValue(lineMetric, value),
        style: chartAxisLabelStyle(fontSize),
      },
    });
  }
  const revenueAxisSeriesName = series.find((s) => s.type === "column")?.name;
  if (axisBounds.hasBars && revenueAxisSeriesName) {
    yaxis.push({
      seriesName: revenueAxisSeriesName,
      opposite: true,
      min: axisBounds.revenueMin,
      max: axisBounds.revenueMax,
      stepSize: axisBounds.revenueStep,
      forceNiceScale: false,
      title: { text: "Revenue", style: chartAxisTitleStyle(fontSize) },
      labels: {
        formatter: (value) => formatRevenueAxis(value),
        style: chartAxisLabelStyle(fontSize),
      },
    });
  }

  return {
    chart: {
      type: "line",
      height,
      stacked: axisBounds.hasBars,
      toolbar: { show: false },
      fontFamily: "Inter, Roboto, 'Helvetica Neue', Arial, sans-serif",
    },
    stroke: {
      width: strokeWidths,
    },
    plotOptions: {
      bar: {
        columnWidth: "55%",
      },
    },
    dataLabels: { enabled: false },
    annotations: { xaxis: importedAnnotations },
    xaxis: {
      categories: axisLabels,
      labels: {
        rotate: -35,
        hideOverlappingLabels: false,
        style: chartAxisLabelStyle(fontSize),
      },
    },
    yaxis,
    colors,
    legend: {
      show: false,
    },
    tooltip: {
      shared: true,
      intersect: false,
      style: { fontSize: `${fontSize}px` },
      custom: ({ dataPointIndex, w }) =>
        buildOverviewTooltipHtml(
          dataPointIndex,
          labels,
          data.buckets[dataPointIndex],
          w,
          revenueTotals,
          lineMetric,
          visibility.barEnabled,
          fontSize,
        ),
    },
    series: series as ApexOptions["series"],
  };
}

export default function OverviewChart(props: OverviewChartProps) {
  const { range, timeZone: selectedTimeZone } = useDashboardState();
  const { chartVisibility } = useDashboardPreferences();
  const timeZone = () => props.timeZone ?? selectedTimeZone();
  const visibility = () => props.visibility ?? chartVisibility();
  const height = () => props.height ?? DEFAULT_CHART_HEIGHT;
  let chartElement: HTMLDivElement | undefined;
  let chart: ApexCharts | undefined;
  const [chartWidth, setChartWidth] = createSignal(0);

  onMount(() => {
    if (!chartElement) {
      return;
    }
    const mount = chartElement;
    chart = new ApexCharts(mount, {
      chart: { type: "line", height: height() },
      series: [],
      xaxis: { categories: [] },
    });
    chart.render();

    // Rounded so ordinary reflows don't churn through chart option rebuilds.
    const observer = new ResizeObserver(() => {
      setChartWidth(Math.round(mount.clientWidth / 20) * 20);
    });
    observer.observe(mount);
    onCleanup(() => observer.disconnect());
  });

  createEffect(() => {
    const width = chartWidth();
    if (!chart || props.loading || props.error || !props.data) {
      return;
    }
    chart.updateOptions(
      buildChartOptions(
        props.data,
        props.rangeEnd ?? range().end,
        timeZone(),
        visibility(),
        width,
        height(),
        props.importedRanges,
      ),
      true,
      true,
    );
  });

  onCleanup(() => {
    chart?.destroy();
    chart = undefined;
  });

  return (
    <div class={chartMount} style={{ "min-height": `${height()}px` }}>
      <Show when={props.loading}>
        <p class={message}>Loading chart…</p>
      </Show>
      <Show when={!props.loading && props.error}>
        <p class={errorMessage}>{props.error}</p>
      </Show>
      <div
        ref={chartElement}
        style={{
          display: props.loading || props.error ? "none" : "block",
        }}
      />
    </div>
  );
}
