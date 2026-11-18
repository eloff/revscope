/** What tiles are sized by: revenue, or the traffic behind it (visitors, clicks, views). */
export type TreemapMetric = "revenue" | "traffic";

export type TreemapTile = {
  /** Stable unique key for the ApexCharts datum (`x`). */
  datumKey: string;
  label: string;
  /** Tile area in the metric the tiles were built for. */
  value: number;
};

/** One series: tiles drawn in shades of one color. */
export type TreemapGroup<T extends TreemapTile> = {
  name: string;
  /** Ignored when the treemap's `tileColor` colors each tile. */
  color: string;
  tiles: T[];
};

/**
 * Orders rows largest first by `metric`, then by the other metric, then by
 * `tiebreak`, so which rows make a treemap's top doesn't depend on input order.
 */
export function compareByMetric<R>(
  metric: TreemapMetric,
  value: (row: R, metric: TreemapMetric) => number,
  tiebreak: (left: R, right: R) => number,
): (left: R, right: R) => number {
  const other: TreemapMetric = metric === "revenue" ? "traffic" : "revenue";
  return (left, right) =>
    value(right, metric) - value(left, metric) ||
    value(right, other) - value(left, other) ||
    tiebreak(left, right);
}
