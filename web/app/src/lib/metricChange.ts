/**
 * A normalized, display-ready period-over-period change computed server-side.
 *
 * `ratio` is the relative change versus the prior 30 days, with additive
 * quantities normalized to the current period length (`0.25` == +25%). It is
 * `null` when there is no usable comparison (including a zero prior baseline
 * with a non-zero current value, or a breakdown row whose key was absent from
 * the prior 30 days).
 */
export type MetricChange = {
  ratio: number | null;
};
