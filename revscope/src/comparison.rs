//! Shared helpers for period-over-period comparisons.
//!
//! The comparison ("previous") window is always the 30 days immediately
//! preceding the selected range, regardless of how long the selected range is.
//! Because the two windows can have different lengths, absolute previous-period
//! values (revenue, visitors, counts, total session time, …) are normalized to
//! the elapsed length of the current period before any delta is computed. Rate
//! metrics (conversion rate, revenue per visitor, bounce rate, average session
//! time, …) already have the relevant quantity in their denominator, so they
//! are compared directly without normalization.

use serde::Serialize;

pub(crate) const DAY_SECONDS: i64 = 86_400;

/// Length of the comparison window: the 30 days before the selected range.
pub(crate) const PREVIOUS_PERIOD_SECONDS: i64 = 30 * DAY_SECONDS;

/// Maximum allowed dashboard date-range length (matches the UI clamp).
pub(crate) const MAX_RANGE_DAYS: i64 = 366;

/// Maximum allowed `end - start` for half-open dashboard ranges.
pub(crate) const MAX_RANGE_SECONDS: i64 = MAX_RANGE_DAYS * DAY_SECONDS;

/// Earliest instant a range may reference (1900-01-01), leaving room below it
/// for the comparison window.
const MIN_RANGE_SECOND: i64 = -2_208_988_800;
/// Latest instant a range may reference (2200-01-01).
const MAX_RANGE_SECOND: i64 = 7_258_118_400;

/// Validate a required half-open `[start, end)` dashboard date range.
///
/// Ensures both bounds are present, `end > start`, and the span is at most
/// [`MAX_RANGE_DAYS`] days.
pub(crate) fn validate_date_range(
    start: Option<i64>,
    end: Option<i64>,
) -> Result<(i64, i64), &'static str> {
    let start = start.ok_or("start is required")?;
    let end = end.ok_or("end is required")?;
    validate_date_range_bounds(start, end)?;
    Ok((start, end))
}

/// Validate bounds for an already-parsed half-open `[start, end)` range.
///
/// Bounds are also held to a calendar window no dashboard can sensibly ask
/// beyond, so the timezone-aware bucketing downstream is always working with
/// instants it can convert to a real calendar date.
pub(crate) fn validate_date_range_bounds(start: i64, end: i64) -> Result<(), &'static str> {
    if end <= start {
        return Err("end must be after start");
    }
    if end - start > MAX_RANGE_SECONDS {
        return Err("date range must be at most 366 days");
    }
    if start < MIN_RANGE_SECOND || end > MAX_RANGE_SECOND {
        return Err("date range is outside the supported calendar");
    }
    Ok(())
}

/// Start of the comparison window for a selected range starting at `start`.
///
/// The comparison window is `[start - 30 days, start)`.
pub(crate) fn previous_period_start(start: i64) -> i64 {
    start.saturating_sub(PREVIOUS_PERIOD_SECONDS)
}

/// Seconds of the selected period that have actually elapsed.
///
/// When the selected range extends past `now` (it includes today, for which we
/// have no future data), the elapsed length is `now - start` rather than the
/// nominal `end - start`.
pub(crate) fn current_elapsed_seconds(start: i64, end: i64, now: i64) -> i64 {
    (end.min(now) - start).max(0)
}

/// Factor that scales a 30-day previous-period absolute value down (or up) to
/// the elapsed length of the current period, so the two are directly
/// comparable.
///
/// Returns `current_elapsed_seconds / 30 days`.
pub(crate) fn normalization_factor(start: i64, end: i64, now: i64) -> f64 {
    current_elapsed_seconds(start, end, now) as f64 / PREVIOUS_PERIOD_SECONDS as f64
}

/// A normalized, display-ready period-over-period change.
///
/// `ratio` is the relative change versus the normalized previous value
/// (`0.25` == +25%). It is:
/// - `Some(r)` when there is a usable baseline (previous normalized value > 0),
/// - `Some(0.0)` when both the current and previous values are zero,
/// - `None` when no comparison is available (previous was zero with a non-zero
///   current, or a breakdown row's key was absent from the prior 30 days).
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq)]
pub(crate) struct Change {
    pub(crate) ratio: Option<f64>,
}

impl Change {
    /// No comparison available.
    pub(crate) fn none() -> Self {
        Self { ratio: None }
    }

    /// Build a change from a current value and an *already-normalized* previous
    /// value (both expressed on the same time base).
    pub(crate) fn from_normalized(current: f64, previous_normalized: f64) -> Self {
        if previous_normalized == 0.0 {
            if current == 0.0 {
                Self { ratio: Some(0.0) }
            } else {
                Self::none()
            }
        } else {
            Self {
                ratio: Some((current - previous_normalized) / previous_normalized),
            }
        }
    }

    /// Build a change for an absolute metric: the raw previous value is first
    /// normalized to the current period length using `factor`.
    pub(crate) fn absolute(current: f64, previous_raw: f64, factor: f64) -> Self {
        Self::from_normalized(current, previous_raw * factor)
    }

    /// Build a change for a rate metric (already normalized by construction):
    /// the previous value is compared directly without scaling.
    pub(crate) fn rate(current: f64, previous: f64) -> Self {
        Self::from_normalized(current, previous)
    }

    /// Compare an absolute metric for a breakdown row when the row's key may be
    /// absent from the prior 30 days.
    pub(crate) fn breakdown_absolute(current: f64, previous: Option<f64>, factor: f64) -> Self {
        match previous {
            Some(p) => Self::absolute(current, p, factor),
            None => Self::none(),
        }
    }

    /// Compare a rate metric for a breakdown row when the row's key may be
    /// absent from the prior 30 days.
    pub(crate) fn breakdown_rate(current: f64, previous: Option<f64>) -> Self {
        match previous {
            Some(p) => Self::rate(current, p),
            None => Self::none(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previous_window_is_thirty_days_before_start() {
        assert_eq!(previous_period_start(0), -PREVIOUS_PERIOD_SECONDS);
        assert_eq!(previous_period_start(100 * DAY_SECONDS), 70 * DAY_SECONDS);
    }

    #[test]
    fn elapsed_seconds_clamp_to_now_for_today_periods() {
        // Range fully in the past: elapsed is the whole range.
        assert_eq!(
            current_elapsed_seconds(0, 3 * DAY_SECONDS, 100 * DAY_SECONDS),
            3 * DAY_SECONDS
        );
        // Range includes the future (today): elapsed is now - start.
        assert_eq!(
            current_elapsed_seconds(0, 3 * DAY_SECONDS, DAY_SECONDS),
            DAY_SECONDS
        );
        // Entirely in the future: no elapsed time.
        assert_eq!(current_elapsed_seconds(10, 20, 5), 0);
    }

    #[test]
    fn normalization_factor_scales_thirty_days_to_current_length() {
        // 3-day past range -> 3/30.
        let factor = normalization_factor(0, 3 * DAY_SECONDS, 100 * DAY_SECONDS);
        assert!((factor - 0.1).abs() < 1e-12);
        // 30-day past range -> 1.0.
        let factor = normalization_factor(0, 30 * DAY_SECONDS, 100 * DAY_SECONDS);
        assert!((factor - 1.0).abs() < 1e-12);
    }

    #[test]
    fn change_from_normalized_handles_zero_baseline() {
        assert_eq!(Change::from_normalized(0.0, 0.0).ratio, Some(0.0));
        assert_eq!(Change::from_normalized(5.0, 0.0), Change::none());
        let change = Change::from_normalized(150.0, 100.0);
        assert_eq!(change.ratio, Some(0.5));
    }

    #[test]
    fn absolute_change_normalizes_previous_first() {
        // current 3, previous raw 30 over 30 days, factor 0.1 -> normalized 3.
        let change = Change::absolute(3.0, 30.0, 0.1);
        assert_eq!(change.ratio, Some(0.0));
    }

    #[test]
    fn breakdown_row_without_prior_key_is_none() {
        assert_eq!(Change::breakdown_absolute(5.0, None, 1.0), Change::none());
        assert_eq!(Change::breakdown_rate(0.1, None), Change::none());
    }

    #[test]
    fn breakdown_row_with_zero_prior_is_none() {
        assert_eq!(
            Change::breakdown_absolute(5.0, Some(0.0), 1.0),
            Change::none()
        );
    }

    #[test]
    fn validate_date_range_requires_bounds_and_order() {
        assert_eq!(
            validate_date_range(None, Some(10)).unwrap_err(),
            "start is required"
        );
        assert_eq!(
            validate_date_range(Some(10), None).unwrap_err(),
            "end is required"
        );
        assert_eq!(
            validate_date_range(Some(10), Some(10)).unwrap_err(),
            "end must be after start"
        );
        assert_eq!(
            validate_date_range(Some(10), Some(20)).expect("ok"),
            (10, 20)
        );
    }

    #[test]
    fn validate_date_range_rejects_spans_longer_than_max() {
        let start = 0;
        let ok_end = MAX_RANGE_SECONDS;
        let too_long_end = MAX_RANGE_SECONDS + 1;
        assert_eq!(
            validate_date_range(Some(start), Some(ok_end)).expect("max ok"),
            (start, ok_end)
        );
        assert_eq!(
            validate_date_range(Some(start), Some(too_long_end)).unwrap_err(),
            "date range must be at most 366 days"
        );
    }

    #[test]
    fn validate_date_range_rejects_instants_outside_the_calendar() {
        assert_eq!(
            validate_date_range(Some(MIN_RANGE_SECOND - DAY_SECONDS), Some(MIN_RANGE_SECOND))
                .unwrap_err(),
            "date range is outside the supported calendar"
        );
        assert_eq!(
            validate_date_range(
                Some(MAX_RANGE_SECOND - DAY_SECONDS),
                Some(MAX_RANGE_SECOND + 1)
            )
            .unwrap_err(),
            "date range is outside the supported calendar"
        );
        // Both edges themselves are usable.
        assert!(
            validate_date_range(Some(MIN_RANGE_SECOND), Some(MIN_RANGE_SECOND + DAY_SECONDS))
                .is_ok()
        );
        assert!(
            validate_date_range(Some(MAX_RANGE_SECOND - DAY_SECONDS), Some(MAX_RANGE_SECOND))
                .is_ok()
        );
    }
}
