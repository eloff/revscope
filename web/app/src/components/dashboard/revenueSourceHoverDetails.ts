import { formatPercentRatio, formatRevenueAmount } from "../../lib/dashboardApi";
import type { GroupSummary } from "../../lib/revenueSourcesApi";
import { formatWilsonScoreInterval } from "./conversionConfidence";
import type {
  HoverHighlight,
  HoverLandingPath,
  HoverStatLine,
  MetricHoverContent,
} from "./metricHoverDetails";
import { conversionCount, type RevenueSourceRecord } from "./revenueSourcesLogic";

function plainStatLine(label: string, value: string): HoverStatLine {
  return { label, value };
}

function landingPathsForRecord(record: RevenueSourceRecord): HoverLandingPath[] | undefined {
  if (record.landingPaths.length === 0) {
    return undefined;
  }
  return record.landingPaths.map((entry) => ({
    id: entry.id,
    path: entry.path,
    visitors: entry.count,
  }));
}

function referrersForRecord(record: RevenueSourceRecord): HoverLandingPath[] | undefined {
  if (!record.isCampaign || record.referrers.length === 0) {
    return undefined;
  }
  return record.referrers.map((entry) => ({
    id: 0,
    path: entry.label,
    visitors: entry.visitors,
  }));
}

/**
 * Hover-card content for a channel-group summary tile, built entirely from
 * server-computed metrics and period-over-period deltas (versus the prior 30
 * days, normalized).
 */
export function buildGroupSummaryHoverContent(summary: GroupSummary): MetricHoverContent {
  const stats: HoverStatLine[] = [
    {
      label: "Revenue",
      value: formatRevenueAmount(summary.revenue),
      changeRatio: summary.revenue_change.ratio,
    },
    {
      label: "Visitors",
      value: summary.visitors.toLocaleString(),
      changeRatio: summary.visitors_change.ratio,
    },
    {
      label: "Revenue / visitor",
      value: formatRevenueAmount(summary.revenue_per_visitor),
      changeRatio: summary.revenue_per_visitor_change.ratio,
    },
    {
      label: "Conversion rate",
      value: formatPercentRatio(summary.conversion_rate, 2),
      changeRatio: summary.conversion_rate_change.ratio,
    },
    plainStatLine("Conversion 95% CI", formatWilsonScoreInterval(summary.conversion_ci ?? null, 2)),
  ];
  return { stats };
}

/**
 * Hover-card content for a single revenue-source row, built from
 * server-computed metrics and deltas. Adds the row's landing paths, the top
 * referrers for campaign rows, and any best-in-group highlight descriptions.
 */
export function buildBestSourceHoverContent(
  record: RevenueSourceRecord,
  highlightDescriptions?: HoverHighlight[],
): MetricHoverContent {
  const conversions = conversionCount(
    record.visitors,
    record.convertingVisitors,
    record.conversionRate,
  );
  const stats: HoverStatLine[] = [
    {
      label: "Revenue",
      value: formatRevenueAmount(record.revenue),
      changeRatio: record.revenueChangeRatio,
    },
    {
      label: "Visitors",
      value: record.visitors.toLocaleString(),
      changeRatio: record.visitorsChangeRatio,
    },
    {
      label: "Conversions",
      value: conversions.toLocaleString(),
      changeRatio: record.conversionsChangeRatio,
    },
    {
      label: "Revenue / visitor",
      value: formatRevenueAmount(record.revenuePerVisitor),
      changeRatio: record.revenuePerVisitorChangeRatio,
    },
    {
      label: "Conversion rate",
      value: formatPercentRatio(record.conversionRate, 2),
      changeRatio: record.conversionRateChangeRatio,
    },
    plainStatLine("Conversion 95% CI", formatWilsonScoreInterval(record.conversionCi ?? null, 2)),
  ];

  return {
    stats,
    landingPaths: landingPathsForRecord(record),
    referrers: referrersForRecord(record),
    ...(highlightDescriptions?.length ? { highlightDescriptions } : {}),
  };
}
