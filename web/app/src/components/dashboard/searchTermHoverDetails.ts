import { formatPercentRatio, formatRevenueAmount } from "../../lib/dashboardApi";
import type { MetricChange, SearchTermRow, SearchTermsPeriod } from "../../lib/searchTermsApi";
import type { HoverHighlight, HoverStatLine, MetricHoverContent } from "./metricHoverDetails";
import { searchTermRowKey } from "./searchTermsLogic";

function plainStatLine(label: string, value: string): HoverStatLine {
  return { label, value };
}

function changeStatLine(
  label: string,
  value: string,
  change: MetricChange,
  higherIsBetter = true,
): HoverStatLine {
  return {
    label,
    value,
    changeRatio: change.ratio,
    higherIsBetter,
  };
}

export function indexSearchTermsByQuery(terms: SearchTermRow[]): Map<string, SearchTermRow> {
  const index = new Map<string, SearchTermRow>();
  for (const term of terms) {
    index.set(searchTermRowKey(term), term);
  }
  return index;
}

export function buildSearchTermHoverContent(
  row: SearchTermRow,
  highlightDescriptions?: HoverHighlight[],
): MetricHoverContent {
  const stats: HoverStatLine[] = [
    changeStatLine("Position", row.position.toFixed(1), row.position_change, false),
    changeStatLine("Impressions", row.impressions.toLocaleString(), row.impressions_change),
    changeStatLine("Clicks", row.clicks.toLocaleString(), row.clicks_change),
    changeStatLine("CTR", formatPercentRatio(row.ctr, 2), row.ctr_change),
    changeStatLine("% Mobile", formatPercentRatio(row.mobile_share, 1), row.mobile_share_change),
    changeStatLine(
      "Est. revenue",
      formatRevenueAmount(row.estimated_revenue),
      row.estimated_revenue_change,
    ),
  ];
  if (row.expected_ctr > 0) {
    stats.push(
      plainStatLine("Expected CTR", formatPercentRatio(row.expected_ctr, 2)),
      plainStatLine("Lost clicks", Math.round(row.lost_clicks).toLocaleString()),
      plainStatLine("Value of matching rank CTR", formatRevenueAmount(row.lost_revenue)),
    );
  }

  const landingPaths = landingPathsForTerm(row);
  return {
    stats,
    ...(landingPaths.length > 0
      ? {
          landingPaths,
          landingPathsTitle: "Landing paths",
          landingPathsCountLabel: "Clicks",
          landingPathsShareLabel: "%",
        }
      : {}),
    ...(highlightDescriptions?.length ? { highlightDescriptions } : {}),
  };
}

function landingPathsForTerm(row: SearchTermRow): NonNullable<MetricHoverContent["landingPaths"]> {
  const paths = [...row.landing_paths].sort((left, right) => {
    if (right.clicks !== left.clicks) {
      return right.clicks - left.clicks;
    }
    return left.path.localeCompare(right.path);
  });
  const totalClicks = paths.reduce((sum, entry) => sum + entry.clicks, 0);
  return paths.map((entry) => ({
    id: entry.path_id,
    path: entry.path,
    visitors: entry.clicks,
    share: totalClicks > 0 ? entry.clicks / totalClicks : 0,
  }));
}

type OtherMetrics = {
  revenue: number;
  impressions: number;
  clicks: number;
  /** Null when no loaded term outside the top has a position to average. */
  position: number | null;
  ctr: number;
  mobileShare: number;
};

function aggregateOtherMetrics(period: SearchTermsPeriod, topQueries: Set<string>): OtherMetrics {
  const topTerms = period.terms.filter((term) => topQueries.has(term.query));
  const topRevenue = topTerms.reduce((total, term) => total + term.estimated_revenue, 0);
  const topImpressions = topTerms.reduce((total, term) => total + term.impressions, 0);
  const topClicks = topTerms.reduce((total, term) => total + term.clicks, 0);
  const otherTerms = period.terms.filter((term) => !topQueries.has(term.query));
  const weightedPosition = otherTerms.reduce(
    (total, term) => total + term.position * term.impressions,
    0,
  );
  const otherTermImpressions = otherTerms.reduce((total, term) => total + term.impressions, 0);
  const impressions = Math.max(0, period.summary.impressions - topImpressions);
  const clicks = Math.max(0, period.summary.clicks - topClicks);
  const otherMobileClicks = otherTerms.reduce((total, term) => total + term.mobile_clicks, 0);
  const otherDetailClicks = otherTerms.reduce((total, term) => total + term.detail_clicks, 0);

  return {
    revenue: Math.max(0, period.search_channel_revenue - topRevenue),
    impressions,
    clicks,
    // Terms are paged, so `impressions` (from the summary) also counts terms not
    // loaded yet; averaging over it would credit them with position 0.
    position:
      otherTermImpressions > 0
        ? weightedPosition / otherTermImpressions
        : otherTerms.length > 0
          ? otherTerms.reduce((total, term) => total + term.position, 0) / otherTerms.length
          : null,
    ctr: impressions > 0 ? clicks / impressions : 0,
    mobileShare: otherDetailClicks > 0 ? otherMobileClicks / otherDetailClicks : 0,
  };
}

export function buildSearchTermOtherHoverContent(
  current: SearchTermsPeriod,
  topQueries: Set<string>,
  topTermCount: number,
): MetricHoverContent {
  const currentMetrics = aggregateOtherMetrics(current, topQueries);
  const stats: HoverStatLine[] = [
    plainStatLine("Est. revenue", formatRevenueAmount(currentMetrics.revenue)),
    plainStatLine("Clicks", currentMetrics.clicks.toLocaleString()),
    plainStatLine("Impressions", currentMetrics.impressions.toLocaleString()),
  ];
  if (currentMetrics.impressions > 0) {
    if (currentMetrics.position !== null) {
      stats.push(plainStatLine("Position", currentMetrics.position.toFixed(1)));
    }
    stats.push(plainStatLine("CTR", formatPercentRatio(currentMetrics.ctr, 2)));
  }
  if (currentMetrics.mobileShare > 0) {
    stats.push(plainStatLine("% Mobile", formatPercentRatio(currentMetrics.mobileShare, 1)));
  }

  return {
    stats,
    description: `Estimated search revenue outside the top ${topTermCount} terms`,
  };
}
