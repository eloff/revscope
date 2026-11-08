use std::collections::BTreeMap;

use rusqlite::params_from_iter;
use rusqlite::types::{FromSql, FromSqlResult, ValueRef};
use serde::Serialize;
use smallvec::SmallVec;

use crate::auth::{AuthError, map_row_error};
use crate::comparison;
use crate::db::{ProjectConnection, SqlCompactStr};
use crate::segment_filter::SegmentFilters;
use crate::timezone::Buckets;
use crate::untrusted_keys::UntrustedKeySet;

use super::buckets::{NANOS_PER_SECOND, OverviewBucket};

#[derive(Debug, Default, Serialize)]
pub(crate) struct RevenuePeriodTotals {
    /// Gross payment revenue (recurring + new subscriptions + other).
    pub(crate) total_revenue: i64,
    /// Gross payment revenue from new subscriptions and non-subscription payments.
    pub(crate) new_revenue: i64,
    /// Distinct buyers with a first purchase in the period, deduplicated by landing or customer.
    pub(crate) converting_visitors: u32,
    /// Average seconds from landing to purchase for attributed first purchases.
    pub(crate) avg_time_to_purchase: f64,
    #[serde(skip)]
    time_to_purchase_total_seconds: i64,
    #[serde(skip)]
    time_to_purchase_count: u32,
}

impl RevenuePeriodTotals {
    fn record_time_to_purchase(&mut self, seconds: i64) {
        self.time_to_purchase_total_seconds += seconds;
        self.time_to_purchase_count = self.time_to_purchase_count.saturating_add(1);
    }

    fn finalize_time_to_purchase(&mut self) {
        self.avg_time_to_purchase = if self.time_to_purchase_count == 0 {
            0.0
        } else {
            self.time_to_purchase_total_seconds as f64 / self.time_to_purchase_count as f64
        };
    }
}

#[derive(Debug)]
pub(super) struct PopulatedRevenueMetrics {
    pub(super) current: RevenuePeriodTotals,
    pub(super) previous: RevenuePeriodTotals,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CustomerId(SmallVec<[u8; 32]>);

impl FromSql for CustomerId {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        match value {
            ValueRef::Integer(value) => Ok(Self(SmallVec::from_slice(&value.to_be_bytes()))),
            ValueRef::Blob(value) => Ok(Self(SmallVec::from_slice(value))),
            ValueRef::Text(value) => Ok(Self(SmallVec::from_slice(value))),
            _ => value
                .as_str()
                .map(|text| Self(SmallVec::from_slice(text.as_bytes()))),
        }
    }
}

/// Revenue totals for one comparison window, with the buyers already counted
/// as converting in it.
#[derive(Debug, Default)]
struct PeriodRevenue {
    totals: RevenuePeriodTotals,
    /// A converting buyer is identified by their attributed landing when
    /// available, otherwise by their customer key (`provider_customer_id`,
    /// `customer_email`, or `order_email`).
    buyers: UntrustedKeySet<CustomerId>,
}

impl PeriodRevenue {
    /// Counts a first purchase toward converting visitors, returning whether
    /// it was this buyer's first in the period. A purchase with no identity
    /// always counts, since nothing can match it to an earlier one.
    fn record_buyer(&mut self, customer_id: Option<&CustomerId>) -> bool {
        let first = customer_id.is_none_or(|id| self.buyers.insert(id.clone()));
        if first {
            self.totals.converting_visitors = self.totals.converting_visitors.saturating_add(1);
        }
        first
    }
}

/// How a payment counts, as classified by the query in [`populate_revenue_metrics`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaymentCategory {
    /// A subscription payment after the subscription's first real charge.
    Recurring,
    NewSubscription,
    Refund,
    Chargeback,
    /// A single (non-subscription) payment, or a payment type this query does
    /// not recognize, which likewise counts as new revenue.
    Other,
}

impl PaymentCategory {
    fn from_sql(category: &str) -> Self {
        match category {
            "recurring" => Self::Recurring,
            "new_subscription" => Self::NewSubscription,
            "refund" => Self::Refund,
            "chargeback" => Self::Chargeback,
            _ => Self::Other,
        }
    }
}

impl OverviewBucket {
    fn add_payment(&mut self, category: PaymentCategory, amount: i64, mrr: i64, annual: bool) {
        match category {
            PaymentCategory::Recurring => {
                self.recurring_subscriptions += amount;
                self.recurring_subscriptions_count += 1;
                if annual {
                    self.recurring_annual_subscriptions_count += 1;
                }
            }
            PaymentCategory::NewSubscription => {
                self.new_subscriptions += amount;
                self.new_subscriptions_mrr += mrr;
                self.new_subscriptions_count += 1;
                if annual {
                    self.new_annual_subscriptions_count += 1;
                }
            }
            PaymentCategory::Refund => {
                self.refunds += amount;
                self.refunds_chargebacks_count += 1;
            }
            PaymentCategory::Chargeback => {
                self.chargebacks += amount;
                self.refunds_chargebacks_count += 1;
            }
            PaymentCategory::Other => {
                self.other_revenue += amount;
                self.other_payments_count += 1;
            }
        }
    }
}

/// Adds every settled payment from the start of the comparison window to
/// `period_end` into `buckets`, and returns revenue totals for the selected
/// period and the comparison window.
///
/// Rows are classified in SQL and aggregated here. A country filter restricts
/// revenue to payments whose attributed visitor landed from the selected countries.
pub(super) fn populate_revenue_metrics(
    conn: &ProjectConnection,
    buckets: &mut BTreeMap<i64, OverviewBucket>,
    bucket_bounds: &Buckets,
    period_start: i64,
    period_end: i64,
    segments: &SegmentFilters,
) -> Result<PopulatedRevenueMetrics, AuthError> {
    let previous_period_start = comparison::previous_period_start(period_start);
    let mut current = PeriodRevenue::default();
    let mut previous = PeriodRevenue::default();
    let country_filter =
        segments.country_exists_clause("payments.attributed_visitor_timestamp_id", 3);
    let session_filter =
        segments.session_filter_clause("payments.attributed_visitor_timestamp_id", None);
    let mut sql_params: Vec<&dyn rusqlite::types::ToSql> =
        vec![&previous_period_start, &period_end];
    segments.append_country_params(&mut sql_params);
    conn.with_query(
        &format!(
            "SELECT payments.paid_at,
                payments.amount,
                COALESCE((
                    SELECT h.mrr_amount
                    FROM subscription_history h
                    WHERE h.subscription_id = payments.subscription_id
                      AND h.effective_from <= payments.paid_at
                      AND h.mrr_amount IS NOT NULL
                    ORDER BY h.effective_from DESC
                    LIMIT 1
                ), payments.mrr_amount) AS mrr_amount,
                payments.attributed_visitor_timestamp_id,
                COALESCE(payments.attributed_visitor_timestamp_id,
                         payments.provider_customer_id,
                         payments.customer_email,
                         payments.order_email) AS customer_id,
                payments.interval_unit,
                CASE
                    WHEN payments.payment_type = 'single' THEN 'other'
                    WHEN payments.payment_type = 'refund' THEN 'refund'
                    WHEN payments.payment_type = 'chargeback' THEN 'chargeback'
                    WHEN payments.payment_type = 'subscription' AND EXISTS (
                        SELECT 1
                        FROM payments prior
                        WHERE prior.subscription_id = payments.subscription_id
                          AND prior.status = 'ok'
                          AND prior.payment_type = 'subscription'
                          AND prior.amount > 0
                          AND prior.paid_at < payments.paid_at
                          AND prior.id != payments.id
                    ) THEN 'recurring'
                    WHEN payments.payment_type = 'subscription' THEN 'new_subscription'
                    ELSE 'other'
                END AS category
         FROM payments
         WHERE payments.status = 'ok'
           -- A free trial opens with a settled invoice for nothing. It buys no
           -- revenue and no MRR, so the first real charge is the purchase.
           AND NOT (payments.payment_type = 'subscription' AND payments.amount = 0)
           AND payments.paid_at >= ?1
           AND payments.paid_at < ?2{country_filter}{session_filter}"
        ),
        params_from_iter(sql_params),
        |mut rows| {
            while let Some(row) = rows.next()? {
                let paid_at: i64 = row.get(0)?;
                let amount: i64 = row.get(1)?;
                let mrr_amount: i64 = row.get::<_, Option<i64>>(2)?.unwrap_or(0);
                let visitor_timestamp_id: Option<i64> = row.get(3)?;
                let customer_id: Option<CustomerId> = row.get(4)?;
                let interval_unit: Option<SqlCompactStr> = row.get(5)?;
                let category = PaymentCategory::from_sql(row.get::<_, SqlCompactStr>(6)?.as_str());
                let annual = interval_unit.as_ref().is_some_and(|unit| unit == &"year");

                let mut bucket = bucket_bounds
                    .bucket_for_second(paid_at)
                    .and_then(|bucket| buckets.get_mut(&bucket));
                if let Some(bucket) = bucket.as_deref_mut() {
                    bucket.add_payment(category, amount, mrr_amount, annual);
                }

                let period = if paid_at >= period_start {
                    &mut current
                } else if paid_at >= previous_period_start {
                    &mut previous
                } else {
                    continue;
                };
                match category {
                    PaymentCategory::Recurring => period.totals.total_revenue += amount,
                    PaymentCategory::NewSubscription | PaymentCategory::Other => {
                        period.totals.total_revenue += amount;
                        period.totals.new_revenue += amount;
                        if period.record_buyer(customer_id.as_ref())
                            && let Some(bucket) = bucket
                        {
                            bucket.converting_visitors += 1;
                            if customer_id.is_none() {
                                bucket.unattributed_converting_visitors += 1;
                            }
                        }
                        if let Some(visitor_timestamp_id) = visitor_timestamp_id {
                            let seconds =
                                (paid_at - visitor_timestamp_id / NANOS_PER_SECOND).max(0);
                            period.totals.record_time_to_purchase(seconds);
                        }
                    }
                    PaymentCategory::Refund | PaymentCategory::Chargeback => {}
                }
            }
            Ok(())
        },
    )
    .map_err(map_row_error)?;

    current.totals.finalize_time_to_purchase();
    previous.totals.finalize_time_to_purchase();
    Ok(PopulatedRevenueMetrics {
        current: current.totals,
        previous: previous.totals,
    })
}
