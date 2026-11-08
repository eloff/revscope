use serde_json::Value;

use crate::metrics::{monthly_amount, months_per_period};
use crate::test_utils::open_migrated_db;
use crate::untrusted_keys::UntrustedKeyMap;

use super::checkout::{
    checkout_enrichment_metadata, update_payment_checkout_by_provider_payment_id,
};
use super::plan::{PlanItem, RecurringDiscount, monthly_plan_amount, recurring_discount};
use super::*;

/// One currency unit, in the integer thousandths amounts are stored in.
const DOLLAR: i64 = 1_000;

fn plan_item(amount: i64, interval_unit: &str, interval_count: i64) -> PlanItem<'_> {
    PlanItem {
        amount,
        interval_unit,
        interval_count,
        discounts: Vec::new(),
    }
}

fn discounted_plan_item<'a>(
    amount: i64,
    interval_unit: &'a str,
    interval_count: i64,
    discounts: &[RecurringDiscount],
) -> PlanItem<'a> {
    PlanItem {
        discounts: discounts.to_vec(),
        ..plan_item(amount, interval_unit, interval_count)
    }
}

/// The plan's monthly amount, rounded the way it is when stored.
fn monthly(
    items: &[PlanItem<'_>],
    discounts: &[RecurringDiscount],
    period_months: f64,
) -> Option<i64> {
    monthly_plan_amount(items, discounts, period_months).map(|monthly| monthly.round() as i64)
}

fn forever_coupon() -> stripe_shared::Coupon {
    stripe_shared::Coupon {
        amount_off: None,
        applies_to: None,
        created: 0,
        currency: None,
        currency_options: None,
        duration: stripe_shared::CouponDuration::Forever,
        duration_in_months: None,
        id: "co_test".into(),
        livemode: false,
        max_redemptions: None,
        metadata: None,
        name: None,
        percent_off: None,
        redeem_by: None,
        times_redeemed: 0,
        valid: true,
    }
}

#[test]
fn mixed_interval_items_are_normalized_before_being_summed() {
    let items = [
        plan_item(100 * DOLLAR, "month", 1),
        plan_item(1_200 * DOLLAR, "year", 1),
    ];

    assert_eq!(monthly(&items, &[], 1.0), Some(200 * DOLLAR));
}

#[test]
fn item_amounts_are_divided_by_their_own_interval_count() {
    let items = [plan_item(300 * DOLLAR, "month", 3)];

    assert_eq!(monthly(&items, &[], 3.0), Some(100 * DOLLAR));
}

/// The billing interval reported for a mixed-interval subscription is the
/// first item's, so the amount stored alongside it has to be the monthly
/// total expressed over that interval for the derived MRR to come out right.
#[test]
fn plan_amount_stays_consistent_with_an_annual_billing_interval() {
    let items = [
        plan_item(1_200 * DOLLAR, "year", 1),
        plan_item(100 * DOLLAR, "month", 1),
    ];
    let period_months = months_per_period("year", 1).expect("months per year");

    let amount = (monthly_plan_amount(&items, &[], period_months).expect("monthly") * period_months)
        .round() as i64;

    assert_eq!(amount, 2_400 * DOLLAR);
    assert_eq!(monthly_amount(amount, "year", 1), Some(200 * DOLLAR));
}

#[test]
fn an_item_discount_only_reduces_its_own_item() {
    let items = [
        discounted_plan_item(
            100 * DOLLAR,
            "month",
            1,
            &[RecurringDiscount::PercentOff(25.0)],
        ),
        plan_item(100 * DOLLAR, "month", 1),
    ];

    assert_eq!(monthly(&items, &[], 1.0), Some(175 * DOLLAR));
}

#[test]
fn a_fixed_discount_is_spread_over_the_period_it_is_taken_off() {
    let items = [discounted_plan_item(
        1_200 * DOLLAR,
        "year",
        1,
        &[RecurringDiscount::AmountOff(120 * DOLLAR)],
    )];

    assert_eq!(monthly(&items, &[], 12.0), Some(90 * DOLLAR));
}

#[test]
fn a_subscription_discount_reduces_the_item_total() {
    let items = [
        plan_item(100 * DOLLAR, "month", 1),
        plan_item(1_200 * DOLLAR, "year", 1),
    ];

    assert_eq!(
        monthly(&items, &[RecurringDiscount::PercentOff(50.0)], 1.0),
        Some(100 * DOLLAR)
    );
    assert_eq!(
        monthly(&items, &[RecurringDiscount::AmountOff(120 * DOLLAR)], 12.0),
        Some(190 * DOLLAR)
    );
}

#[test]
fn discounts_cannot_take_the_plan_below_zero() {
    let items = [plan_item(100 * DOLLAR, "month", 1)];

    assert_eq!(
        monthly(&items, &[RecurringDiscount::AmountOff(500 * DOLLAR)], 1.0),
        Some(0)
    );
}

#[test]
fn an_interval_we_cannot_normalize_leaves_the_amount_unknown() {
    assert_eq!(monthly(&[], &[], 1.0), None);
    assert_eq!(
        monthly(&[plan_item(100 * DOLLAR, "fortnight", 1)], &[], 1.0),
        None
    );
    assert_eq!(
        monthly(
            &[
                plan_item(100 * DOLLAR, "fortnight", 1),
                plan_item(50 * DOLLAR, "month", 1),
            ],
            &[],
            1.0,
        ),
        Some(50 * DOLLAR)
    );
}

#[test]
fn a_one_off_coupon_leaves_the_run_rate_alone() {
    let coupon = stripe_shared::Coupon {
        duration: stripe_shared::CouponDuration::Once,
        percent_off: Some(50.0),
        ..forever_coupon()
    };

    assert_eq!(
        recurring_discount(&coupon, &stripe_types::Currency::USD),
        None
    );
}

#[test]
fn a_repeating_coupon_discounts_the_run_rate_while_it_lasts() {
    let coupon = stripe_shared::Coupon {
        duration: stripe_shared::CouponDuration::Repeating,
        duration_in_months: Some(3),
        percent_off: Some(50.0),
        ..forever_coupon()
    };

    assert_eq!(
        recurring_discount(&coupon, &stripe_types::Currency::USD),
        Some(RecurringDiscount::PercentOff(50.0))
    );
}

#[test]
fn a_fixed_coupon_needs_an_amount_in_the_subscription_currency() {
    let coupon = stripe_shared::Coupon {
        amount_off: Some(1_000),
        currency: Some(stripe_types::Currency::USD),
        ..forever_coupon()
    };

    assert_eq!(
        recurring_discount(&coupon, &stripe_types::Currency::USD),
        Some(RecurringDiscount::AmountOff(10 * DOLLAR))
    );
    assert_eq!(
        recurring_discount(&coupon, &stripe_types::Currency::EUR),
        None
    );

    let with_eur = stripe_shared::Coupon {
        currency_options: Some(
            [(
                stripe_types::Currency::EUR,
                stripe_shared::CouponCurrencyOption { amount_off: 900 },
            )]
            .into(),
        ),
        ..coupon
    };

    assert_eq!(
        recurring_discount(&with_eur, &stripe_types::Currency::EUR),
        Some(RecurringDiscount::AmountOff(9 * DOLLAR))
    );
}

#[test]
fn checkout_enrichment_sets_payment_checkout_id_and_merges_metadata() {
    let conn = open_migrated_db();
    conn.execute(
        "INSERT INTO payments (
            provider, provider_payment_id, customer_email, payment_type, status,
            amount, currency, paid_at, created_at, metadata
         ) VALUES (
            'stripe', 'pi_123', 'buyer@example.com', 'single', 'ok',
            5000, 'USD', 100, 100, json('{\"plan\":\"basic\",\"shared\":\"payment\"}')
         )",
        [],
    )
    .expect("insert payment");

    let mut checkout_metadata = UntrustedKeyMap::new();
    checkout_metadata.insert("checkout_key".to_string(), "checkout_value".to_string());
    checkout_metadata.insert("shared".to_string(), "checkout".to_string());
    let mut counts = SyncCounts::default();

    assert!(
        update_payment_checkout_by_provider_payment_id(
            &conn,
            "pi_123",
            "cs_123",
            Some(&checkout_metadata),
            &mut counts,
        )
        .expect("enrich payment")
    );
    assert_eq!(counts.records_updated, 1);

    let payment: (Option<String>, Option<String>) = conn
        .query_row(
            "SELECT provider_checkout_id, metadata
             FROM payments
             WHERE provider = 'stripe' AND provider_payment_id = 'pi_123'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("payment");
    assert_eq!(payment.0.as_deref(), Some("cs_123"));
    let metadata: Value = serde_json::from_str(&payment.1.expect("metadata")).unwrap();
    assert_eq!(metadata["plan"], "basic");
    assert_eq!(metadata["checkout_key"], "checkout_value");
    assert_eq!(metadata["shared"], "checkout");

    assert!(
        !update_payment_checkout_by_provider_payment_id(
            &conn,
            "pi_123",
            "cs_123",
            Some(&checkout_metadata),
            &mut counts,
        )
        .expect("enrich payment again")
    );
    assert_eq!(counts.records_updated, 1);
}

#[test]
fn checkout_enrichment_metadata_includes_client_reference_id() {
    let mut session_metadata = UntrustedKeyMap::new();
    session_metadata.insert("plan".to_string(), "pro".to_string());

    let merged = checkout_enrichment_metadata(Some(&session_metadata), Some("  visitor_abc  "))
        .expect("metadata");
    assert_eq!(merged.get("plan").map(String::as_str), Some("pro"));
    assert_eq!(
        merged.get("client_reference_id").map(String::as_str),
        Some("visitor_abc")
    );

    let only_ref = checkout_enrichment_metadata(None, Some("visitor_abc")).expect("metadata");
    assert_eq!(
        only_ref.get("client_reference_id").map(String::as_str),
        Some("visitor_abc")
    );

    assert!(checkout_enrichment_metadata(None, Some("   ")).is_none());
    assert!(checkout_enrichment_metadata(None, None).is_none());
}

#[test]
fn checkout_enrichment_stores_client_reference_id_in_payment_metadata() {
    let conn = open_migrated_db();
    conn.execute(
        "INSERT INTO payments (
            provider, provider_payment_id, customer_email, payment_type, status,
            amount, currency, paid_at, created_at, metadata
         ) VALUES (
            'stripe', 'pi_ref', 'buyer@example.com', 'single', 'ok',
            5000, 'USD', 100, 100, NULL
         )",
        [],
    )
    .expect("insert payment");

    let checkout_metadata =
        checkout_enrichment_metadata(None, Some("attr_key_123")).expect("metadata");
    let mut counts = SyncCounts::default();
    assert!(
        update_payment_checkout_by_provider_payment_id(
            &conn,
            "pi_ref",
            "cs_ref",
            Some(&checkout_metadata),
            &mut counts,
        )
        .expect("enrich payment")
    );

    let metadata: Value = conn
        .query_row(
            "SELECT metadata
             FROM payments
             WHERE provider = 'stripe' AND provider_payment_id = 'pi_ref'",
            [],
            |row| row.get::<_, Option<String>>(0),
        )
        .expect("payment")
        .map(|value| serde_json::from_str(&value).unwrap())
        .expect("metadata");
    assert_eq!(metadata["client_reference_id"], "attr_key_123");
}
