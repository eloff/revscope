/// <reference types="solid-js" />

import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import type { PaymentActivityResponse } from "../../lib/paymentActivityApi";
import ApexCharts, { type ApexOptions } from "./apexcharts";
import { errorMessage, message } from "./messages.css";
import {
  adviceBody,
  adviceCard,
  adviceCritical,
  adviceHealthy,
  adviceInfo,
  adviceTitle,
  adviceWarning,
  chartCard,
  chartMount,
  chartTitle,
  emptyState,
  growthWindowSelect,
  inputAffix,
  inputBox,
  inputCard,
  inputControl,
  inputDefaultNote,
  inputField,
  inputGrid,
  inputHint,
  inputLabel,
  inputLabelIcon,
  inputLabelIconBurn,
  inputLabelIconPositive,
  inputLabelRow,
  resetButton,
  runwayIntro,
  runwayPanel,
  summaryBar,
  summaryCard,
  summaryDetail,
  summaryLabel,
  summaryValue,
  summaryValueNegative,
  summaryValuePositive,
} from "./RunwayCalculator.css";
import {
  computeHistoricalCashPoints,
  computeRunwayProjection,
  DEFAULT_TREND_WINDOW_MONTHS,
  type HistoricalCashPoint,
  MIN_TREND_WINDOW_MONTHS,
  maxTrendWindowMonths,
  type ProjectionPoint,
  type RunwayProjection,
  revenueDefaultsFromActivity,
  trendWindowOptions,
} from "./runwayProjection";

type RunwayCalculatorProps = {
  data?: PaymentActivityResponse;
  loading?: boolean;
  error?: string;
  /**
   * Public tool mode: no project data, no auth. All inputs are free-form
   * with starter defaults instead of revenue/growth derived from history.
   */
  standalone?: boolean;
};

const STANDALONE_DEFAULTS = {
  cash: "250,000",
  revenue: "10,000",
  burn: "40,000",
  growth: "10",
} as const;

const CHART_FONT_SIZE = "15px";

function parseNumber(value: string | null): number | null {
  if (value === null) {
    return null;
  }
  const cleaned = value.replace(/[^0-9.-]/g, "");
  if (cleaned === "" || cleaned === "-" || cleaned === ".") {
    return null;
  }
  const parsed = Number(cleaned);
  return Number.isFinite(parsed) ? parsed : null;
}

function formatDollars(value: number): string {
  const rounded = Math.round(value);
  const sign = rounded < 0 ? "-" : "";
  return `${sign}$${Math.abs(rounded).toLocaleString()}`;
}

function formatCompactDollars(value: number): string {
  const abs = Math.abs(value);
  const sign = value < 0 ? "-" : "";
  if (abs >= 1_000_000) {
    return `${sign}$${(abs / 1_000_000).toFixed(abs >= 10_000_000 ? 0 : 1)}M`;
  }
  if (abs >= 1_000) {
    return `${sign}$${(abs / 1_000).toFixed(abs >= 10_000 ? 0 : 1)}k`;
  }
  return `${sign}$${Math.round(abs)}`;
}

function formatMonthsValue(months: number): string {
  if (months >= 24) {
    const years = months / 12;
    return `${years.toFixed(1)} yrs`;
  }
  return `${months.toFixed(1)} mo`;
}

function formatGrowthPercent(percent: number): string {
  const abs = Math.abs(percent);
  if (abs >= 1000) {
    return `${Math.round(percent).toLocaleString()}%`;
  }
  if (abs >= 100) {
    return `${percent.toFixed(0)}%`;
  }
  return `${percent.toFixed(1)}%`;
}

/** Compound a monthly growth rate (as a percentage) to an annualized rate. */
function annualizedGrowthPercent(monthlyPercent: number): number {
  return ((1 + monthlyPercent / 100) ** 12 - 1) * 100;
}

type AdviceTone = "critical" | "warning" | "info" | "healthy";

type RunwayAdvice = {
  tone: AdviceTone;
  title: string;
  body: string;
};

/** Stage-appropriate fundraising/operating advice keyed off remaining runway. */
function runwayAdvice(runwayMonths: number | null): RunwayAdvice {
  if (runwayMonths === null) {
    return {
      tone: "healthy",
      title: "Default alive",
      body: "If you keep growing at this pace, your projected revenue overtakes your burn before the cash runs out. Keep pushing on growth and revisit these numbers whenever revenue or costs shift.",
    };
  }
  if (runwayMonths < 6) {
    return {
      tone: "critical",
      title: "Act now",
      body: "You have under six months of cash. Treat this as urgent: cut non-essential spend, line up bridge financing, and pull every lever you can on near-term revenue and reducing burn. Fundraising is your top priority.",
    };
  }
  if (runwayMonths < 12) {
    return {
      tone: "warning",
      title: "Begin fundraising",
      body: "With six to twelve months left, this is the time to raise in earnest. Tighten your burn, focus on the metrics investors want to see, and make sure fundraising is your top priority.",
    };
  }
  if (runwayMonths < 18) {
    return {
      tone: "info",
      title: "Start preparing",
      body: "Twelve to eighteen months gives you space to raise on your own terms. Start building investor relationships and assembling your story and numbers so you're in good shape to raise when you're ready.",
    };
  }
  return {
    tone: "healthy",
    title: "Plenty of runway",
    body: "You can concentrate on execution and growth rather than fundraising. Watch your burn and reassess once you drop back inside a twelve-month window.",
  };
}

const ADVICE_TONE_CLASS: Record<AdviceTone, string> = {
  critical: adviceCritical,
  warning: adviceWarning,
  info: adviceInfo,
  healthy: adviceHealthy,
};

type RevenueInputIconKind = "cash" | "revenue" | "burn" | "growth";

const REVENUE_INPUT_ICON_CLASS: Record<RevenueInputIconKind, string> = {
  cash: inputLabelIcon,
  revenue: inputLabelIconPositive,
  burn: inputLabelIconBurn,
  growth: inputLabelIconPositive,
};

function RevenueInputIcon(props: { kind: RevenueInputIconKind }) {
  const commonProps = {
    class: REVENUE_INPUT_ICON_CLASS[props.kind],
    viewBox: "0 0 24 24",
    fill: "none",
    stroke: "currentColor",
    "stroke-width": "2",
    "stroke-linecap": "round",
    "stroke-linejoin": "round",
    "aria-hidden": true,
  } as const;

  switch (props.kind) {
    case "cash":
      return (
        <svg {...commonProps} aria-hidden="true">
          <rect x="3" y="6" width="18" height="12" rx="2" />
          <path d="M3 10h18" />
          <path d="M7 14h3" />
          <circle cx="17" cy="14" r="1" />
        </svg>
      );
    case "revenue":
      return (
        <svg {...commonProps} aria-hidden="true">
          <circle cx="12" cy="12" r="8" />
          <path d="M12 7v10" />
          <path d="M15 9.5a3 3 0 0 0-3-1.5c-1.7 0-3 0.9-3 2s1.3 2 3 2 3 0.9 3 2-1.3 2-3 2a3 3 0 0 1-3-1.5" />
        </svg>
      );
    case "burn":
      return (
        <svg {...commonProps} aria-hidden="true">
          <path d="M12 22c3.3 0 6-2.5 6-6 0-2.5-1.3-4.6-3.8-6.5.2 1.7-.4 3.1-1.7 4.2.2-3.4-1.2-6.3-4.5-8.7.4 3.3-.8 5.4-2.4 7.1A5.8 5.8 0 0 0 4 16c0 3.5 2.7 6 6 6Z" />
          <path d="M12 22c1.7 0 3-1.3 3-3 0-1.2-.6-2.2-1.8-3.1 0 1-.4 1.8-1.2 2.4 0-1.8-.8-3.2-2.4-4.3.2 1.7-.4 2.8-1.2 3.6A3 3 0 0 0 9 19c0 1.7 1.3 3 3 3Z" />
        </svg>
      );
    case "growth":
      return (
        <svg {...commonProps} aria-hidden="true">
          <path d="M3 17 9 11l4 4 8-8" />
          <path d="M15 7h6v6" />
        </svg>
      );
  }
}

const chartAxisLabelStyle = { fontSize: CHART_FONT_SIZE };
const chartAxisTitleStyle = { fontSize: CHART_FONT_SIZE, fontWeight: 600 };

function buildTooltipHtml(
  point: ProjectionPoint | undefined,
  monthLabel: (month: number) => string,
): string {
  if (!point) {
    return "";
  }
  const burning = point.netBurn > 0;
  const netLabel = burning ? "Net burn" : "Net surplus";
  const netValue = formatDollars(Math.abs(point.netBurn));
  const netColor = burning ? "#dc2626" : "#059669";
  return (
    `<div class="apexcharts-tooltip-title" style="font-size: ${CHART_FONT_SIZE};">${monthLabel(
      point.month,
    )} · Month ${point.month}</div>` +
    `<div style="padding: 4px 8px; font-size: ${CHART_FONT_SIZE};">` +
    `<div><strong>Cash:</strong> ${formatDollars(point.cash)}</div>` +
    `<div><strong>Revenue:</strong> ${formatDollars(point.revenue)}/mo</div>` +
    `<div style="color: ${netColor};"><strong>${netLabel}:</strong> ${netValue}/mo</div>` +
    `</div>`
  );
}

/** Milestones at the same rounded month share one chart annotation. */
function milestonesOverlap(a: number, b: number): boolean {
  return Math.round(a) === Math.round(b);
}

function buildHistoricalTooltipHtml(
  point: HistoricalCashPoint | undefined,
  monthLabel: (month: number) => string,
): string {
  if (!point) {
    return "";
  }
  return (
    `<div class="apexcharts-tooltip-title" style="font-size: ${CHART_FONT_SIZE};">${monthLabel(
      point.month,
    )} · Month ${point.month}</div>` +
    `<div style="padding: 4px 8px; font-size: ${CHART_FONT_SIZE};">` +
    `<div><strong>Cash:</strong> ${formatDollars(point.cash)}</div>` +
    `<div><strong>Revenue:</strong> ${formatDollars(point.revenue)}/mo</div>` +
    `</div>`
  );
}

function buildChartOptions(
  projection: RunwayProjection,
  historicalPoints: HistoricalCashPoint[],
  monthLabel: (month: number) => string,
): ApexOptions {
  const { points, horizon, cashZeroMonth, breakEvenMonth, recoveryMonth } = projection;
  const xMin = historicalPoints.length > 0 ? historicalPoints[0].month : 0;

  const xaxisAnnotations: NonNullable<NonNullable<ApexOptions["annotations"]>["xaxis"]> = [];
  const pointAnnotations: NonNullable<NonNullable<ApexOptions["annotations"]>["points"]> = [];

  const labelStyleBase = {
    fontSize: "12px",
    fontWeight: 600,
    color: "#ffffff",
  };

  // Mark "today" (month 0) so it's clear where actual history ends and the
  // projection begins. Placed at the bottom so it never collides with the
  // stacked top-aligned labels below.
  xaxisAnnotations.push({
    x: 0,
    borderColor: "#6b7280",
    strokeDashArray: 0,
    label: {
      text: "Today",
      orientation: "horizontal",
      position: "bottom",
      style: { ...labelStyleBase, background: "#6b7280" },
    },
  });

  // When milestones land on the same month, show one label: profitable, then
  // cash zero, then back to positive.
  const showBreakEven = breakEvenMonth !== null;
  const showCashZero =
    cashZeroMonth !== null &&
    (breakEvenMonth === null || !milestonesOverlap(cashZeroMonth, breakEvenMonth));
  const showRecovery =
    recoveryMonth !== null &&
    (breakEvenMonth === null || !milestonesOverlap(recoveryMonth, breakEvenMonth)) &&
    (cashZeroMonth === null || !showCashZero || !milestonesOverlap(recoveryMonth, cashZeroMonth));

  let milestoneLabelOffset = 0;

  if (showCashZero && cashZeroMonth !== null) {
    xaxisAnnotations.push({
      x: cashZeroMonth,
      borderColor: "#dc2626",
      strokeDashArray: 5,
      label: {
        text: `Cash zero · ${monthLabel(cashZeroMonth)}`,
        orientation: "horizontal",
        position: "top",
        offsetY: milestoneLabelOffset,
        style: { ...labelStyleBase, background: "#dc2626" },
      },
    });
    pointAnnotations.push({
      x: cashZeroMonth,
      y: 0,
      marker: {
        size: 6,
        fillColor: "#dc2626",
        strokeColor: "#ffffff",
        strokeWidth: 2,
      },
    });
    milestoneLabelOffset += 22;
  }

  if (showBreakEven && breakEvenMonth !== null) {
    const breakEvenPoint = points[breakEvenMonth];
    xaxisAnnotations.push({
      x: breakEvenMonth,
      borderColor: "#059669",
      strokeDashArray: 5,
      label: {
        text:
          breakEvenMonth === 0 ? "Profitable now" : `Profitable · ${monthLabel(breakEvenMonth)}`,
        orientation: "horizontal",
        position: "top",
        offsetY: milestoneLabelOffset,
        style: { ...labelStyleBase, background: "#059669" },
      },
    });
    if (breakEvenPoint) {
      pointAnnotations.push({
        x: breakEvenMonth,
        y: Math.round(breakEvenPoint.cash),
        marker: {
          size: 6,
          fillColor: "#059669",
          strokeColor: "#ffffff",
          strokeWidth: 2,
        },
      });
    }
    milestoneLabelOffset += 22;
  }

  if (showRecovery && recoveryMonth !== null) {
    xaxisAnnotations.push({
      x: recoveryMonth,
      borderColor: "#2563eb",
      strokeDashArray: 5,
      label: {
        text: `Back to positive · ${monthLabel(recoveryMonth)}`,
        orientation: "horizontal",
        position: "top",
        offsetY: milestoneLabelOffset,
        style: { ...labelStyleBase, background: "#2563eb" },
      },
    });
    pointAnnotations.push({
      x: recoveryMonth,
      y: 0,
      marker: {
        size: 6,
        fillColor: "#2563eb",
        strokeColor: "#ffffff",
        strokeWidth: 2,
      },
    });
  }

  return {
    chart: {
      type: "area",
      height: 420,
      toolbar: { show: false },
      zoom: { enabled: false },
      fontFamily: "Inter, Roboto, 'Helvetica Neue', Arial, sans-serif",
      animations: { enabled: false },
    },
    series: [
      {
        name: "Estimated past cash balance",
        data: historicalPoints.map((point) => ({
          x: point.month,
          y: Math.round(point.cash),
        })),
      },
      {
        name: "Cash balance",
        data: points.map((point) => ({
          x: point.month,
          y: Math.round(point.cash),
        })),
      },
    ],
    colors: ["#9ca3af", "#059669"],
    stroke: { curve: "monotoneCubic", width: [2, 3], dashArray: [6, 0] },
    fill: {
      type: ["solid", "gradient"],
      opacity: [0, 1],
      gradient: {
        shadeIntensity: 0.3,
        opacityFrom: 0.35,
        opacityTo: 0.05,
        stops: [0, 100],
      },
    },
    dataLabels: { enabled: false },
    markers: { size: 0 },
    grid: { borderColor: "#eef2f5" },
    xaxis: {
      type: "numeric",
      min: xMin,
      max: horizon,
      tickAmount: Math.min(horizon - xMin, 16),
      title: { text: "Month", style: chartAxisTitleStyle },
      labels: {
        rotate: -35,
        formatter: (value) => monthLabel(Number(value)),
        style: chartAxisLabelStyle,
      },
    },
    yaxis: {
      title: { text: "Cash balance", style: chartAxisTitleStyle },
      labels: {
        formatter: (value) => formatCompactDollars(value),
        style: chartAxisLabelStyle,
      },
    },
    annotations: {
      yaxis: [
        {
          y: 0,
          borderColor: "#9ca3af",
          strokeDashArray: 4,
        },
      ],
      xaxis: xaxisAnnotations,
      points: pointAnnotations,
    },
    tooltip: {
      shared: false,
      intersect: false,
      custom: ({ seriesIndex, dataPointIndex }) =>
        seriesIndex === 0
          ? buildHistoricalTooltipHtml(historicalPoints[dataPointIndex], monthLabel)
          : buildTooltipHtml(points[dataPointIndex], monthLabel),
    },
    legend: { show: false },
  };
}

export default function RunwayCalculator(props: RunwayCalculatorProps) {
  const standalone = () => props.standalone === true;
  const activity = () => (standalone() ? undefined : props.data);

  const [cashInput, setCashInput] = createSignal<string>(
    standalone() ? STANDALONE_DEFAULTS.cash : "250,000",
  );
  const [burnInput, setBurnInput] = createSignal<string>(
    standalone() ? STANDALONE_DEFAULTS.burn : "40,000",
  );
  // `null` means "use the value derived from recent revenue history".
  // In standalone mode there is no history, so start with editable defaults.
  const [revenueOverride, setRevenueOverride] = createSignal<string | null>(
    standalone() ? STANDALONE_DEFAULTS.revenue : null,
  );
  const [growthOverride, setGrowthOverride] = createSignal<string | null>(
    standalone() ? STANDALONE_DEFAULTS.growth : null,
  );
  const [growthWindowMonths, setGrowthWindowMonths] = createSignal(DEFAULT_TREND_WINDOW_MONTHS);
  let growthWindowInitialized = false;

  const now = new Date();

  const monthLabel = (month: number): string => {
    const date = new Date(now.getFullYear(), now.getMonth() + Math.round(month));
    return date.toLocaleDateString(undefined, {
      month: "short",
      year: "numeric",
    });
  };

  // Keep the default six-month window selectable even when sparse history
  // would otherwise cap the fitted trend to fewer revenue-bearing buckets.
  const growthWindowCap = createMemo(() =>
    Math.max(DEFAULT_TREND_WINDOW_MONTHS, maxTrendWindowMonths(activity()?.monthly_revenue ?? [])),
  );

  // Derive the dropdown options from the (memoized) cap so the option nodes
  // only rebuild when the cap actually changes, rather than on every render.
  const growthWindowOptions = createMemo(() => trendWindowOptions(growthWindowCap()));

  // Keep the dropdown selection and fitted growth rate on the same window.
  // On first activity load, default to six months (clamped to available history).
  createEffect(() => {
    const cap = growthWindowCap();
    if (!activity()) {
      return;
    }
    setGrowthWindowMonths((current) => {
      if (!growthWindowInitialized) {
        growthWindowInitialized = true;
        return Math.min(DEFAULT_TREND_WINDOW_MONTHS, cap);
      }
      return Math.max(MIN_TREND_WINDOW_MONTHS, Math.min(current, cap));
    });
  });

  const defaults = createMemo(() => {
    const response = activity();
    return response ? revenueDefaultsFromActivity(response, growthWindowMonths()) : undefined;
  });

  const defaultRevenue = createMemo(() => defaults()?.monthlyRevenue ?? null);
  const defaultGrowthPercent = createMemo(() => defaults()?.growthRatePercent ?? null);
  const revenueValue = createMemo(() => {
    const override = revenueOverride();
    if (override !== null) {
      return override;
    }
    const fallback = defaultRevenue();
    return fallback === null ? "" : String(fallback);
  });

  const growthValue = createMemo(() => {
    const override = growthOverride();
    if (override !== null) {
      return override;
    }
    const fallback = defaultGrowthPercent();
    return fallback === null ? "" : fallback.toFixed(1);
  });

  const annualizedGrowthLabel = createMemo(() => {
    const monthlyPercent = parseNumber(growthValue());
    if (monthlyPercent === null) {
      return null;
    }
    return formatGrowthPercent(annualizedGrowthPercent(monthlyPercent));
  });

  const growthTrendHint = createMemo(() => {
    const annualized = annualizedGrowthLabel();
    if (annualized === null) {
      return null;
    }
    return `Trend over last ${growthWindowMonths()} months (${annualized} annualized)`;
  });

  const usingGrowthDefault = createMemo(
    () => growthOverride() === null && defaultGrowthPercent() !== null,
  );

  const growthHint = createMemo(() => {
    if (usingGrowthDefault()) {
      return growthTrendHint() ?? "Not enough revenue history yet to estimate a trend";
    }
    if (growthOverride() === null) {
      return "Not enough revenue history yet to estimate a trend";
    }
    const annualized = annualizedGrowthLabel();
    return annualized === null ? "Month-over-month growth rate" : `${annualized} annualized`;
  });

  const effectiveCash = createMemo(() => parseNumber(cashInput()));
  const effectiveBurn = createMemo(() => {
    const parsed = parseNumber(burnInput());
    if (parsed === null) {
      return null;
    }
    return Math.max(0, parsed);
  });
  const effectiveRevenue = createMemo(() => parseNumber(revenueValue()) ?? 0);
  const effectiveGrowth = createMemo(() => (parseNumber(growthValue()) ?? 0) / 100);

  const projection = createMemo<RunwayProjection | null>(() => {
    const cash = effectiveCash();
    const burn = effectiveBurn();
    if (cash === null || burn === null) {
      return null;
    }
    return computeRunwayProjection({
      cash,
      burn,
      revenue: effectiveRevenue(),
      growthRate: effectiveGrowth(),
    });
  });

  const hasOverrides = createMemo(
    () => !standalone() && (revenueOverride() !== null || growthOverride() !== null),
  );

  // Estimated past cash balance, walked backwards from the current balance
  // using each trailing 30-day bucket's actual revenue and the entered burn
  // rate, so the chart can show where cash has been leading up to today.
  const historicalPoints = createMemo<HistoricalCashPoint[]>(() => {
    const cash = effectiveCash();
    const burn = effectiveBurn();
    const response = activity();
    if (cash === null || burn === null || !response) {
      return [];
    }
    return computeHistoricalCashPoints(cash, burn, response.monthly_revenue);
  });

  let chartElement: HTMLDivElement | undefined;
  let chart: ApexCharts | undefined;

  onMount(() => {
    if (!chartElement) {
      return;
    }
    chart = new ApexCharts(chartElement, {
      chart: { type: "area", height: 420 },
      series: [],
      xaxis: { categories: [] },
    });
    void chart.render();
  });

  createEffect(() => {
    const current = projection();
    if (!chart || !current) {
      return;
    }
    void chart.updateOptions(
      buildChartOptions(current, historicalPoints(), monthLabel),
      true,
      true,
    );
  });

  onCleanup(() => {
    chart?.destroy();
    chart = undefined;
  });

  return (
    <div class={runwayPanel}>
      <Show when={!standalone()}>
        <h2 class={chartTitle}>Growth projection</h2>
        <p class={runwayIntro}>
          See how many months of cash you have left. Add your financials to calculate your runway
          and test different scenarios.
        </p>
      </Show>

      <div class={inputCard}>
        <div class={inputGrid}>
          <label class={inputField}>
            <span class={inputLabel}>
              <RevenueInputIcon kind="cash" />
              Current cash balance
            </span>
            <span class={inputControl}>
              <span class={inputAffix}>$</span>
              <input
                class={inputBox}
                type="text"
                inputMode="decimal"
                placeholder="250,000"
                value={cashInput()}
                onInput={(event) => setCashInput(event.currentTarget.value)}
              />
            </span>
            <span class={inputHint}>Total cash in the bank right now</span>
          </label>

          <label class={inputField}>
            <span class={inputLabel}>
              <RevenueInputIcon kind="revenue" />
              Monthly revenue
            </span>
            <span class={inputControl}>
              <span class={inputAffix}>$</span>
              <input
                class={inputBox}
                type="text"
                inputMode="decimal"
                placeholder="0"
                value={revenueValue()}
                onInput={(event) => setRevenueOverride(event.currentTarget.value)}
              />
            </span>
            <Show
              when={revenueOverride() === null && defaultRevenue() !== null}
              fallback={<span class={inputHint}>Gross revenue per month</span>}
            >
              <span class={inputDefaultNote}>From last 30 days</span>
            </Show>
          </label>

          <label class={inputField}>
            <span class={inputLabel}>
              <RevenueInputIcon kind="burn" />
              Monthly burn rate
            </span>
            <span class={inputControl}>
              <span class={inputAffix}>$</span>
              <input
                class={inputBox}
                type="text"
                inputMode="decimal"
                placeholder="40,000"
                value={burnInput()}
                onInput={(event) => setBurnInput(event.currentTarget.value)}
              />
            </span>
            <span class={inputHint}>Total monthly expenses</span>
          </label>

          <label class={inputField}>
            <span class={inputLabelRow}>
              <span class={inputLabel}>
                <RevenueInputIcon kind="growth" />
                Monthly revenue growth
              </span>
              <Show when={!standalone()}>
                <select
                  class={growthWindowSelect}
                  value={growthOverride() !== null ? "custom" : String(growthWindowMonths())}
                  onChange={(event) => {
                    const value = event.currentTarget.value;
                    if (value === "custom") {
                      return;
                    }
                    setGrowthWindowMonths(Number(value));
                    setGrowthOverride(null);
                  }}
                >
                  <For each={growthWindowOptions()}>
                    {(option) => (
                      <option
                        value={String(option.months)}
                        selected={
                          growthOverride() === null && option.months === growthWindowMonths()
                        }
                      >
                        {option.label}
                      </option>
                    )}
                  </For>
                  <option value="custom" disabled selected={growthOverride() !== null}>
                    Custom
                  </option>
                </select>
              </Show>
            </span>
            <span class={inputControl}>
              <input
                class={inputBox}
                type="text"
                inputMode="decimal"
                placeholder="0"
                value={growthValue()}
                onInput={(event) => setGrowthOverride(event.currentTarget.value)}
              />
              <span class={inputAffix}>%</span>
            </span>
            <span class={usingGrowthDefault() ? inputDefaultNote : inputHint}>{growthHint()}</span>
          </label>
        </div>

        <Show when={hasOverrides()}>
          <button
            type="button"
            class={resetButton}
            onClick={() => {
              setRevenueOverride(null);
              setGrowthOverride(null);
            }}
          >
            Reset revenue &amp; growth to defaults
          </button>
        </Show>
      </div>

      <Show when={!standalone() && props.loading}>
        <p class={message}>Loading recent revenue...</p>
      </Show>
      <Show when={!standalone() && props.error}>
        <p class={errorMessage}>{props.error}</p>
      </Show>

      <Show when={projection()}>
        {(current) => (
          <>
            <RunwaySummary projection={current()} />
            <RunwayAdviceCard advice={runwayAdvice(current().cashZeroMonth)} />
          </>
        )}
      </Show>

      <div class={chartCard}>
        <h3 class={chartTitle}>Cash balance projection</h3>
        <Show when={!projection()}>
          <p class={emptyState}>
            Enter your current cash balance and monthly burn rate to project your runway.
          </p>
        </Show>
        <div
          ref={chartElement}
          class={chartMount}
          style={{ display: projection() ? "block" : "none" }}
        />
      </div>
    </div>
  );
}

function RunwayAdviceCard(props: { advice: RunwayAdvice }) {
  return (
    <div class={`${adviceCard} ${ADVICE_TONE_CLASS[props.advice.tone]}`}>
      <p class={adviceTitle}>{props.advice.title}</p>
      <p class={adviceBody}>{props.advice.body}</p>
    </div>
  );
}

function RunwaySummary(props: { projection: RunwayProjection }) {
  const now = new Date();
  const monthLabel = (month: number): string => {
    const date = new Date(now.getFullYear(), now.getMonth() + Math.round(month));
    return date.toLocaleDateString(undefined, {
      month: "short",
      year: "numeric",
    });
  };

  const runway = () => {
    const zero = props.projection.cashZeroMonth;
    return zero === null ? null : zero;
  };

  return (
    <div class={summaryBar}>
      <div class={summaryCard}>
        <span class={summaryLabel}>Runway</span>
        {(() => {
          const months = runway();
          return months !== null ? (
            <span class={summaryValue}>{formatMonthsValue(months)}</span>
          ) : (
            <span class={`${summaryValue} ${summaryValuePositive}`}>∞</span>
          );
        })()}
        <span class={summaryDetail}>
          {runway() !== null ? "Until cash runs out" : "Cash never reaches zero"}
        </span>
      </div>

      <div class={summaryCard}>
        <span class={summaryLabel}>Monthly net burn</span>
        <span
          class={`${summaryValue} ${
            props.projection.initialNetBurn > 0 ? summaryValueNegative : summaryValuePositive
          }`}
        >
          {props.projection.initialNetBurn > 0
            ? formatDollars(props.projection.initialNetBurn)
            : `+${formatDollars(-props.projection.initialNetBurn)}`}
        </span>
        <span class={summaryDetail}>
          {props.projection.initialNetBurn > 0 ? "Expenses minus revenue" : "Monthly cash surplus"}
        </span>
      </div>

      <div class={summaryCard}>
        <span class={summaryLabel}>Cash Zero Date</span>
        <span class={summaryValue}>
          {props.projection.cashZeroMonth !== null
            ? monthLabel(props.projection.cashZeroMonth)
            : "—"}
        </span>
        <span class={summaryDetail}>When cash balance reaches zero</span>
      </div>

      <div class={summaryCard}>
        <span class={summaryLabel}>Break-even</span>
        <span class={summaryValue}>
          {props.projection.breakEvenMonth === null
            ? "—"
            : props.projection.breakEvenMonth === 0
              ? "Now"
              : monthLabel(props.projection.breakEvenMonth)}
        </span>
        <span class={summaryDetail}>When revenue covers expenses</span>
      </div>
    </div>
  );
}
