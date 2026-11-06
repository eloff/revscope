import { type ConversionInterval, formatPercentRatio } from "../../lib/dashboardApi";

export function formatWilsonScoreInterval(
  interval: ConversionInterval | null | undefined,
  precision = 2,
): string {
  if (!interval) {
    return "—";
  }
  const lower = (interval.lower * 100).toFixed(precision);
  const upper = formatPercentRatio(interval.upper, precision);
  return `${lower} - ${upper}`;
}

/**
 * Sources-table conversion cell. A measured rate of zero is `0%` with no
 * interval. Rows without an interval (no visitors, or the synthetic Other
 * bucket) stay as an em dash.
 */
export function formatSourceConversionCell(
  conversionRate: number,
  interval: ConversionInterval | null | undefined,
  precision = 2,
): string {
  if (interval && conversionRate <= 0) {
    return "0%";
  }
  return formatWilsonScoreInterval(interval, precision);
}
