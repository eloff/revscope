//! How much history each plan keeps.
//!
//! The pricing page's retention lines in web/site/src/lib/pricing.ts state these
//! same numbers; change them together.

use jiff::civil::Date;
use jiff::{Span, ToSpan};

/// Months of data the plan keeps, or `None` when it keeps everything.
pub(crate) fn plan_retention_months(plan: &str) -> Option<i64> {
    if cfg!(feature = "self-hosted") {
        return None;
    }
    match plan {
        "starter" => Some(24),
        // Founder is the Growth plan at a launch price.
        "growth" | "founder" => Some(60),
        // Every account in a hosted build without the private billing module.
        "enterprise" => None,
        // A plan type added without an entry here shouldn't get more than Free.
        _ => Some(6),
    }
}

/// The oldest date inside the plan's retention, as of `today`.
pub(crate) fn oldest_retained_date(plan: &str, today: Date) -> Option<Date> {
    let months: Span = plan_retention_months(plan)?.months();
    today.checked_sub(months).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    #[test]
    #[cfg(not(feature = "self-hosted"))]
    fn each_plan_keeps_what_the_pricing_page_promises() {
        assert_eq!(plan_retention_months("free"), Some(6));
        assert_eq!(plan_retention_months("starter"), Some(24));
        assert_eq!(plan_retention_months("growth"), Some(60));
        assert_eq!(plan_retention_months("founder"), Some(60));
        assert_eq!(plan_retention_months("enterprise"), None);
    }

    #[test]
    #[cfg(not(feature = "self-hosted"))]
    fn an_unknown_plan_gets_the_shortest_retention() {
        assert_eq!(plan_retention_months("brand_new_plan"), Some(6));
    }

    #[test]
    #[cfg(feature = "self-hosted")]
    fn self_hosted_keeps_everything_whatever_the_plan() {
        assert_eq!(plan_retention_months("free"), None);
    }

    #[test]
    #[cfg(not(feature = "self-hosted"))]
    fn month_ends_clamp_rather_than_overflow() {
        assert_eq!(
            oldest_retained_date("free", date(2026, 8, 31)),
            Some(date(2026, 2, 28))
        );
    }

    #[test]
    fn enterprise_has_no_oldest_date() {
        assert_eq!(oldest_retained_date("enterprise", date(2026, 8, 31)), None);
    }
}
