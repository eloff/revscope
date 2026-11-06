//! Upserting payments, refunds and chargebacks.

use rusqlite::{OptionalExtension, params};

use crate::db::ProjectConnection;
use crate::integration::attribution;
use crate::integration::sync::{SyncCounts, SyncResult};
use crate::metrics::monthly_amount;
use crate::util::non_empty;

use super::count_upsert;

/// What a `payments` row represents. Refunds and chargebacks are stored with a
/// negative amount, which [`upsert_payment`] enforces.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::integration) enum PaymentType {
    Single,
    Subscription,
    Refund,
    Chargeback,
}

impl PaymentType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Subscription => "subscription",
            Self::Refund => "refund",
            Self::Chargeback => "chargeback",
        }
    }

    fn is_negative(self) -> bool {
        matches!(self, Self::Refund | Self::Chargeback)
    }
}

/// Normalized payment outcome. Provider-native status strings are kept
/// separately in `payments.provider_status`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::integration) enum PaymentStatus {
    Pending,
    Ok,
    Failed,
    Canceled,
}

impl PaymentStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Ok => "ok",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
        }
    }
}

/// The billing interval of the subscription a payment belongs to, denormalized
/// onto the payment so MRR can be derived without a join.
#[derive(Clone, Debug)]
pub(in crate::integration) struct PaymentInterval {
    pub(in crate::integration) unit: String,
    pub(in crate::integration) count: i64,
}

/// A payment, refund, or chargeback as reported by a provider.
#[derive(Clone, Debug)]
pub(in crate::integration) struct PaymentUpsert<'a> {
    pub(in crate::integration) provider: &'a str,
    pub(in crate::integration) provider_payment_id: &'a str,
    pub(in crate::integration) provider_object_id: Option<i64>,
    pub(in crate::integration) provider_object_type: Option<&'a str>,
    pub(in crate::integration) subscription_id: Option<i64>,
    pub(in crate::integration) interval: Option<PaymentInterval>,
    pub(in crate::integration) provider_status: Option<&'a str>,
    pub(in crate::integration) provider_order_id: Option<&'a str>,
    pub(in crate::integration) provider_checkout_id: Option<&'a str>,
    pub(in crate::integration) provider_invoice_id: Option<&'a str>,
    pub(in crate::integration) provider_refund_id: Option<&'a str>,
    pub(in crate::integration) provider_dispute_id: Option<&'a str>,
    pub(in crate::integration) parent_provider_payment_id: Option<&'a str>,
    pub(in crate::integration) provider_customer_id: Option<&'a str>,
    pub(in crate::integration) customer_email: Option<&'a str>,
    pub(in crate::integration) order_email: Option<&'a str>,
    pub(in crate::integration) payment_type: PaymentType,
    pub(in crate::integration) status: PaymentStatus,
    /// Integer thousandths of the currency unit. The sign is normalized from
    /// `payment_type`, so callers can pass the provider's magnitude as-is.
    pub(in crate::integration) amount: i64,
    pub(in crate::integration) currency: &'a str,
    pub(in crate::integration) fee_amount: Option<i64>,
    pub(in crate::integration) net_amount: Option<i64>,
    pub(in crate::integration) settlement_currency: Option<&'a str>,
    pub(in crate::integration) settlement_amount: Option<i64>,
    pub(in crate::integration) exchange_rate: Option<f64>,
    pub(in crate::integration) paid_at: i64,
    pub(in crate::integration) created_at: Option<i64>,
    pub(in crate::integration) metadata: Option<&'a str>,
}

impl<'a> PaymentUpsert<'a> {
    pub(in crate::integration) fn new(
        provider: &'a str,
        provider_payment_id: &'a str,
        payment_type: PaymentType,
        status: PaymentStatus,
        amount: i64,
        currency: &'a str,
        paid_at: i64,
    ) -> Self {
        Self {
            provider,
            provider_payment_id,
            provider_object_id: None,
            provider_object_type: None,
            subscription_id: None,
            interval: None,
            provider_status: None,
            provider_order_id: None,
            provider_checkout_id: None,
            provider_invoice_id: None,
            provider_refund_id: None,
            provider_dispute_id: None,
            parent_provider_payment_id: None,
            provider_customer_id: None,
            customer_email: None,
            order_email: None,
            payment_type,
            status,
            amount,
            currency,
            fee_amount: None,
            net_amount: None,
            settlement_currency: None,
            settlement_amount: None,
            exchange_rate: None,
            paid_at,
            created_at: None,
            metadata: None,
        }
    }

    /// Signed to match `payment_type`, satisfying the `payments` amount check.
    fn signed_amount(&self) -> i64 {
        signed(self.amount, self.payment_type)
    }

    /// The amount check requires a strictly negative value on refunds and
    /// chargebacks, so a zero-value one can't be stored at all.
    fn is_storable(&self) -> bool {
        !self.payment_type.is_negative() || self.signed_amount() < 0
    }

    fn signed_net_amount(&self) -> Option<i64> {
        self.net_amount
            .map(|amount| signed(amount, self.payment_type))
            .filter(|amount| !self.payment_type.is_negative() || *amount < 0)
    }

    fn signed_settlement_amount(&self) -> Option<i64> {
        self.settlement_amount
            .map(|amount| signed(amount, self.payment_type))
    }

    /// Only recurring charges contribute to MRR, and only when we know the
    /// billing interval to normalize them by.
    fn mrr_amount(&self) -> Option<i64> {
        if self.payment_type != PaymentType::Subscription {
            return None;
        }
        let interval = self.interval.as_ref()?;
        monthly_amount(self.signed_amount(), &interval.unit, interval.count)
    }
}

pub(super) fn signed(amount: i64, payment_type: PaymentType) -> i64 {
    if payment_type.is_negative() {
        -amount.abs()
    } else {
        amount.max(0)
    }
}

/// Upserts a `payments` row keyed on `(provider, provider_payment_id)`.
///
/// Zero-value refunds and chargebacks are skipped rather than written, since the
/// schema requires them to carry a negative amount.
pub(in crate::integration) fn upsert_payment(
    conn: &ProjectConnection,
    payment: &PaymentUpsert<'_>,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    if !payment.is_storable() {
        return Ok(());
    }
    let existing = payment_exists(conn, payment.provider, payment.provider_payment_id)?;
    let (interval_unit, interval_count) = match payment.interval.as_ref() {
        Some(interval) => (Some(interval.unit.as_str()), Some(interval.count.max(1))),
        None => (None, None),
    };

    conn.execute(
        "INSERT INTO payments (
            provider, provider_payment_id, provider_object_id, provider_object_type,
            subscription_id, interval_unit, interval_count,
            provider_status, provider_order_id, provider_checkout_id, provider_invoice_id,
            provider_refund_id, provider_dispute_id, parent_provider_payment_id,
            provider_customer_id, customer_email, order_email,
            payment_type, status, amount, mrr_amount, currency,
            fee_amount, net_amount, settlement_currency, settlement_amount, exchange_rate,
            paid_at, created_at, metadata
         ) VALUES (
            ?1, ?2, ?3, ?4,
            ?5, ?6, ?7,
            ?8, ?9, ?10, ?11,
            ?12, ?13, ?14,
            ?15, ?16, ?17,
            ?18, ?19, ?20, ?21, ?22,
            ?23, ?24, ?25, ?26, ?27,
            ?28, ?29, ?30
         )
         ON CONFLICT(provider, provider_payment_id) DO UPDATE SET
            provider_object_id = coalesce(excluded.provider_object_id, provider_object_id),
            provider_object_type = excluded.provider_object_type,
            subscription_id = coalesce(excluded.subscription_id, subscription_id),
            interval_unit = coalesce(excluded.interval_unit, interval_unit),
            interval_count = coalesce(excluded.interval_count, interval_count),
            provider_status = excluded.provider_status,
            provider_order_id = coalesce(excluded.provider_order_id, provider_order_id),
            provider_checkout_id = coalesce(excluded.provider_checkout_id, provider_checkout_id),
            provider_invoice_id = coalesce(excluded.provider_invoice_id, provider_invoice_id),
            provider_refund_id = coalesce(excluded.provider_refund_id, provider_refund_id),
            provider_dispute_id = coalesce(excluded.provider_dispute_id, provider_dispute_id),
            parent_provider_payment_id = coalesce(
                excluded.parent_provider_payment_id, parent_provider_payment_id
            ),
            provider_customer_id = coalesce(excluded.provider_customer_id, provider_customer_id),
            customer_email = coalesce(excluded.customer_email, customer_email),
            order_email = coalesce(excluded.order_email, order_email),
            payment_type = excluded.payment_type,
            status = excluded.status,
            amount = excluded.amount,
            mrr_amount = coalesce(excluded.mrr_amount, mrr_amount),
            currency = excluded.currency,
            fee_amount = coalesce(excluded.fee_amount, fee_amount),
            net_amount = coalesce(excluded.net_amount, net_amount),
            settlement_currency = coalesce(excluded.settlement_currency, settlement_currency),
            settlement_amount = coalesce(excluded.settlement_amount, settlement_amount),
            exchange_rate = coalesce(excluded.exchange_rate, exchange_rate),
            paid_at = excluded.paid_at,
            created_at = excluded.created_at,
            metadata = excluded.metadata",
        params![
            payment.provider,
            payment.provider_payment_id,
            payment.provider_object_id,
            payment.provider_object_type,
            payment.subscription_id,
            interval_unit,
            interval_count,
            non_empty(payment.provider_status),
            non_empty(payment.provider_order_id),
            non_empty(payment.provider_checkout_id),
            non_empty(payment.provider_invoice_id),
            non_empty(payment.provider_refund_id),
            non_empty(payment.provider_dispute_id),
            non_empty(payment.parent_provider_payment_id),
            non_empty(payment.provider_customer_id),
            non_empty(payment.customer_email),
            non_empty(payment.order_email),
            payment.payment_type.as_str(),
            payment.status.as_str(),
            payment.signed_amount(),
            payment.mrr_amount(),
            payment.currency,
            payment.fee_amount,
            payment.signed_net_amount(),
            non_empty(payment.settlement_currency),
            payment.signed_settlement_amount(),
            payment.exchange_rate,
            payment.paid_at,
            payment.created_at.unwrap_or(payment.paid_at),
            payment.metadata,
        ],
    )?;

    if let Some(subscription_id) = payment.subscription_id {
        attribution::propagate_linked_subscription_payment_fields(conn, subscription_id)?;
    }
    count_upsert(existing, counts);
    Ok(())
}

pub(in crate::integration) fn payment_exists(
    conn: &ProjectConnection,
    provider: &str,
    provider_payment_id: &str,
) -> SyncResult<bool> {
    Ok(conn
        .query_row(
            "SELECT 1
             FROM payments
             WHERE provider = ?1 AND provider_payment_id = ?2",
            params![provider, provider_payment_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}
