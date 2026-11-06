use serde::Serialize;

use crate::comparison::Change;
use crate::metrics::conversion_rate;

use super::buckets::OverviewBucket;
use super::revenue::RevenuePeriodTotals;

/// Bucket-summed period quantities (counts and revenue flows). Mirrors the
/// frontend `BucketTotals` so the dashboard can render hover-card breakdowns
/// without re-summing buckets. Previous-period values are normalized to the
/// elapsed length of the current period before serialization.
#[derive(Debug, Default, Clone, Copy, Serialize)]
pub(crate) struct OverviewTotals {
    pub(super) visitors: f64,
    pub(super) imported_visitors: f64,
    pub(super) converting_visitors: f64,
    pub(super) unattributed_converting_visitors: f64,
    pub(super) recurring: f64,
    pub(super) recurring_subscriptions_count: f64,
    pub(super) recurring_annual_subscriptions_count: f64,
    pub(super) new_subscriptions: f64,
    pub(super) new_subscriptions_mrr: f64,
    pub(super) new_subscriptions_count: f64,
    pub(super) new_annual_subscriptions_count: f64,
    pub(super) churn_count: f64,
    pub(super) churn_mrr: f64,
    pub(super) other: f64,
    pub(super) other_payments_count: f64,
    pub(super) refunds: f64,
    pub(super) chargebacks: f64,
    pub(super) session_time_total: f64,
    pub(super) bounced_sessions: f64,
    pub(super) refunds_chargebacks_count: f64,
}

impl OverviewTotals {
    pub(super) fn from_buckets(buckets: &[OverviewBucket]) -> Self {
        let mut totals = Self::default();
        for bucket in buckets {
            totals.visitors += bucket.visitors as f64;
            totals.imported_visitors += bucket.imported_visitors as f64;
            totals.converting_visitors += bucket.converting_visitors as f64;
            totals.unattributed_converting_visitors +=
                bucket.unattributed_converting_visitors as f64;
            totals.recurring += bucket.recurring_subscriptions as f64;
            totals.recurring_subscriptions_count += bucket.recurring_subscriptions_count as f64;
            totals.recurring_annual_subscriptions_count +=
                bucket.recurring_annual_subscriptions_count as f64;
            totals.new_subscriptions += bucket.new_subscriptions as f64;
            totals.new_subscriptions_mrr += bucket.new_subscriptions_mrr as f64;
            totals.new_subscriptions_count += bucket.new_subscriptions_count as f64;
            totals.new_annual_subscriptions_count += bucket.new_annual_subscriptions_count as f64;
            totals.churn_count += bucket.churn as f64;
            totals.churn_mrr += bucket.churn_mrr as f64;
            totals.other += bucket.other_revenue as f64;
            totals.other_payments_count += bucket.other_payments_count as f64;
            totals.refunds += bucket.refunds as f64;
            totals.chargebacks += bucket.chargebacks as f64;
            totals.session_time_total += bucket.session_time_total as f64;
            totals.bounced_sessions += bucket.bounced_sessions as f64;
            totals.refunds_chargebacks_count += bucket.refunds_chargebacks_count as f64;
        }
        totals
    }

    /// The denominator for rates over purchases, which only tracked visitors can make.
    pub(super) fn tracked_visitors(&self) -> f64 {
        self.visitors - self.imported_visitors
    }

    /// Scale every (additive) quantity by `factor`. All fields here are counts
    /// or revenue flows, so they all normalize.
    pub(super) fn scaled(self, factor: f64) -> Self {
        Self {
            visitors: self.visitors * factor,
            imported_visitors: self.imported_visitors * factor,
            converting_visitors: self.converting_visitors * factor,
            unattributed_converting_visitors: self.unattributed_converting_visitors * factor,
            recurring: self.recurring * factor,
            recurring_subscriptions_count: self.recurring_subscriptions_count * factor,
            recurring_annual_subscriptions_count: self.recurring_annual_subscriptions_count
                * factor,
            new_subscriptions: self.new_subscriptions * factor,
            new_subscriptions_mrr: self.new_subscriptions_mrr * factor,
            new_subscriptions_count: self.new_subscriptions_count * factor,
            new_annual_subscriptions_count: self.new_annual_subscriptions_count * factor,
            churn_count: self.churn_count * factor,
            churn_mrr: self.churn_mrr * factor,
            other: self.other * factor,
            other_payments_count: self.other_payments_count * factor,
            refunds: self.refunds * factor,
            chargebacks: self.chargebacks * factor,
            session_time_total: self.session_time_total * factor,
            bounced_sessions: self.bounced_sessions * factor,
            refunds_chargebacks_count: self.refunds_chargebacks_count * factor,
        }
    }
}

/// Display-ready metric tile values for a period. Mirrors the frontend
/// `PeriodMetrics`. Rate fields (`revenue_per_visitor`, `conversion_rate`,
/// `session_time`, `bounce_rate`) are not normalized for the previous period
/// because the normalization factor cancels in the ratio. `mrr` is a
/// point-in-time snapshot and is likewise not normalized.
#[derive(Debug, Default, Clone, Copy, Serialize)]
pub(crate) struct PeriodMetrics {
    pub(super) total_revenue: f64,
    pub(super) mrr: f64,
    pub(super) new_subscriptions_mrr: f64,
    pub(super) churn_mrr: f64,
    pub(super) churn_count: f64,
    pub(super) new_revenue: f64,
    pub(super) visitors: f64,
    pub(super) refunds_and_chargebacks: f64,
    pub(super) revenue_per_visitor: f64,
    pub(super) conversion_rate: f64,
    pub(super) session_time: f64,
    pub(super) bounce_rate: f64,
}

impl PeriodMetrics {
    pub(super) fn from_totals(
        totals: &OverviewTotals,
        revenue: &RevenuePeriodTotals,
        mrr: i64,
    ) -> Self {
        let visitors = totals.visitors;
        let attributed = totals.new_subscriptions + totals.other;
        let share_of = |numerator: f64, denominator: f64| {
            if denominator > 0.0 {
                numerator / denominator
            } else {
                0.0
            }
        };
        let tracked_visitors = totals.tracked_visitors();
        Self {
            total_revenue: revenue.total_revenue as f64,
            mrr: mrr as f64,
            new_subscriptions_mrr: totals.new_subscriptions_mrr,
            churn_mrr: totals.churn_mrr,
            churn_count: totals.churn_count,
            new_revenue: totals.other,
            visitors,
            refunds_and_chargebacks: totals.refunds + totals.chargebacks,
            revenue_per_visitor: share_of(attributed, tracked_visitors),
            conversion_rate: share_of(revenue.converting_visitors as f64, tracked_visitors),
            session_time: share_of(totals.session_time_total, visitors),
            bounce_rate: share_of(totals.bounced_sessions, visitors),
        }
    }

    /// Normalize the previous period's flow metrics to the current period
    /// length. Rates and the MRR snapshot are left untouched.
    pub(super) fn scaled_flows(self, factor: f64) -> Self {
        Self {
            total_revenue: self.total_revenue * factor,
            mrr: self.mrr,
            new_subscriptions_mrr: self.new_subscriptions_mrr * factor,
            churn_mrr: self.churn_mrr * factor,
            churn_count: self.churn_count * factor,
            new_revenue: self.new_revenue * factor,
            visitors: self.visitors * factor,
            refunds_and_chargebacks: self.refunds_and_chargebacks * factor,
            revenue_per_visitor: self.revenue_per_visitor,
            conversion_rate: self.conversion_rate,
            session_time: self.session_time,
            bounce_rate: self.bounce_rate,
        }
    }
}

/// Period-over-period change ratio for each headline metric tile. Computed
/// against the normalized previous period so the comparison is meaningful even
/// when the selected period is shorter than 30 days.
#[derive(Debug, Default, Clone, Copy, Serialize)]
pub(crate) struct MetricChanges {
    pub(super) total_revenue: Change,
    pub(super) mrr: Change,
    pub(super) new_subscriptions_mrr: Change,
    pub(super) churn_mrr: Change,
    pub(super) churn_count: Change,
    pub(super) new_revenue: Change,
    pub(super) visitors: Change,
    pub(super) refunds_and_chargebacks: Change,
    pub(super) revenue_per_visitor: Change,
    pub(super) conversion_rate: Change,
    pub(super) session_time: Change,
    pub(super) bounce_rate: Change,
}

impl MetricChanges {
    /// Both arguments are on the same time base (`previous` already normalized),
    /// so every field is a direct ratio.
    pub(super) fn between(current: &PeriodMetrics, previous: &PeriodMetrics) -> Self {
        let change = |c: f64, p: f64| Change::from_normalized(c, p);
        Self {
            total_revenue: change(current.total_revenue, previous.total_revenue),
            mrr: change(current.mrr, previous.mrr),
            new_subscriptions_mrr: change(
                current.new_subscriptions_mrr,
                previous.new_subscriptions_mrr,
            ),
            churn_mrr: change(current.churn_mrr, previous.churn_mrr),
            churn_count: change(current.churn_count, previous.churn_count),
            new_revenue: change(current.new_revenue, previous.new_revenue),
            visitors: change(current.visitors, previous.visitors),
            refunds_and_chargebacks: change(
                current.refunds_and_chargebacks,
                previous.refunds_and_chargebacks,
            ),
            revenue_per_visitor: change(current.revenue_per_visitor, previous.revenue_per_visitor),
            conversion_rate: change(current.conversion_rate, previous.conversion_rate),
            session_time: change(current.session_time, previous.session_time),
            bounce_rate: change(current.bounce_rate, previous.bounce_rate),
        }
    }
}

/// Period-over-period changes for bucket-summed totals used in metric hover-card
/// breakdowns. Previous values are already normalized to the current period
/// length.
#[derive(Debug, Default, Clone, Copy, Serialize)]
pub(crate) struct OverviewTotalsChanges {
    visitors: Change,
    converting_visitors: Change,
    unattributed_converting_visitors: Change,
    recurring: Change,
    recurring_subscriptions_count: Change,
    recurring_annual_subscriptions_count: Change,
    new_subscriptions: Change,
    new_subscriptions_mrr: Change,
    new_subscriptions_count: Change,
    new_annual_subscriptions_count: Change,
    churn_count: Change,
    churn_mrr: Change,
    other: Change,
    other_payments_count: Change,
    refunds: Change,
    chargebacks: Change,
    session_time_total: Change,
    bounced_sessions: Change,
    refunds_chargebacks_count: Change,
}

impl OverviewTotalsChanges {
    pub(super) fn between(current: &OverviewTotals, previous: &OverviewTotals) -> Self {
        let change = Change::from_normalized;
        let abs_change = |c: f64, p: f64| Change::from_normalized(c.abs(), p.abs());
        Self {
            visitors: change(current.visitors, previous.visitors),
            converting_visitors: change(current.converting_visitors, previous.converting_visitors),
            unattributed_converting_visitors: change(
                current.unattributed_converting_visitors,
                previous.unattributed_converting_visitors,
            ),
            recurring: change(current.recurring, previous.recurring),
            recurring_subscriptions_count: change(
                current.recurring_subscriptions_count,
                previous.recurring_subscriptions_count,
            ),
            recurring_annual_subscriptions_count: change(
                current.recurring_annual_subscriptions_count,
                previous.recurring_annual_subscriptions_count,
            ),
            new_subscriptions: change(current.new_subscriptions, previous.new_subscriptions),
            new_subscriptions_mrr: change(
                current.new_subscriptions_mrr,
                previous.new_subscriptions_mrr,
            ),
            new_subscriptions_count: change(
                current.new_subscriptions_count,
                previous.new_subscriptions_count,
            ),
            new_annual_subscriptions_count: change(
                current.new_annual_subscriptions_count,
                previous.new_annual_subscriptions_count,
            ),
            churn_count: change(current.churn_count, previous.churn_count),
            churn_mrr: change(current.churn_mrr, previous.churn_mrr),
            other: change(current.other, previous.other),
            other_payments_count: change(
                current.other_payments_count,
                previous.other_payments_count,
            ),
            refunds: abs_change(current.refunds, previous.refunds),
            chargebacks: abs_change(current.chargebacks, previous.chargebacks),
            session_time_total: change(current.session_time_total, previous.session_time_total),
            bounced_sessions: change(current.bounced_sessions, previous.bounced_sessions),
            refunds_chargebacks_count: change(
                current.refunds_chargebacks_count,
                previous.refunds_chargebacks_count,
            ),
        }
    }
}

fn gross_revenue_from_totals(totals: &OverviewTotals) -> f64 {
    totals.recurring + totals.new_subscriptions + totals.other
}

fn net_after_refunds_from_totals(totals: &OverviewTotals) -> f64 {
    gross_revenue_from_totals(totals) - totals.refunds.abs() - totals.chargebacks.abs()
}

fn avg_payment_from_totals(totals: &OverviewTotals) -> f64 {
    if totals.other_payments_count > 0.0 {
        totals.other / totals.other_payments_count
    } else {
        0.0
    }
}

fn mrr_div_gross_payment_from_totals(totals: &OverviewTotals) -> f64 {
    if totals.new_subscriptions > 0.0 {
        totals.new_subscriptions_mrr / totals.new_subscriptions
    } else {
        0.0
    }
}

fn avg_mrr_per_churn_from_totals(totals: &OverviewTotals) -> f64 {
    if totals.churn_count > 0.0 {
        totals.churn_mrr / totals.churn_count
    } else {
        0.0
    }
}

fn total_refund_outflows_from_totals(totals: &OverviewTotals) -> f64 {
    totals.refunds.abs() + totals.chargebacks.abs()
}

fn engaged_visitors_from_totals(totals: &OverviewTotals) -> f64 {
    (totals.visitors - totals.bounced_sessions).max(0.0)
}

fn avg_revenue_per_conversion(revenue: &RevenuePeriodTotals) -> f64 {
    if revenue.converting_visitors > 0 {
        revenue.new_revenue as f64 / revenue.converting_visitors as f64
    } else {
        0.0
    }
}

fn derived_change(current: f64, previous: f64) -> Change {
    if current > 0.0 && previous > 0.0 {
        Change::from_normalized(current, previous)
    } else {
        Change::none()
    }
}

/// Period-over-period changes for derived hover-card breakdown lines that are
/// not direct bucket totals or headline metric tiles.
#[derive(Debug, Default, Clone, Copy, Serialize)]
pub(crate) struct OverviewHoverChanges {
    /// Converting visitors: current from payment attribution, previous from
    /// normalized bucket totals (matches the hover-card breakdown sources).
    converting_visitors: Change,
    net_after_refunds: Change,
    avg_payment: Change,
    mrr_div_gross_payment: Change,
    avg_mrr_per_churn: Change,
    total_refund_outflows: Change,
    conversion_share: Change,
    engaged_visitors: Change,
    avg_revenue_per_conversion: Change,
    avg_time_to_purchase: Change,
}

impl OverviewHoverChanges {
    pub(super) fn between(
        current_totals: &OverviewTotals,
        previous_totals: &OverviewTotals,
        current_revenue: &RevenuePeriodTotals,
        previous_revenue: &RevenuePeriodTotals,
    ) -> Self {
        let current_avg_payment = avg_payment_from_totals(current_totals);
        let previous_avg_payment = avg_payment_from_totals(previous_totals);
        let current_mrr_div_gross = mrr_div_gross_payment_from_totals(current_totals);
        let previous_mrr_div_gross = mrr_div_gross_payment_from_totals(previous_totals);
        let current_avg_mrr_per_churn = avg_mrr_per_churn_from_totals(current_totals);
        let previous_avg_mrr_per_churn = avg_mrr_per_churn_from_totals(previous_totals);
        let current_avg_rev_per_conv = avg_revenue_per_conversion(current_revenue);
        let previous_avg_rev_per_conv = avg_revenue_per_conversion(previous_revenue);

        Self {
            converting_visitors: Change::from_normalized(
                current_revenue.converting_visitors as f64,
                previous_totals.converting_visitors,
            ),
            net_after_refunds: Change::from_normalized(
                net_after_refunds_from_totals(current_totals),
                net_after_refunds_from_totals(previous_totals),
            ),
            avg_payment: derived_change(current_avg_payment, previous_avg_payment),
            mrr_div_gross_payment: derived_change(current_mrr_div_gross, previous_mrr_div_gross),
            avg_mrr_per_churn: derived_change(
                current_avg_mrr_per_churn,
                previous_avg_mrr_per_churn,
            ),
            total_refund_outflows: Change::from_normalized(
                total_refund_outflows_from_totals(current_totals),
                total_refund_outflows_from_totals(previous_totals),
            ),
            conversion_share: if current_totals.tracked_visitors() > 0.0
                && previous_totals.tracked_visitors() > 0.0
            {
                Change::rate(
                    conversion_rate(
                        current_revenue.converting_visitors as f64,
                        current_totals.tracked_visitors(),
                    ),
                    conversion_rate(
                        previous_totals.converting_visitors,
                        previous_totals.tracked_visitors(),
                    ),
                )
            } else {
                Change::none()
            },
            engaged_visitors: Change::from_normalized(
                engaged_visitors_from_totals(current_totals),
                engaged_visitors_from_totals(previous_totals),
            ),
            avg_revenue_per_conversion: derived_change(
                current_avg_rev_per_conv,
                previous_avg_rev_per_conv,
            ),
            avg_time_to_purchase: if current_revenue.avg_time_to_purchase > 0.0
                && previous_revenue.avg_time_to_purchase > 0.0
            {
                Change::rate(
                    current_revenue.avg_time_to_purchase,
                    previous_revenue.avg_time_to_purchase,
                )
            } else {
                Change::none()
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn purchase_rates_ignore_imported_visitors_but_engagement_rates_count_them() {
        let totals = OverviewTotals {
            visitors: 10_000.0,
            imported_visitors: 9_000.0,
            bounced_sessions: 5_000.0,
            session_time_total: 100_000.0,
            new_subscriptions: 2_000.0,
            ..OverviewTotals::default()
        };
        let mut revenue = RevenuePeriodTotals::default();
        revenue.converting_visitors = 10;
        let metrics = PeriodMetrics::from_totals(&totals, &revenue, 0);
        assert_eq!(metrics.visitors, 10_000.0);
        assert_eq!(metrics.conversion_rate, 0.01);
        assert_eq!(metrics.revenue_per_visitor, 2.0);
        assert_eq!(metrics.bounce_rate, 0.5);
        assert_eq!(metrics.session_time, 10.0);
    }

    #[test]
    fn days_with_only_imported_visitors_have_no_purchase_rates() {
        let totals = OverviewTotals {
            visitors: 100.0,
            imported_visitors: 100.0,
            ..OverviewTotals::default()
        };
        let metrics = PeriodMetrics::from_totals(&totals, &RevenuePeriodTotals::default(), 0);
        assert_eq!(metrics.conversion_rate, 0.0);
        assert_eq!(metrics.revenue_per_visitor, 0.0);
    }
}
