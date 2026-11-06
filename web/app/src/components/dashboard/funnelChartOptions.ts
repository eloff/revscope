import { formatPercentRatio, formatRevenueAmount } from "../../lib/dashboardApi";
import type { FunnelDataResponse, FunnelStepData } from "../../lib/funnelsApi";
import type { ApexOptions } from "./apexcharts";
import { hoverHighlightRevenueIcon } from "./icons.css";
import type { MetricHoverContent } from "./metricHoverDetails";
import { SOURCE_GROUP_COLORS } from "./revenueSourcesLogic";

const FUNNEL_STEP_HEIGHT = 54;
/** Smallest drawn step width as a fraction of the largest step's width. */
const FUNNEL_MIN_STEP_WIDTH_RATIO = 0.15;
const HIGHEST_DROPOFF_DESCRIPTION =
  "Highest overall drop-off rate in the funnel, focus on retention here to improve conversion rate.";
const PAYMENT_STEP_DESCRIPTION =
  "These are the visitors who completed the funnel and purchased your product.";
const FUNNEL_NEGATIVE_MARKER = "⚠";
const FUNNEL_PAYMENT_MARKER = "$";

/**
 * Compute relative drawing widths for funnel steps that keep the funnel shape
 * but stop the smallest steps from collapsing to nothing.
 *
 * Rather than clamping every small step to the same minimum (which flattens the
 * tail of the funnel), we apply a monotonic power compression to the true
 * visitor ratios. The widest step stays full width and the narrowest step lands
 * exactly on `FUNNEL_MIN_STEP_WIDTH_RATIO`. Because the same exponent is applied
 * to every step, larger drops between steps stay larger than smaller ones, so
 * the shape still approximates the real data. Compression only kicks in when the
 * true funnel would push the smallest step below the floor; otherwise the real
 * proportions are drawn untouched.
 */
function funnelDisplayWidths(visitors: number[]): number[] {
  const maxVisitors = Math.max(...visitors, 0);
  if (maxVisitors <= 0) {
    return visitors.map(() => 1);
  }
  const positives = visitors.filter((value) => value > 0);
  const minVisitors = positives.length ? Math.min(...positives) : maxVisitors;
  const minRatio = minVisitors / maxVisitors;
  // gamma < 1 lifts the tail; gamma === 1 leaves the true shape untouched.
  const gamma =
    minRatio > 0 && minRatio < FUNNEL_MIN_STEP_WIDTH_RATIO
      ? Math.log(FUNNEL_MIN_STEP_WIDTH_RATIO) / Math.log(minRatio)
      : 1;
  return visitors.map((value) => {
    if (value <= 0) {
      return FUNNEL_MIN_STEP_WIDTH_RATIO;
    }
    const ratio = value / maxVisitors;
    return Math.max(ratio ** gamma, FUNNEL_MIN_STEP_WIDTH_RATIO);
  });
}

export function sortFunnelSteps(steps: FunnelStepData[]): FunnelStepData[] {
  return [...steps].sort((a, b) => a.order - b.order);
}

export function stepDisplayName(step: FunnelStepData): string {
  const description = step.description?.trim();
  return description || step.name;
}

export function highestDropoffStepOrder(sortedSteps: FunnelStepData[]): number | undefined {
  let highestOrder: number | undefined;
  let highestDropoffRate: number | undefined;
  for (const step of sortedSteps) {
    if (step.dropoff_rate === undefined) {
      continue;
    }
    if (highestDropoffRate === undefined || step.dropoff_rate > highestDropoffRate) {
      highestDropoffRate = step.dropoff_rate;
      highestOrder = step.order;
    }
  }
  return highestOrder;
}

function funnelStepLabelPrefix(
  step: FunnelStepData,
  highestDropoffOrder: number | undefined,
): string {
  const markers: string[] = [];
  if (step.order === highestDropoffOrder) {
    markers.push(FUNNEL_NEGATIVE_MARKER);
  }
  if (step.step_type === "payment") {
    markers.push(FUNNEL_PAYMENT_MARKER);
  }
  return markers.length > 0 ? `${markers.join(" ")} ` : "";
}

export type FunnelChartHandlers = {
  onStepClick?: (index: number | null) => void;
  onStepHover?: (index: number | null) => void;
};

export function buildStepDetailContent(
  step: FunnelStepData,
  totalRevenue?: number,
  isHighestDropoff = false,
): MetricHoverContent {
  const stats: MetricHoverContent["stats"] = [];
  if (step.path) {
    stats.push({ label: "Page", value: step.path });
  }
  if (step.event_name) {
    stats.push({ label: "Event", value: step.event_name });
  }
  stats.push({
    label: "Visitors",
    value: step.visitors.toLocaleString(),
    changeRatio: step.visitors_change.ratio,
  });
  if (step.dropoff_rate !== undefined) {
    stats.push({
      label: "Drop-off to next step",
      value: formatPercentRatio(step.dropoff_rate, 1),
    });
  }
  if (step.revenue_per_visitor !== undefined) {
    stats.push({
      label: "Revenue per visitor",
      value: formatRevenueAmount(step.revenue_per_visitor),
    });
  }
  if (totalRevenue !== undefined) {
    stats.push({
      label: "Funnel revenue",
      value: formatRevenueAmount(totalRevenue),
    });
  }
  const highlightDescriptions: MetricHoverContent["highlightDescriptions"] = [];
  if (isHighestDropoff) {
    highlightDescriptions.push({
      description: HIGHEST_DROPOFF_DESCRIPTION,
      tone: "negative",
    });
  }
  if (step.step_type === "payment") {
    highlightDescriptions.push({
      description: PAYMENT_STEP_DESCRIPTION,
      iconClass: hoverHighlightRevenueIcon,
    });
  }
  return {
    stats,
    ...(highlightDescriptions.length > 0 ? { highlightDescriptions } : {}),
  };
}

export function buildChartOptions(
  data: FunnelDataResponse,
  handlers?: FunnelChartHandlers,
): ApexOptions {
  const sortedSteps = sortFunnelSteps(data.steps);
  const highestDropoffOrder = highestDropoffStepOrder(sortedSteps);
  const displayWidths = funnelDisplayWidths(sortedSteps.map((step) => step.visitors));
  const chartHeight = Math.max(540, sortedSteps.length * FUNNEL_STEP_HEIGHT);
  return {
    chart: {
      type: "funnel",
      height: chartHeight,
      parentHeightOffset: 10,
      toolbar: { show: false },
      animations: { enabled: true },
      events: {
        dataPointSelection(_event, _chartContext, config) {
          const index = config?.dataPointIndex;
          handlers?.onStepClick?.(typeof index === "number" && index >= 0 ? index : null);
        },
        dataPointMouseEnter(_event, _chartContext, config) {
          const index = config?.dataPointIndex;
          handlers?.onStepHover?.(typeof index === "number" && index >= 0 ? index : null);
        },
        dataPointMouseLeave() {
          handlers?.onStepHover?.(null);
        },
        mouseLeave() {
          handlers?.onStepHover?.(null);
        },
      },
    },
    series: [
      {
        name: "Visitors",
        data: displayWidths,
      },
    ],
    colors: [SOURCE_GROUP_COLORS.search],
    states: {
      // Match hover (default lighten) so a pinned step looks the same as hover.
      active: {
        filter: {
          type: "lighten",
        },
      },
    },
    plotOptions: {
      bar: {
        horizontal: true,
        barHeight: "85%",
      },
      funnel: {
        shape: "trapezoid",
        lastShape: "flat",
      },
    },
    dataLabels: {
      enabled: true,
      formatter: (_value, chartOptions) => {
        const step = sortedSteps[chartOptions?.dataPointIndex ?? -1];
        if (!step) {
          return "";
        }
        const label = `${stepDisplayName(step)}: ${step.visitors.toLocaleString()}`;
        return `${funnelStepLabelPrefix(step, highestDropoffOrder)}${label}`;
      },
      style: {
        fontSize: "13px",
        fontWeight: 700,
        colors: ["#ffffff"],
      },
    },
    xaxis: {
      categories: sortedSteps.map((step) => stepDisplayName(step)),
      labels: { show: false },
      axisBorder: { show: false },
      axisTicks: { show: false },
    },
    yaxis: {
      labels: { show: false },
      axisBorder: { show: false },
      axisTicks: { show: false },
    },
    grid: {
      show: false,
      padding: {
        top: 0,
        right: 0,
        bottom: 0,
        left: 0,
      },
    },
    tooltip: { enabled: false },
    legend: { show: false },
  };
}
