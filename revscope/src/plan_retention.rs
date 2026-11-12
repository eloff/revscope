//! How much history each plan keeps.
//!
//! Older data stays in the project's database; retention only limits what the
//! dashboard shows and how far back a GA import may reach.
//!
//! The pricing page's retention lines in web/site/src/lib/pricing.ts state these
//! same numbers; change them together.

use jiff::civil::Date;
use jiff::{Span, ToSpan};

use crate::projects::plan_includes_paid_features;
use crate::timezone::ProjectTimezone;

/// Months of data the plan keeps, or `None` when it keeps everything.
pub(crate) fn plan_retention_months(plan: &str) -> Option<i64> {
    if cfg!(feature = "self-hosted") {
        return None;
    }
    match plan {
        "starter" => Some(12),
        // Founder is the Growth plan at a launch price, and Founder & Friends its private free form.
        "growth" | "founder" | "founder_friends" => Some(60),
        // Every account in a hosted build without the private billing module.
        "enterprise" => None,
        // A plan type added without an entry here shouldn't get more than Free.
        _ => Some(3),
    }
}

/// Months an account keeps, given its subscription. A paid plan nobody is
/// paying for, such as one chosen at signup whose checkout was never finished,
/// keeps only what Free does.
pub(crate) fn account_retention_months(
    plan: &str,
    subscription_status: Option<&str>,
) -> Option<i64> {
    if plan_includes_paid_features(plan, subscription_status) {
        plan_retention_months(plan)
    } else {
        plan_retention_months("free")
    }
}

/// The subscription status [`account_retention_months`] reads, as SQL over the
/// `account_subscriptions` row aliased `sub`. A customer's own cancellation
/// runs out the period already paid for, so until then it reads as active.
pub(crate) const RETENTION_STATUS_SQL: &str = "CASE
    WHEN sub.subscription_status = 'canceled' AND sub.current_period_end > NOW() THEN 'active'
    ELSE sub.subscription_status
END";

/// The oldest date inside `months` of retention, as of `today`.
pub(crate) fn oldest_retained_date(months: Option<i64>, today: Date) -> Option<Date> {
    let months: Span = months?.months();
    today.checked_sub(months).ok()
}

/// The first second of the oldest retained day in the project's zone, or
/// `None` when nothing is held back.
pub(crate) fn oldest_retained_second(
    months: Option<i64>,
    timezone: &ProjectTimezone,
    now: i64,
) -> Option<i64> {
    let today = timezone.date_of(now)?;
    timezone.day_start(oldest_retained_date(months, today)?)
}

pub(crate) const RANGE_OLDER_THAN_RETENTION: &str =
    "this date range is older than your plan's data retention";

/// Moves a `[start, end)` range's start up to `oldest`, refusing one that ends
/// before it, so nothing older than the retention is shown.
pub(crate) fn retained_range(
    start: i64,
    end: i64,
    oldest: Option<i64>,
) -> Result<(i64, i64), &'static str> {
    match oldest {
        Some(oldest) if start < oldest => {
            if end <= oldest {
                Err(RANGE_OLDER_THAN_RETENTION)
            } else {
                Ok((oldest, end))
            }
        }
        _ => Ok((start, end)),
    }
}

/// A comparison window the caller chose, moved inside retention like
/// [`retained_range`]. One wholly older shrinks to the retention's first second
/// rather than being refused, since the main range may still be shown.
pub(crate) fn retained_comparison(start: i64, end: i64, oldest: Option<i64>) -> (i64, i64) {
    match oldest {
        Some(oldest) if start < oldest => (oldest, end.max(oldest + 1)),
        _ => (start, end),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    #[test]
    #[cfg(not(feature = "self-hosted"))]
    fn each_plan_keeps_what_the_pricing_page_promises() {
        assert_eq!(plan_retention_months("free"), Some(3));
        assert_eq!(plan_retention_months("starter"), Some(12));
        assert_eq!(plan_retention_months("growth"), Some(60));
        assert_eq!(plan_retention_months("founder"), Some(60));
        assert_eq!(plan_retention_months("founder_friends"), Some(60));
        assert_eq!(plan_retention_months("enterprise"), None);
    }

    #[test]
    #[cfg(not(feature = "self-hosted"))]
    fn an_unknown_plan_gets_the_shortest_retention() {
        assert_eq!(plan_retention_months("brand_new_plan"), Some(3));
    }

    #[test]
    #[cfg(not(feature = "self-hosted"))]
    fn a_paid_plan_keeps_its_retention_only_while_it_is_paid_for() {
        assert_eq!(
            account_retention_months("starter", Some("active")),
            Some(12)
        );
        assert_eq!(
            account_retention_months("starter", Some("trialing")),
            Some(12)
        );
        assert_eq!(
            account_retention_months("growth", Some("past_due")),
            Some(60)
        );
        assert_eq!(account_retention_months("starter", None), Some(3));
        assert_eq!(
            account_retention_months("growth", Some("canceled")),
            Some(3)
        );
        assert_eq!(account_retention_months("growth", Some("unpaid")), Some(3));
        assert_eq!(account_retention_months("free", None), Some(3));
        assert_eq!(account_retention_months("enterprise", None), None);
        // Never taken back for want of payment.
        assert_eq!(
            account_retention_months("founder_friends", Some("unpaid")),
            Some(60)
        );
        assert_eq!(
            account_retention_months("founder_friends", Some("canceled")),
            Some(60)
        );
    }

    #[test]
    #[cfg(feature = "self-hosted")]
    fn self_hosted_keeps_everything_whatever_the_plan() {
        assert_eq!(plan_retention_months("free"), None);
        assert_eq!(account_retention_months("starter", None), None);
    }

    #[test]
    fn month_ends_clamp_rather_than_overflow() {
        assert_eq!(
            oldest_retained_date(Some(6), date(2026, 8, 31)),
            Some(date(2026, 2, 28))
        );
    }

    #[test]
    fn unlimited_retention_has_no_oldest_date() {
        assert_eq!(oldest_retained_date(None, date(2026, 8, 31)), None);
        assert_eq!(
            oldest_retained_second(None, &ProjectTimezone::utc(), 0),
            None
        );
    }

    #[test]
    fn the_oldest_second_is_the_start_of_the_oldest_day() {
        // 2026-11-11T15:00:00Z, three months after 2026-08-11T00:00:00Z.
        assert_eq!(
            oldest_retained_second(Some(3), &ProjectTimezone::utc(), 1_794_409_200),
            Some(1_786_406_400)
        );
    }

    #[test]
    fn a_range_starting_before_retention_starts_at_its_edge() {
        assert_eq!(retained_range(10, 50, Some(20)), Ok((20, 50)));
        assert_eq!(retained_range(30, 50, Some(20)), Ok((30, 50)));
        assert_eq!(retained_range(10, 50, None), Ok((10, 50)));
    }

    #[test]
    fn a_comparison_window_stays_inside_retention() {
        assert_eq!(retained_comparison(10, 50, Some(20)), (20, 50));
        assert_eq!(retained_comparison(10, 15, Some(20)), (20, 21));
        assert_eq!(retained_comparison(30, 50, Some(20)), (30, 50));
        assert_eq!(retained_comparison(10, 15, None), (10, 15));
    }

    #[test]
    fn a_range_wholly_before_retention_is_refused() {
        assert_eq!(
            retained_range(10, 20, Some(20)),
            Err(RANGE_OLDER_THAN_RETENTION)
        );
    }
}
