import type {
  BucketTotals,
  MetricChange,
  OverviewResponse,
  PeriodMetrics,
} from "../../lib/dashboardApi";
import {
  formatPercentRatio,
  formatRevenueAmount,
  formatSessionDuration,
  sumOverviewBuckets,
  visitorAttributedRevenue,
} from "../../lib/dashboardApi";
import type { SourceGroup } from "../../lib/revenueSourcesApi";
import { formatWilsonScoreInterval } from "./conversionConfidence";
import { formatAvgTimeToPurchase } from "./conversionsLogic";

function formatAvgRevenuePerConversion(newRevenue: number, convertingVisitors: number): string {
  if (convertingVisitors <= 0) {
    return "—";
  }
  return formatRevenueAmount(newRevenue / convertingVisitors);
}

export type HoverStatLine = {
  label: string;
  value: string;
  /** When set and not null, shown after the value with period-over-period styling. */
  changeRatio?: number | null;
  /** When true, `changeRatio` is rendered in the value column instead of the change column. */
  changeInValueColumn?: boolean;
  /** When lower values are better (e.g. refunds), inverts positive/negative styling. */
  higherIsBetter?: boolean;
};

export type HoverLandingPath = {
  id: number;
  path: string;
  visitors: number;
  /** Share of the parent total (0..1), shown as a percent when set. */
  share?: number;
};

export type MetricHoverContent = {
  stats: HoverStatLine[];
  /** Landing page paths with visitor counts, shown after stats. */
  landingPaths?: HoverLandingPath[];
  /** Section header for `landingPaths`. Defaults to Top landing paths. */
  landingPathsTitle?: string;
  /** Column header for `landingPaths` counts. Defaults to Visitors. */
  landingPathsCountLabel?: string;
  /** Column header for `landingPaths` shares. Defaults to %. */
  landingPathsShareLabel?: string;
  /** Top referrers for a campaign, shown after landing paths. */
  referrers?: HoverLandingPath[];
  /** Section header for `referrers`. Defaults to Top referrers. */
  referrersTitle?: string;
  description?: string;
  /** One callout line per highlight (best metric, worst rate, or funnel note). */
  highlightDescriptions?: HoverHighlight[];
  /** Optional note shown below the description and above the period comparison note. */
  footnote?: string;
};

export type HighlightTone = "positive" | "negative";

export type HighlightMetric =
  | "revenue"
  | "revenuePerVisitor"
  | "conversion"
  | "visitorsUplift"
  | "conversionUplift"
  | "exitUplift"
  | "bounceUplift"
  | "sessionUplift"
  | "highestExitRate"
  | "highestBounceRate"
  | "conversionLeak"
  | "highestCannibalization"
  | "highestLostClicks";

export type HoverHighlight =
  | {
      metric: HighlightMetric;
      /** When set, shown as a badge after "in" (revenue source groups). */
      sourceGroup?: SourceGroup;
    }
  | {
      description: string;
      /** CSS class for an icon shown before the description. Defaults to ★. */
      iconClass?: string;
      /** Defaults to positive (★). Negative uses the warning icon. */
      tone?: HighlightTone;
    };

const HIGHLIGHT_METRIC_LABELS: Record<HighlightMetric, string> = {
  revenue: "Best revenue",
  revenuePerVisitor: "Best revenue / visitor",
  conversion: "Best conversion rate",
  visitorsUplift: "Biggest gain from more visitors. Focus here.",
  conversionUplift: "Best conversion uplift (+25%). Focus here.",
  exitUplift: "Biggest gain from fewer exits. Focus here.",
  bounceUplift: "Biggest gain from fewer bounces. Focus here.",
  sessionUplift: "Biggest gain from longer sessions. Focus here.",
  highestExitRate: "Highest exit rate. Focus here.",
  highestBounceRate: "Highest bounce rate. Focus here.",
  conversionLeak: "Most conversions lost vs the group average. Focus here.",
  highestCannibalization: "Splitting traffic is reducing rankings and revenue. Focus here.",
  highestLostClicks: "Below average click through is costing revenue. Focus here.",
};

const HIGHLIGHT_METRIC_TONE: Record<HighlightMetric, HighlightTone> = {
  revenue: "positive",
  revenuePerVisitor: "positive",
  conversion: "positive",
  visitorsUplift: "positive",
  conversionUplift: "positive",
  exitUplift: "positive",
  bounceUplift: "positive",
  sessionUplift: "positive",
  highestExitRate: "negative",
  highestBounceRate: "negative",
  conversionLeak: "negative",
  highestCannibalization: "negative",
  highestLostClicks: "negative",
};

export function highlightMetricTone(metric: HighlightMetric): HighlightTone {
  return HIGHLIGHT_METRIC_TONE[metric];
}

/** Tone for a highlight line; metric highlights derive it from the metric. */
export function highlightTone(highlight: HoverHighlight): HighlightTone {
  return "description" in highlight
    ? (highlight.tone ?? "positive")
    : highlightMetricTone(highlight.metric);
}

/** Full highlight line for a metric. Kept as one phrase so translations can reorder words. */
export function highlightMetricLabel(metric: HighlightMetric): string {
  return HIGHLIGHT_METRIC_LABELS[metric];
}

/** Builds one highlight per metric the row leads. */
export function buildHoverHighlights(
  metrics: HighlightMetric[],
  sourceGroup?: SourceGroup,
): HoverHighlight[] {
  return metrics.map((metric) => ({
    metric,
    ...(sourceGroup ? { sourceGroup } : {}),
  }));
}

function shareOf(part: number, total: number): string {
  if (total <= 0) {
    return "—";
  }
  return formatPercentRatio(part / total);
}

/** Revenue amounts from the API are in thousandths of a dollar. */
const REVENUE_SHARE_EQUAL_EPSILON = 10;

function changeLine(
  change: MetricChange,
  higherIsBetter = true,
): Pick<HoverStatLine, "changeRatio" | "higherIsBetter"> {
  return {
    changeRatio: change.ratio,
    higherIsBetter,
  };
}

function revenueLine(
  label: string,
  amount: number,
  shareTotal: number,
  change: MetricChange,
  higherIsBetter = true,
): HoverStatLine {
  const formatted = formatRevenueAmount(amount);
  const isFullShare =
    shareTotal > 0 && Math.abs(amount - shareTotal) <= REVENUE_SHARE_EQUAL_EPSILON;
  return {
    label,
    value: isFullShare ? formatted : `${formatted} (${shareOf(amount, shareTotal)})`,
    ...changeLine(change, higherIsBetter),
  };
}

function countLine(
  label: string,
  count: number,
  change: MetricChange,
  higherIsBetter = true,
): HoverStatLine {
  return {
    label,
    value: count.toLocaleString(),
    ...changeLine(change, higherIsBetter),
  };
}

type HoverContext = {
  totals: BucketTotals;
  overview: OverviewResponse;
  metrics: PeriodMetrics;
};

function grossRevenue(totals: BucketTotals): number {
  return totals.recurring + totals.new_subscriptions + totals.other;
}

function buildTotalRevenue(ctx: HoverContext): MetricHoverContent {
  const gross = grossRevenue(ctx.totals);
  const refunds = Math.abs(ctx.totals.refunds);
  const chargebacks = Math.abs(ctx.totals.chargebacks);
  const net = gross - refunds - chargebacks;
  const { totals_changes: tc, hover_changes: hc } = ctx.overview;
  return {
    stats: [
      revenueLine("Recurring", ctx.totals.recurring, gross, tc.recurring),
      revenueLine("New subscriptions", ctx.totals.new_subscriptions, gross, tc.new_subscriptions),
      revenueLine("Non-subscription", ctx.totals.other, gross, tc.other),
      revenueLine("Refunds", refunds, gross, tc.refunds, false),
      revenueLine("Chargebacks", chargebacks, gross, tc.chargebacks, false),
      {
        label: "Net after refunds",
        value: formatRevenueAmount(net),
        ...changeLine(hc.net_after_refunds),
      },
    ],
  };
}

function buildNewRevenue(ctx: HoverContext): MetricHoverContent {
  const total = ctx.metrics.new_revenue;
  const count = ctx.totals.other_payments_count;
  const avg = count > 0 ? formatRevenueAmount(total / count) : "—";
  const { totals_changes: tc, hover_changes: hc, changes } = ctx.overview;
  return {
    stats: [
      revenueLine("Non-subscription", ctx.totals.other, total, changes.new_revenue),
      countLine("Payments", count, tc.other_payments_count),
      {
        label: "Avg payment",
        value: avg,
        ...changeLine(hc.avg_payment),
      },
    ],
    description: "Non-subscription payment revenue in the period.",
  };
}

function growthRateLine(label: string, rate: number | undefined): HoverStatLine {
  if (rate === undefined) {
    return { label, value: "—" };
  }
  return {
    label,
    value: "",
    changeRatio: rate,
    changeInValueColumn: true,
  };
}

function buildMrr(ctx: HoverContext): MetricHoverContent {
  const start = ctx.overview.previous_mrr;
  const end = ctx.overview.mrr;
  const added = ctx.metrics.new_subscriptions_mrr;
  const removed = ctx.metrics.churn_mrr;
  const netChange = end - start;
  const growth = ctx.overview.mrr_growth;
  const { changes } = ctx.overview;
  return {
    stats: [
      {
        label: "MRR at period start",
        value: formatRevenueAmount(start),
      },
      {
        label: "New MRR added",
        value: formatRevenueAmount(added),
        ...changeLine(changes.new_subscriptions_mrr),
      },
      {
        label: "MRR lost to churn",
        value: formatRevenueAmount(removed),
        ...changeLine(changes.churn_mrr, false),
      },
      {
        label: "Net MRR change",
        value: formatRevenueAmount(netChange),
      },
      growthRateLine("Monthly growth rate", growth?.monthly),
      growthRateLine("Annualized growth rate", growth?.annualized),
    ],
  };
}

function buildNewSubscriptionsMrr(ctx: HoverContext): MetricHoverContent {
  const gross = ctx.totals.new_subscriptions;
  const mrr = ctx.metrics.new_subscriptions_mrr;
  const count = ctx.totals.new_subscriptions_count;
  const annualCount = ctx.totals.new_annual_subscriptions_count;
  const ratio = gross > 0 ? formatPercentRatio(mrr / gross) : "—";
  const { totals_changes: tc, hover_changes: hc } = ctx.overview;
  return {
    stats: [
      countLine("New subscriptions", count, tc.new_subscriptions_count),
      countLine("New annual subscriptions", annualCount, tc.new_annual_subscriptions_count),
      {
        label: "Gross new-sub revenue",
        value: formatRevenueAmount(gross),
        ...changeLine(tc.new_subscriptions),
      },
      {
        label: "MRR added",
        value: formatRevenueAmount(mrr),
        ...changeLine(tc.new_subscriptions_mrr),
      },
      {
        label: "MRR ÷ gross payment",
        value: ratio,
        ...changeLine(hc.mrr_div_gross_payment),
      },
    ],
  };
}

function buildChurnMrr(ctx: HoverContext): MetricHoverContent {
  const count = ctx.metrics.churn_count;
  const mrr = ctx.metrics.churn_mrr;
  const avg = count > 0 ? formatRevenueAmount(mrr / count) : "—";
  const { changes, hover_changes: hc } = ctx.overview;
  return {
    stats: [
      countLine("Churned subscriptions", count, changes.churn_count, false),
      {
        label: "Total MRR removed",
        value: formatRevenueAmount(mrr),
        ...changeLine(changes.churn_mrr, false),
      },
      {
        label: "Avg MRR per churn",
        value: avg,
        ...changeLine(hc.avg_mrr_per_churn, false),
      },
    ],
  };
}

function buildRefunds(ctx: HoverContext): MetricHoverContent {
  const refunds = Math.abs(ctx.totals.refunds);
  const chargebacks = Math.abs(ctx.totals.chargebacks);
  const gross = grossRevenue(ctx.totals);
  const total = refunds + chargebacks;
  const { totals_changes: tc, hover_changes: hc } = ctx.overview;
  return {
    stats: [
      revenueLine("Refunds", refunds, gross, tc.refunds, false),
      revenueLine("Chargebacks", chargebacks, gross, tc.chargebacks, false),
      {
        label: "Total outflows",
        value: `${formatRevenueAmount(total)} (${shareOf(total, gross)})`,
        ...changeLine(hc.total_refund_outflows, false),
      },
      countLine(
        "Refund/chargeback events",
        ctx.totals.refunds_chargebacks_count,
        tc.refunds_chargebacks_count,
        false,
      ),
    ],
  };
}

function buildVisitors(ctx: HoverContext): MetricHoverContent {
  const converting = ctx.overview.current.converting_visitors;
  const { totals_changes: tc, hover_changes: hc } = ctx.overview;
  return {
    stats: [
      countLine("Converting visitors", converting, hc.converting_visitors),
      {
        label: "Conversion share",
        value: shareOf(converting, trackedVisitors(ctx)),
        ...changeLine(hc.conversion_share),
      },
      countLine("Bounced sessions", ctx.totals.bounced_sessions, tc.bounced_sessions, false),
    ],
  };
}

/** Imported visitors have no purchases, so rates over purchases leave them out. */
function trackedVisitors(ctx: HoverContext): number {
  return ctx.totals.visitors - ctx.totals.imported_visitors;
}

function denominatorLabel(ctx: HoverContext): string {
  return ctx.totals.imported_visitors > 0 ? "Tracked visitors" : "Visitors";
}

function buildRevenuePerVisitor(ctx: HoverContext): MetricHoverContent {
  const attributed = visitorAttributedRevenue(ctx.totals);
  const { totals_changes: tc, changes } = ctx.overview;
  return {
    stats: [
      revenueLine(
        "New subscriptions",
        ctx.totals.new_subscriptions,
        attributed,
        tc.new_subscriptions,
      ),
      revenueLine("Non-subscription", ctx.totals.other, attributed, tc.other),
      // The change on hand is total visitors', which isn't this count's.
      ctx.totals.imported_visitors > 0
        ? { label: denominatorLabel(ctx), value: trackedVisitors(ctx).toLocaleString() }
        : countLine("Visitors", ctx.totals.visitors, tc.visitors),
      {
        label: "Revenue per visitor",
        value: formatRevenueAmount(ctx.metrics.revenue_per_visitor),
        ...changeLine(changes.revenue_per_visitor),
      },
    ],
    description:
      "New revenue attributed to visitors in the period, divided by unique visitors. Recurring subscription revenue is excluded because it is not tied to visitors in this period.",
  };
}

function buildConversionRate(ctx: HoverContext): MetricHoverContent {
  const converting = ctx.overview.current.converting_visitors;
  const unattributed = ctx.totals.unattributed_converting_visitors;
  const current = ctx.overview.current;
  const { totals_changes: tc, hover_changes: hc, changes } = ctx.overview;
  return {
    stats: [
      countLine("Converting visitors", converting, hc.converting_visitors),
      countLine("Unattributed conversions", unattributed, tc.unattributed_converting_visitors),
      {
        label: "Conversion rate",
        value: formatPercentRatio(ctx.metrics.conversion_rate, 3),
        ...changeLine(changes.conversion_rate),
      },
      {
        label: "95% CI",
        value: formatWilsonScoreInterval(ctx.overview.conversion_rate_ci, 3),
      },
      {
        label: "Avg revenue / conversion",
        value: formatAvgRevenuePerConversion(current.new_revenue, current.converting_visitors),
        ...changeLine(hc.avg_revenue_per_conversion),
      },
      {
        label: "Avg time to purchase",
        value: formatAvgTimeToPurchase(current.avg_time_to_purchase),
        ...changeLine(hc.avg_time_to_purchase, false),
      },
    ],
    description: "Average revenue per conversion is new revenue divided by converting visitors.",
  };
}

function buildSessionTime(ctx: HoverContext): MetricHoverContent {
  const totalSeconds = ctx.totals.session_time_total;
  const visitors = ctx.totals.visitors;
  const canCompareAverage = visitors > 0 && ctx.overview.previous_totals.visitors > 0;
  const { totals_changes: tc, changes } = ctx.overview;
  return {
    stats: [
      {
        label: "Total session time",
        value: formatSessionDuration(totalSeconds),
        ...changeLine(tc.session_time_total),
      },
      countLine("Visitors (denominator)", visitors, tc.visitors),
      {
        label: "Average per visitor",
        value: formatSessionDuration(ctx.metrics.session_time),
        ...(canCompareAverage ? changeLine(changes.session_time) : {}),
      },
    ],
  };
}

function buildBounceRate(ctx: HoverContext): MetricHoverContent {
  const visitors = ctx.totals.visitors;
  const bounced = ctx.totals.bounced_sessions;
  const engaged = Math.max(0, visitors - bounced);
  const canCompareBounceRate = visitors > 0 && ctx.overview.previous_totals.visitors > 0;
  const { totals_changes: tc, hover_changes: hc, changes } = ctx.overview;
  return {
    stats: [
      countLine("Bounced sessions", bounced, tc.bounced_sessions, false),
      countLine("Engaged visitors", engaged, hc.engaged_visitors),
      {
        label: "Bounce rate",
        value: formatPercentRatio(ctx.metrics.bounce_rate),
        ...(canCompareBounceRate ? changeLine(changes.bounce_rate, false) : {}),
      },
    ],
    description: "Share of visitors whose session was a single pageview lasting under 10 seconds.",
  };
}

const BUILDERS: Record<keyof PeriodMetrics, (ctx: HoverContext) => MetricHoverContent> = {
  total_revenue: buildTotalRevenue,
  new_revenue: buildNewRevenue,
  mrr: buildMrr,
  new_subscriptions_mrr: buildNewSubscriptionsMrr,
  churn_mrr: buildChurnMrr,
  churn_count: buildChurnMrr,
  refunds_and_chargebacks: buildRefunds,
  visitors: buildVisitors,
  revenue_per_visitor: buildRevenuePerVisitor,
  conversion_rate: buildConversionRate,
  session_time: buildSessionTime,
  bounce_rate: buildBounceRate,
};

export function buildMetricHoverContent(
  key: keyof PeriodMetrics,
  overview: OverviewResponse,
  metrics: PeriodMetrics,
): MetricHoverContent {
  const totals = sumOverviewBuckets(overview.buckets);
  const builder = BUILDERS[key];
  return builder({
    totals,
    overview,
    metrics,
  });
}
