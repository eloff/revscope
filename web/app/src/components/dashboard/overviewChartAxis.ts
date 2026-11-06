/** Target y-axis tick intervals (step ≈ range / this count). */
export const Y_AXIS_TARGET_INTERVALS = 8;

const REVENUE_SCALE = 1000;

/** Smallest 1/2/5×10ⁿ step ≥ `rough`. */
export function niceStepCeil(rough: number): number {
  if (!Number.isFinite(rough) || rough <= 0) {
    return 1;
  }
  const power = 10 ** Math.floor(Math.log10(rough));
  const normalized = rough / power;
  if (normalized <= 1) {
    return power;
  }
  if (normalized <= 2) {
    return 2 * power;
  }
  if (normalized <= 5) {
    return 5 * power;
  }
  return 10 * power;
}

export type NiceAxisBounds = {
  min: number;
  max: number;
  step: number;
  ticksBelowZero: number;
  ticksAboveZero: number;
};

/**
 * Pick a nice step from the data span, then expand min/max to tick multiples.
 * `dataMin` may be negative; `dataMax` is the positive extent (≥ 0).
 */
export function niceAxisBoundsFromExtent(dataMin: number, dataMax: number): NiceAxisBounds {
  const max = Math.max(dataMax, 0);
  const min = Math.min(dataMin, 0);
  const span = max - min;
  const step =
    span > 0
      ? niceStepCeil(span / Y_AXIS_TARGET_INTERVALS)
      : niceStepCeil(max / Y_AXIS_TARGET_INTERVALS);

  const ticksBelowZero = min < 0 ? Math.max(1, Math.ceil(Math.abs(min) / step)) : 0;
  const ticksAboveZero = Math.max(1, Math.ceil(max / step));

  return {
    min: -ticksBelowZero * step,
    max: ticksAboveZero * step,
    step,
    ticksBelowZero,
    ticksAboveZero,
  };
}

export type NiceRevenueAxisBounds = NiceAxisBounds & {
  stepDollars: number;
};

/** Revenue amounts are in thousandths of a dollar. */
export function niceRevenueAxisBounds(
  revenueMin: number,
  revenueMax: number,
): NiceRevenueAxisBounds {
  const minDollars = revenueMin / REVENUE_SCALE;
  const maxDollars = revenueMax / REVENUE_SCALE;
  const bounds = niceAxisBoundsFromExtent(minDollars, maxDollars);

  return {
    ...bounds,
    min: bounds.min * REVENUE_SCALE,
    max: bounds.max * REVENUE_SCALE,
    step: bounds.step * REVENUE_SCALE,
    stepDollars: bounds.step,
  };
}

/** Line metrics stored in native units (visitors, seconds, 0–1 rates, etc.). */
export function niceLineAxisBounds(lineMax: number): NiceAxisBounds {
  const max = Math.max(lineMax, 0);
  return niceAxisBoundsFromExtent(0, max);
}
