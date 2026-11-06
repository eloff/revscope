//! Derived analytics metrics computed server-side.
//!
//! These were previously computed in the dashboard frontend; they are ported
//! here so the API returns display-ready values (conversion rate, revenue per
//! visitor, Wilson intervals). The arithmetic intentionally mirrors the former
//! TypeScript implementation.

use serde::Serialize;

/// z for a 95% Wilson score interval.
const WILSON_95_Z_SCORE: f64 = 1.959_963_984_540_054;

const DAY_SECONDS: f64 = 86_400.0;
/// Length of a nominal month, matching [`crate::comparison::PREVIOUS_PERIOD_SECONDS`].
const MONTH_SECONDS: f64 = 30.0 * DAY_SECONDS;
const YEAR_SECONDS: f64 = 365.0 * DAY_SECONDS;

/// Monthly and annualized MRR growth rates normalized from a selected period.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub(crate) struct MrrGrowthRates {
    pub(crate) monthly: f64,
    pub(crate) annualized: f64,
}

/// Compound monthly and annualized growth from net MRR over a period.
///
/// Period growth is `(mrr - previous_mrr) / previous_mrr`. Monthly and
/// annualized rates extrapolate that growth to a 30-day month and 365-day year.
pub(crate) fn mrr_growth_rates(
    mrr: i64,
    previous_mrr: i64,
    elapsed_seconds: i64,
) -> Option<MrrGrowthRates> {
    if previous_mrr <= 0 || elapsed_seconds <= 0 {
        return None;
    }
    let period_growth = (mrr - previous_mrr) as f64 / previous_mrr as f64;
    let elapsed = elapsed_seconds as f64;
    let monthly = (1.0 + period_growth).powf(MONTH_SECONDS / elapsed) - 1.0;
    let annualized = (1.0 + period_growth).powf(YEAR_SECONDS / elapsed) - 1.0;
    if monthly.is_finite() && annualized.is_finite() {
        Some(MrrGrowthRates {
            monthly,
            annualized,
        })
    } else {
        None
    }
}

/// How many months one billing period spans.
///
/// `interval_count` is the number of `interval_unit`s per billing period, so a
/// price billed every three months spans three months. Returns `None` for
/// interval units we can't normalize.
pub(crate) fn months_per_period(interval_unit: &str, interval_count: i64) -> Option<f64> {
    let interval_count = interval_count.max(1) as f64;
    let months = match interval_unit {
        "month" => interval_count,
        "year" => interval_count * 12.0,
        "week" => interval_count * 12.0 / 52.0,
        "day" => interval_count * 12.0 / 365.25,
        _ => return None,
    };
    if months > 0.0 { Some(months) } else { None }
}

/// Normalizes a recurring amount charged once per billing interval to a monthly
/// amount, in the same integer thousandths unit as the input.
///
/// A price billed every three months contributes a third of its amount to MRR.
/// Returns `None` for interval units we can't normalize.
pub(crate) fn monthly_amount(amount: i64, interval_unit: &str, interval_count: i64) -> Option<i64> {
    let months = months_per_period(interval_unit, interval_count)?;
    Some((amount as f64 / months).round() as i64)
}

/// `numerator / denominator`, or zero when there is nothing to divide by.
pub(crate) fn ratio(numerator: i64, denominator: i64) -> f64 {
    if denominator > 0 {
        numerator as f64 / denominator as f64
    } else {
        0.0
    }
}

/// Conversions ÷ visitors (0 when there are no visitors).
pub(crate) fn conversion_rate(converting_visitors: f64, visitors: f64) -> f64 {
    if visitors > 0.0 {
        converting_visitors / visitors
    } else {
        0.0
    }
}

/// Revenue ÷ visitors (0 when there are no visitors).
pub(crate) fn revenue_per_visitor(revenue: f64, visitors: f64) -> f64 {
    if visitors > 0.0 {
        revenue / visitors
    } else {
        0.0
    }
}

/// 95% Wilson score confidence interval for a binomial proportion.
#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub(crate) struct WilsonInterval {
    pub(crate) lower: f64,
    pub(crate) upper: f64,
}

/// 95% Wilson score interval, or `None` when there are no visitors.
pub(crate) fn conversion_rate_wilson_interval(
    converting_visitors: f64,
    visitors: f64,
) -> Option<WilsonInterval> {
    if visitors <= 0.0 {
        return None;
    }
    let successes = converting_visitors.clamp(0.0, visitors);
    let rate = successes / visitors;
    let z_squared = WILSON_95_Z_SCORE * WILSON_95_Z_SCORE;
    let denominator = 1.0 + z_squared / visitors;
    let center = (rate + z_squared / (2.0 * visitors)) / denominator;
    let margin = (WILSON_95_Z_SCORE
        * ((rate * (1.0 - rate) + z_squared / (4.0 * visitors)) / visitors).sqrt())
        / denominator;
    Some(WilsonInterval {
        lower: (center - margin).max(0.0),
        upper: (center + margin).min(1.0),
    })
}

/// Lower bound of the 95% Wilson interval (0 when undefined). Used for ranking.
pub(crate) fn conversion_rate_wilson_lower_bound(converting_visitors: f64, visitors: f64) -> f64 {
    conversion_rate_wilson_interval(converting_visitors, visitors)
        .map(|interval| interval.lower)
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "expected {a} ≈ {b}");
    }

    #[test]
    fn conversion_rate_is_zero_without_visitors() {
        approx(conversion_rate(3.0, 0.0), 0.0);
        approx(conversion_rate(1.0, 4.0), 0.25);
    }

    #[test]
    fn wilson_interval_brackets_point_estimate() {
        let interval = conversion_rate_wilson_interval(5.0, 20.0).expect("interval");
        assert!(interval.lower < 0.25 && interval.upper > 0.25);
        assert!(interval.lower >= 0.0 && interval.upper <= 1.0);
        assert!(conversion_rate_wilson_interval(0.0, 0.0).is_none());
    }

    #[test]
    fn mrr_growth_rates_normalize_to_month_and_year() {
        let thirty_days = 30 * 86_400;
        let rates = mrr_growth_rates(110, 100, thirty_days).expect("rates for 10% over 30 days");
        approx(rates.monthly, 0.1);
        approx(rates.annualized, (1.1_f64).powf(365.0 / 30.0) - 1.0);

        let seven_days = 7 * 86_400;
        let rates = mrr_growth_rates(110, 100, seven_days).expect("rates for 10% over 7 days");
        approx(rates.monthly, (1.1_f64).powf(30.0 / 7.0) - 1.0);
        approx(rates.annualized, (1.1_f64).powf(365.0 / 7.0) - 1.0);

        assert!(mrr_growth_rates(100, 0, thirty_days).is_none());
        assert!(mrr_growth_rates(100, 100, 0).is_none());
    }
}
