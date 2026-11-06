import { gridChangeNegative, gridChangeNeutral, gridChangePositive } from "./grid.css";
import { changeNegative, changeNeutral, changePositive } from "./metricChange.css";

export type ChangeTone = "positive" | "negative" | "neutral";

/**
 * Whether a period-over-period change is good, bad, or flat. A missing or
 * non-finite ratio is flat.
 */
export function changeToneForRatio(ratio: number | null, higherIsBetter = true): ChangeTone {
  if (ratio === null || !Number.isFinite(ratio) || ratio === 0) {
    return "neutral";
  }
  return ratio > 0 === higherIsBetter ? "positive" : "negative";
}

const METRIC_CHANGE_CLASS: Record<ChangeTone, string> = {
  positive: changePositive,
  negative: changeNegative,
  neutral: changeNeutral,
};

const GRID_CHANGE_CLASS: Record<ChangeTone, string> = {
  positive: gridChangePositive,
  negative: gridChangeNegative,
  neutral: gridChangeNeutral,
};

/** Standalone change badge, as on metric tiles and hover cards. */
export function metricChangeClass(tone: ChangeTone): string {
  return METRIC_CHANGE_CLASS[tone];
}

/** Change shown inline in a grid cell or summary card. */
export function gridChangeClass(ratio: number): string {
  return GRID_CHANGE_CLASS[changeToneForRatio(ratio)];
}
