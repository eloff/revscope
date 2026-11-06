import {
  formatPercentRatio,
  formatRevenueAmount,
  formatSessionDuration,
} from "../../lib/dashboardApi";
import type { PageRow } from "../../lib/pagesApi";
import { formatWilsonScoreInterval } from "./conversionConfidence";
import type { HoverHighlight, HoverStatLine, MetricHoverContent } from "./metricHoverDetails";

function plainStatLine(label: string, value: string): HoverStatLine {
  return { label, value };
}

function marginalLine(label: string, amount: number): HoverStatLine {
  return plainStatLine(label, formatRevenueAmount(amount));
}

export type PagesVariant = "pages" | "landingPages";

/**
 * Hover-card content for a single page row. Includes core engagement metrics
 * and the server-computed marginal revenue opportunities.
 */
export function buildPageHoverContent(
  row: PageRow,
  variant: PagesVariant,
  highlightDescriptions?: HoverHighlight[],
): MetricHoverContent {
  const revenueLabel = variant === "landingPages" ? "Revenue" : "Attributed revenue";
  const stats: HoverStatLine[] = [
    plainStatLine("Visitors", row.visitors.toLocaleString()),
    plainStatLine(revenueLabel, formatRevenueAmount(row.attributed_revenue)),
    plainStatLine("Revenue / visitor", formatRevenueAmount(row.revenue_per_visitor)),
    plainStatLine("Conversions", row.converting_visitors.toLocaleString()),
    plainStatLine("Conversion rate", formatWilsonScoreInterval(row.conversion_ci, 2)),
  ];

  if (variant === "landingPages") {
    stats.push(plainStatLine("Bounce rate", formatPercentRatio(row.bounce_rate, 1)));
  } else {
    stats.push(plainStatLine("Exit rate", formatPercentRatio(row.exit_rate, 1)));
  }

  stats.push(
    plainStatLine("Avg session duration", formatSessionDuration(row.avg_session_duration)),
    plainStatLine("Avg page duration", formatSessionDuration(row.avg_page_duration)),
    plainStatLine("Avg scroll depth", formatPercentRatio(row.avg_scroll_percent / 100, 1)),
    marginalLine("Value of +25% visitors", row.increase_visitors_25pct),
    marginalLine("Value of +25% conversion", row.increase_conversion_25pct),
  );

  if (variant === "pages") {
    stats.push(marginalLine("Value of −25% exit rate", row.reduce_exits_25pct));
  } else {
    stats.push(marginalLine("Value of −25% bounce rate", row.reduce_bounces_25pct));
  }

  stats.push(marginalLine("Value of +25% dwell time", row.increase_session_25pct));

  return {
    stats,
    description: undefined,
    ...(highlightDescriptions?.length ? { highlightDescriptions } : {}),
  };
}
