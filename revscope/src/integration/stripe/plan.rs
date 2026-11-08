//! A subscription's recurring price after discounts, and the payment intents
//! and coupons fetched to work it out.

use fxhash::FxHashMap;
use log::warn;

use crate::integration::sync::SyncResult;
use crate::metrics::months_per_period;
use crate::untrusted_keys::UntrustedKeyMap;
use crate::util::unix_seconds;

use super::STRIPE_PAGE_LIMIT;
use super::records::{invoice_payment_intent, stripe_amount_to_thousandths};

/// The recurring plan and current period, taken from the subscription's items.
pub(super) struct StripePlan<'a> {
    /// The first item's billing period, absent when the payload carries no
    /// priced item to read one from.
    pub(super) interval: Option<(&'a str, i64)>,
    pub(super) provider_price_id: Option<String>,
    pub(super) quantity: Option<i64>,
    pub(super) amount: Option<i64>,
    pub(super) current_period_start: Option<i64>,
    pub(super) current_period_end: Option<i64>,
}

/// Payment intent metadata keyed by payment intent id, with `None` for one we
/// couldn't read. The metadata map is the type the Stripe crate returns.
///
/// Stripe caps expansion at four levels, one short of
/// `data.payments.data.payment.payment_intent` on the invoice list. The invoice
/// payment list reaches the same object in three
/// (`data.payment.payment_intent`).
pub(super) type PaymentIntentMetadataCache =
    FxHashMap<String, Option<UntrustedKeyMap<String, String>>>;

/// Loads payment intent metadata for invoice payments created in the sync
/// window into `payment_intents`.
pub(super) async fn cache_invoice_payment_intents(
    client: &stripe::Client,
    since: Option<i64>,
    payment_intents: &mut PaymentIntentMetadataCache,
) -> SyncResult<()> {
    let mut starting_after = None;
    loop {
        let mut request = stripe_billing::invoice_payment::ListInvoicePayment::new()
            .expand(vec!["data.payment.payment_intent".to_string()])
            .limit(STRIPE_PAGE_LIMIT);
        if let Some(since) = since {
            request = request.created(stripe_types::RangeQueryTs::gte(since));
        }
        if let Some(cursor) = starting_after.take() {
            request = request.starting_after(cursor);
        }
        let page = request.send(client).await?;
        let has_more = page.has_more;
        for invoice_payment in page.data {
            starting_after = Some(invoice_payment.id.to_string());
            let Some(payment_intent) = invoice_payment.payment.payment_intent.as_ref() else {
                continue;
            };
            let Some(object) = payment_intent.as_object() else {
                continue;
            };
            payment_intents.insert(
                payment_intent.id().to_string(),
                Some(object.metadata.clone()),
            );
        }
        if !has_more {
            break;
        }
    }
    Ok(())
}

/// Retrieves payment intents on `invoices` that the invoice payment list did
/// not already cache.
///
/// A payment intent we can't read is cached as absent. The invoice is then
/// stored with its own metadata only, which keeps a single unreadable payment
/// intent from failing the sync.
pub(super) async fn cache_missing_payment_intents(
    client: &stripe::Client,
    invoices: &[stripe_billing::Invoice],
    payment_intents: &mut PaymentIntentMetadataCache,
) {
    for invoice in invoices {
        let Some(payment_intent) = invoice_payment_intent(invoice) else {
            continue;
        };
        if payment_intent.as_object().is_some() {
            continue;
        }
        let payment_intent_id = payment_intent.id().to_string();
        if payment_intents.contains_key(&payment_intent_id) {
            continue;
        }
        let fetched =
            stripe_core::payment_intent::RetrievePaymentIntent::new(payment_intent.id().clone())
                .send(client)
                .await
                .inspect_err(|error| {
                    warn!("failed to fetch stripe payment intent {payment_intent_id}: {error}")
                })
                .ok()
                .map(|payment_intent| payment_intent.metadata);
        payment_intents.insert(payment_intent_id, fetched);
    }
}

/// The coupons behind the discounts on a page of subscriptions, keyed by coupon
/// id, with `None` for one we couldn't read.
///
/// Stripe caps expansion at four levels, one short of what
/// `data.items.data.discounts.source.coupon` needs on the list endpoint, so item
/// discounts arrive carrying only a coupon id. A handful of coupons usually
/// backs an entire account, so resolving those ids into a cache that lives for
/// the whole sync costs a request per distinct coupon rather than per item.
pub(super) type CouponCache = FxHashMap<String, Option<stripe_shared::Coupon>>;

/// Resolves the coupons behind a page's discounts that aren't already expanded,
/// adding them to `coupons`.
///
/// A coupon we can't read is cached as absent: the discount it backs is then
/// left out of the run rate, which reports the undiscounted price rather than
/// failing the sync over a detail this far from the money.
pub(super) async fn cache_page_coupons(
    client: &stripe::Client,
    subscriptions: &[stripe_billing::Subscription],
    coupons: &mut CouponCache,
) {
    for discount in subscriptions.iter().flat_map(discount_refs) {
        let Some(discount) = discount.as_object() else {
            continue;
        };
        let Some(coupon) = discount.source.coupon.as_ref() else {
            continue;
        };
        if let Some(coupon) = coupon.as_object() {
            coupons.insert(coupon.id.to_string(), Some(coupon.clone()));
            continue;
        }
        let coupon_id = coupon.id();
        if coupons.contains_key(coupon_id.as_str()) {
            continue;
        }
        let fetched = stripe_product::coupon::RetrieveCoupon::new(coupon_id.clone())
            .send(client)
            .await
            .inspect_err(|error| warn!("failed to fetch stripe coupon {coupon_id}: {error}"))
            .ok();
        coupons.insert(coupon_id.to_string(), fetched);
    }
}

pub(super) fn discount_refs(
    subscription: &stripe_billing::Subscription,
) -> impl Iterator<Item = &stripe_types::Expandable<stripe_shared::Discount>> {
    subscription.discounts.iter().chain(
        subscription
            .items
            .data
            .iter()
            .flat_map(|item| item.discounts.iter()),
    )
}

/// The coupon a discount redeemed, taken from the discount itself when it was
/// expanded and from the cache otherwise.
pub(super) fn coupon<'a>(
    discount: &'a stripe_shared::Discount,
    coupons: &'a CouponCache,
) -> Option<&'a stripe_shared::Coupon> {
    let coupon = discount.source.coupon.as_ref()?;
    coupon
        .as_object()
        .or_else(|| coupons.get(coupon.id().as_str())?.as_ref())
}

/// A discount that reduces every invoice, and so the run rate, for as long as
/// it is in force.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum RecurringDiscount {
    /// Percentage taken off the amount, in the range `0.0..=100.0`.
    PercentOff(f64),
    /// Fixed amount taken off each invoice, in integer thousandths.
    AmountOff(i64),
}

/// One subscription item reduced to what it contributes to the run rate.
#[derive(Clone, Debug)]
pub(super) struct PlanItem<'a> {
    /// Integer thousandths charged once per this item's own billing period.
    pub(super) amount: i64,
    pub(super) interval_unit: &'a str,
    pub(super) interval_count: i64,
    pub(super) discounts: Vec<RecurringDiscount>,
}

/// Reads the plan off the subscription's items, which is what makes an upgrade
/// visible before the next invoice arrives.
///
/// A subscription can carry several items, each with its own billing interval,
/// so the items are normalized to a monthly amount individually and only then
/// summed: a \$100/month item alongside a \$1,200/year item is \$200 a month,
/// not \$1,300. `interval_unit` and `interval_count` report the first item's
/// interval, and `amount` is the monthly total expressed over that interval,
/// which keeps the pair consistent and leaves the derived MRR correct whichever
/// interval the first item happens to use. For the common case of a single
/// interval, that amount is exactly the sum of the item amounts.
pub(super) fn subscription_plan<'a>(
    subscription: &'a stripe_billing::Subscription,
    coupons: &CouponCache,
) -> StripePlan<'a> {
    let item = subscription.items.data.first();
    // An interval we can't normalize can't carry a monthly total, so such a
    // plan is reported as monthly rather than against a unit we don't
    // understand. A payload with no priced item to read at all reports no
    // interval, which leaves the stored one alone.
    let interval = item
        .and_then(|item| item.price.recurring.as_ref())
        .map(|recurring| match price_interval(recurring) {
            (unit, count) if months_per_period(unit, count).is_some() => (unit, count),
            _ => ("month", 1),
        });
    let period_months = interval
        .and_then(|(unit, count)| months_per_period(unit, count))
        .unwrap_or(1.0);

    let items = plan_items(subscription, coupons);
    let discounts = recurring_discounts(&subscription.discounts, &subscription.currency, coupons);
    let amount = monthly_plan_amount(&items, &discounts, period_months)
        .map(|monthly| (monthly * period_months).round() as i64);

    StripePlan {
        interval,
        provider_price_id: item.map(|item| item.price.id.to_string()),
        quantity: item
            .and_then(|item| item.quantity)
            .and_then(|quantity| i64::try_from(quantity).ok()),
        amount,
        current_period_start: item.map(|item| item.current_period_start),
        current_period_end: item.map(|item| item.current_period_end),
    }
}

/// The monthly value of the items, or `None` when none of them carry a price.
///
/// Item discounts come off that item's own period amount, then subscription
/// discounts come off the total, which is the order Stripe applies them in.
/// `period_months` is how many months a subscription invoice covers, and is what
/// turns a fixed amount off each invoice into a monthly reduction.
pub(super) fn monthly_plan_amount(
    items: &[PlanItem<'_>],
    discounts: &[RecurringDiscount],
    period_months: f64,
) -> Option<f64> {
    let mut monthly: Option<f64> = None;
    for item in items {
        let Some(item_months) = months_per_period(item.interval_unit, item.interval_count) else {
            continue;
        };
        let item_monthly = apply_discounts(
            item.amount as f64 / item_months,
            &item.discounts,
            item_months,
        );
        monthly = Some(monthly.unwrap_or(0.0) + item_monthly);
    }
    Some(apply_discounts(monthly?, discounts, period_months))
}

/// Reduces a monthly amount by each discount, never below zero.
pub(super) fn apply_discounts(
    monthly: f64,
    discounts: &[RecurringDiscount],
    period_months: f64,
) -> f64 {
    discounts
        .iter()
        .fold(monthly, |monthly, discount| match discount {
            RecurringDiscount::PercentOff(percent) => monthly * (1.0 - percent / 100.0),
            RecurringDiscount::AmountOff(amount) => monthly - *amount as f64 / period_months,
        })
        .max(0.0)
}

pub(super) fn plan_items<'a>(
    subscription: &'a stripe_billing::Subscription,
    coupons: &CouponCache,
) -> Vec<PlanItem<'a>> {
    subscription
        .items
        .data
        .iter()
        .filter_map(|item| {
            let recurring = item.price.recurring.as_ref()?;
            let quantity = i64::try_from(item.quantity.unwrap_or(1)).unwrap_or(1);
            let amount =
                stripe_amount_to_thousandths(item.price.unit_amount?, &item.price.currency)
                    .saturating_mul(quantity);
            let (interval_unit, interval_count) = price_interval(recurring);
            Some(PlanItem {
                amount,
                interval_unit,
                interval_count,
                discounts: recurring_discounts(&item.discounts, &item.price.currency, coupons),
            })
        })
        .collect()
}

/// A price's billing interval, as the unit and count MRR normalizes by.
pub(super) fn price_interval(recurring: &stripe_shared::Recurring) -> (&str, i64) {
    (
        recurring.interval.as_str(),
        i64::try_from(recurring.interval_count).unwrap_or(1).max(1),
    )
}

/// The discounts among `discounts` that are in force now and reduce every
/// invoice, so belong in the run rate.
pub(super) fn recurring_discounts(
    discounts: &[stripe_types::Expandable<stripe_shared::Discount>],
    currency: &stripe_types::Currency,
    coupons: &CouponCache,
) -> Vec<RecurringDiscount> {
    let now = unix_seconds();
    discounts
        .iter()
        .filter_map(|discount| {
            let discount = discount.as_object()?;
            if discount.start > now || discount.end.is_some_and(|end| end <= now) {
                return None;
            }
            recurring_discount(coupon(discount, coupons)?, currency)
        })
        .collect()
}

/// A coupon's effect on the run rate, or `None` when it doesn't have one.
///
/// A `once` coupon discounts a single invoice, so like any other one-off credit
/// it leaves the run rate alone. A `repeating` one runs out eventually, but
/// while it lasts the customer is billed less, and `discount.end` is what says
/// when that stops being true.
pub(super) fn recurring_discount(
    coupon: &stripe_shared::Coupon,
    currency: &stripe_types::Currency,
) -> Option<RecurringDiscount> {
    if coupon.duration == stripe_shared::CouponDuration::Once {
        return None;
    }
    if let Some(percent_off) = coupon.percent_off {
        return Some(RecurringDiscount::PercentOff(percent_off));
    }
    // `amount_off` is denominated in the coupon's own currency, with any other
    // currency it supports listed separately, so a coupon that says nothing
    // about this subscription's currency can't be applied to it.
    let amount_off = if coupon.currency.as_ref() == Some(currency) {
        coupon.amount_off?
    } else {
        coupon
            .currency_options
            .as_ref()?
            .get(currency)
            .map(|option| option.amount_off)?
    };
    Some(RecurringDiscount::AmountOff(stripe_amount_to_thousandths(
        amount_off, currency,
    )))
}
