//! Storing Stripe objects as RevScope payments and subscriptions.

use crate::db::ProjectConnection;
use crate::integration::store::{
    PaymentStatus, PaymentType, PaymentUpsert, PlanInterval, Reported, SubscriptionPlan,
    SubscriptionUpsert, minor_units_to_thousandths, subscription_interval, subscription_row_id,
    upsert_payment, upsert_subscription,
};
use crate::integration::sync::{SyncCounts, SyncResult, serialize_metadata};
use crate::untrusted_keys::UntrustedKeyMap;

use super::PROVIDER;
use super::plan::{CouponCache, PaymentIntentMetadataCache, subscription_plan};

pub(super) fn store_subscription(
    conn: &ProjectConnection,
    provider_object_id: i64,
    subscription: &stripe_billing::Subscription,
    coupons: &CouponCache,
    counts: &mut SyncCounts,
) -> SyncResult<i64> {
    let provider_subscription_id = subscription.id.to_string();
    let plan = subscription_plan(subscription, coupons);
    let customer_id = expandable_id(&subscription.customer);
    let currency = currency_code(&subscription.currency);
    let status = subscription.status.to_string();
    let metadata = serialize_metadata(&subscription.metadata)?;

    let mut upsert = SubscriptionUpsert::new(PROVIDER, &provider_subscription_id);
    upsert.provider_object_id = Some(provider_object_id);
    upsert.provider_customer_id = Some(&customer_id);
    upsert.status = Some(&status);
    upsert.plan = SubscriptionPlan {
        interval: plan
            .interval
            .map(|(unit, count)| PlanInterval::new(unit, count)),
        provider_price_id: plan.provider_price_id.as_deref(),
        plan_name: None,
        quantity: plan.quantity,
        currency: Some(&currency),
        amount: plan.amount,
    };
    upsert.current_period_start = plan.current_period_start;
    upsert.current_period_end = plan.current_period_end;
    upsert.started_at = Some(subscription.start_date);
    upsert.ended_at = Reported::or_cleared(subscription.ended_at);
    upsert.canceled_at = Reported::or_cleared(subscription.canceled_at);
    // `canceled_at` is when the cancellation was requested, so a subscription
    // canceled at period end keeps billing until `cancel_at`. Stripe fills that
    // in with the period end when `cancel_at_period_end` is set, but it only
    // started doing so recently, so the period end is used directly when it is
    // missing.
    upsert.cancel_at = Reported::or_cleared(subscription.cancel_at.or(
        if subscription.cancel_at_period_end {
            plan.current_period_end
        } else {
            None
        },
    ));
    upsert.metadata = metadata.as_deref();
    upsert_subscription(conn, &upsert, counts)
}

pub(super) fn store_charge_payment(
    conn: &ProjectConnection,
    provider_object_id: i64,
    charge: &stripe_core::Charge,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    let provider_payment_id = charge_payment_id(charge);
    let balance_transaction = charge
        .balance_transaction
        .as_ref()
        .and_then(|value| value.as_object());
    let settlement_currency = balance_transaction.map(|tx| currency_code(&tx.currency));
    let payment_intent = charge.payment_intent.as_ref().map(expandable_id);
    let customer_id = charge.customer.as_ref().map(expandable_id);
    let metadata = payment_metadata(
        Some(&charge.metadata),
        expanded_payment_intent_metadata(charge.payment_intent.as_ref()),
    )?;
    let currency = currency_code(&charge.currency);
    let status = if charge.paid {
        PaymentStatus::Ok
    } else {
        match charge.status.to_string().as_str() {
            "failed" => PaymentStatus::Failed,
            _ => PaymentStatus::Pending,
        }
    };

    let mut upsert = PaymentUpsert::new(
        PROVIDER,
        &provider_payment_id,
        PaymentType::Single,
        status,
        stripe_amount_to_thousandths(charge.amount, &charge.currency),
        &currency,
        charge.created,
    );
    upsert.provider_object_id = Some(provider_object_id);
    upsert.provider_object_type = Some("charge");
    upsert.provider_invoice_id = payment_intent.as_deref();
    upsert.provider_customer_id = customer_id.as_deref();
    upsert.customer_email = charge.billing_details.email.as_deref();
    upsert.fee_amount =
        balance_transaction.map(|tx| stripe_amount_to_thousandths(tx.fee, &tx.currency));
    upsert.net_amount =
        balance_transaction.map(|tx| stripe_amount_to_thousandths(tx.net, &tx.currency));
    upsert.settlement_currency = settlement_currency.as_deref();
    upsert.settlement_amount =
        balance_transaction.map(|tx| stripe_amount_to_thousandths(tx.amount, &tx.currency));
    upsert.exchange_rate = balance_transaction.and_then(|tx| tx.exchange_rate);
    upsert.metadata = metadata.as_deref();
    upsert_payment(conn, &upsert, counts)
}

pub(super) fn store_invoice_payment(
    conn: &ProjectConnection,
    provider_object_id: i64,
    invoice_id: &str,
    invoice: &stripe_billing::Invoice,
    payment_intents: &PaymentIntentMetadataCache,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    let Some(subscription_id) = invoice_subscription_row_id(conn, invoice)? else {
        return Ok(());
    };
    let provider_payment_id = invoice_payment_id(invoice_id, invoice);
    let customer_id = invoice.customer.as_ref().map(expandable_id);
    let metadata = payment_metadata(
        invoice.metadata.as_ref(),
        invoice_payment_intent_metadata(invoice, payment_intents),
    )?;
    let currency = currency_code(&invoice.currency);
    let paid_at = invoice
        .status_transitions
        .paid_at
        .unwrap_or(invoice.effective_at.unwrap_or(invoice.created));

    let mut upsert = PaymentUpsert::new(
        PROVIDER,
        &provider_payment_id,
        PaymentType::Subscription,
        invoice_payment_status(invoice),
        stripe_amount_to_thousandths(invoice.amount_paid.max(0), &invoice.currency),
        &currency,
        paid_at,
    );
    upsert.provider_object_id = Some(provider_object_id);
    upsert.provider_object_type = Some("invoice");
    upsert.subscription_id = Some(subscription_id);
    upsert.interval = subscription_interval(conn, subscription_id)?;
    upsert.provider_invoice_id = Some(invoice_id);
    upsert.provider_customer_id = customer_id.as_deref();
    upsert.customer_email = invoice.customer_email.as_deref();
    upsert.created_at = Some(invoice.created);
    upsert.metadata = metadata.as_deref();
    upsert_payment(conn, &upsert, counts)
}

pub(super) fn store_refund_payment(
    conn: &ProjectConnection,
    provider_object_id: i64,
    refund: &stripe_core::Refund,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    let provider_payment_id = refund.id.to_string();
    let parent_payment_id = refund_parent_payment_id(refund);
    let metadata = payment_metadata(
        refund.metadata.as_ref(),
        expanded_payment_intent_metadata(refund.payment_intent.as_ref()),
    )?;
    let currency = currency_code(&refund.currency);
    let status = match refund.status.as_deref() {
        Some("succeeded") => PaymentStatus::Ok,
        Some("failed") => PaymentStatus::Failed,
        Some("canceled") => PaymentStatus::Canceled,
        _ => PaymentStatus::Pending,
    };

    let mut upsert = PaymentUpsert::new(
        PROVIDER,
        &provider_payment_id,
        PaymentType::Refund,
        status,
        stripe_amount_to_thousandths(refund.amount, &refund.currency),
        &currency,
        refund.created,
    );
    upsert.provider_object_id = Some(provider_object_id);
    upsert.provider_object_type = Some("refund");
    upsert.provider_refund_id = Some(&provider_payment_id);
    upsert.parent_provider_payment_id = parent_payment_id.as_deref();
    upsert.metadata = metadata.as_deref();
    upsert_payment(conn, &upsert, counts)
}

pub(super) fn store_dispute_payment(
    conn: &ProjectConnection,
    provider_object_id: i64,
    dispute: &stripe_core::Dispute,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    let provider_payment_id = dispute.id.to_string();
    let parent_payment_id = dispute
        .payment_intent
        .as_ref()
        .map(expandable_id)
        .unwrap_or_else(|| expandable_id(&dispute.charge));
    let metadata = payment_metadata(
        Some(&dispute.metadata),
        expanded_payment_intent_metadata(dispute.payment_intent.as_ref()),
    )?;
    let currency = currency_code(&dispute.currency);
    let status = match dispute.status.to_string().as_str() {
        "lost" => PaymentStatus::Ok,
        "won" | "prevented" => PaymentStatus::Canceled,
        _ => PaymentStatus::Pending,
    };

    let mut upsert = PaymentUpsert::new(
        PROVIDER,
        &provider_payment_id,
        PaymentType::Chargeback,
        status,
        stripe_amount_to_thousandths(dispute.amount, &dispute.currency),
        &currency,
        dispute.created,
    );
    upsert.provider_object_id = Some(provider_object_id);
    upsert.provider_object_type = Some("dispute");
    upsert.provider_dispute_id = Some(&provider_payment_id);
    upsert.parent_provider_payment_id = Some(&parent_payment_id);
    upsert.metadata = metadata.as_deref();
    upsert_payment(conn, &upsert, counts)
}

pub(super) fn invoice_subscription_row_id(
    conn: &ProjectConnection,
    invoice: &stripe_billing::Invoice,
) -> SyncResult<Option<i64>> {
    let Some(subscription_id) = invoice.subscription.as_ref().map(expandable_id) else {
        return Ok(None);
    };
    subscription_row_id(conn, PROVIDER, &subscription_id)
}

pub(super) fn charge_payment_id(charge: &stripe_core::Charge) -> String {
    charge
        .payment_intent
        .as_ref()
        .map(expandable_id)
        .unwrap_or_else(|| charge.id.to_string())
}

pub(super) fn invoice_payment_id(invoice_id: &str, invoice: &stripe_billing::Invoice) -> String {
    invoice
        .payments
        .as_ref()
        .and_then(|payments| payments.data.first())
        .and_then(|payment| {
            payment
                .payment
                .payment_intent
                .as_ref()
                .map(expandable_id)
                .or_else(|| payment.payment.charge.as_ref().map(expandable_id))
        })
        .unwrap_or_else(|| invoice_id.to_string())
}

pub(super) fn invoice_payment_intent(
    invoice: &stripe_billing::Invoice,
) -> Option<&stripe_types::Expandable<stripe_shared::PaymentIntent>> {
    invoice
        .payments
        .as_ref()
        .and_then(|payments| payments.data.first())
        .and_then(|payment| payment.payment.payment_intent.as_ref())
}

/// Metadata from an expanded payment intent, when the invoice list returned
/// the object rather than its id.
pub(super) fn expanded_payment_intent_metadata(
    payment_intent: Option<&stripe_types::Expandable<stripe_shared::PaymentIntent>>,
) -> Option<&UntrustedKeyMap<String, String>> {
    payment_intent
        .and_then(|payment_intent| payment_intent.as_object())
        .map(|payment_intent| &payment_intent.metadata)
}

/// The payment intent metadata to merge onto an invoice payment: the expanded
/// object when the invoice carried one, and the cache otherwise.
pub(super) fn invoice_payment_intent_metadata<'a>(
    invoice: &'a stripe_billing::Invoice,
    payment_intents: &'a PaymentIntentMetadataCache,
) -> Option<&'a UntrustedKeyMap<String, String>> {
    let payment_intent = invoice_payment_intent(invoice)?;
    if let Some(metadata) = expanded_payment_intent_metadata(Some(payment_intent)) {
        return Some(metadata);
    }
    payment_intents
        .get(payment_intent.id().as_str())
        .and_then(Option::as_ref)
}

pub(super) fn payment_metadata(
    primary: Option<&UntrustedKeyMap<String, String>>,
    payment_intent_metadata: Option<&UntrustedKeyMap<String, String>>,
) -> SyncResult<Option<String>> {
    match (primary, payment_intent_metadata) {
        (None, None) => Ok(None),
        (Some(metadata), None) | (None, Some(metadata)) => serialize_metadata(metadata),
        (Some(primary), Some(payment_intent_metadata)) => {
            let mut merged = payment_intent_metadata.clone();
            merged.extend(
                primary
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone())),
            );
            serialize_metadata(&merged)
        }
    }
}

pub(super) fn refund_parent_payment_id(refund: &stripe_core::Refund) -> Option<String> {
    refund
        .payment_intent
        .as_ref()
        .map(expandable_id)
        .or_else(|| refund.charge.as_ref().map(expandable_id))
}

pub(super) fn invoice_payment_status(invoice: &stripe_billing::Invoice) -> PaymentStatus {
    match invoice.status.as_ref().map(ToString::to_string).as_deref() {
        Some("paid") => PaymentStatus::Ok,
        Some("void") => PaymentStatus::Canceled,
        Some("uncollectible") => PaymentStatus::Failed,
        Some("open") | Some("draft") => PaymentStatus::Pending,
        _ if invoice.amount_paid > 0 => PaymentStatus::Ok,
        _ => PaymentStatus::Pending,
    }
}

pub(super) fn expandable_id<T>(value: &stripe_types::Expandable<T>) -> String
where
    T: stripe_types::Object,
    T::Id: ToString,
{
    value.id().to_string()
}

pub(super) fn expandable_invoice_id(
    value: &stripe_types::Expandable<stripe_shared::Invoice>,
) -> Option<String> {
    value.id().as_ref().map(ToString::to_string)
}

pub(super) fn currency_code(currency: &stripe_types::Currency) -> String {
    crate::integration::store::currency_code(&currency.to_string())
}

pub(super) fn stripe_amount_to_thousandths(amount: i64, currency: &stripe_types::Currency) -> i64 {
    minor_units_to_thousandths(amount, &currency.to_string())
}
