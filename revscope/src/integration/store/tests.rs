use crate::test_utils::open_migrated_db;

use super::*;

const DAY: i64 = 86_400;
/// Thousandths of a dollar ($50.00).
const DOLLARS_50: i64 = 50_000;
const DOLLARS_100: i64 = 100_000;
const DOLLARS_1200: i64 = 1_200_000;

/// A monthly $50 subscription started at `started_at` and observed now, as a
/// payload describing the whole subscription: it reports no cancellation
/// rather than saying nothing about one.
fn monthly_subscription(started_at: i64, observed_at: i64) -> SubscriptionUpsert<'static> {
    let mut subscription = SubscriptionUpsert::new("stripe", "sub_1");
    subscription.provider_customer_id = Some("cus_1");
    subscription.status = Some("active");
    subscription.plan = SubscriptionPlan {
        interval: Some(PlanInterval::new("month", 1)),
        provider_price_id: Some("price_basic"),
        plan_name: Some("Basic"),
        quantity: Some(1),
        currency: Some("USD"),
        amount: Some(DOLLARS_50),
    };
    subscription.current_period_start = Some(started_at);
    subscription.started_at = Some(started_at);
    subscription.ended_at = Reported::Cleared;
    subscription.canceled_at = Reported::Cleared;
    subscription.cancel_at = Reported::Cleared;
    subscription.observed_at = observed_at;
    subscription
}

/// A payload carrying identity and nothing else, the shape of a subscription
/// embedded in an order.
fn partial_subscription(observed_at: i64) -> SubscriptionUpsert<'static> {
    let mut subscription = SubscriptionUpsert::new("stripe", "sub_1");
    subscription.observed_at = observed_at;
    subscription
}

/// `(interval_unit, interval_count, plan_amount, mrr_amount)` as one row
/// records it.
type StoredPlan = (String, i64, Option<i64>, Option<i64>);

/// The plan on the anchor row.
fn anchor_plan(conn: &ProjectConnection, subscription_id: i64) -> StoredPlan {
    stored_plan(
        conn,
        "SELECT interval_unit, interval_count, plan_amount, mrr_amount
         FROM subscriptions
         WHERE id = ?1",
        subscription_id,
    )
}

/// The plan on the open history version, which is the same plan the anchor
/// holds whenever the two agree.
fn open_version_plan(conn: &ProjectConnection, subscription_id: i64) -> StoredPlan {
    stored_plan(
        conn,
        "SELECT interval_unit, interval_count, plan_amount, mrr_amount
         FROM subscription_history
         WHERE subscription_id = ?1 AND effective_to IS NULL",
        subscription_id,
    )
}

fn stored_plan(conn: &ProjectConnection, sql: &str, subscription_id: i64) -> StoredPlan {
    conn.query_row(sql, params![subscription_id], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
    })
    .expect("stored plan")
}

/// `(version, change_kind, effective_from, effective_to, mrr_amount, prev_mrr_amount)`
/// for every version of a subscription, oldest first.
#[allow(clippy::type_complexity)]
fn versions(
    conn: &ProjectConnection,
    subscription_id: i64,
) -> Vec<(i64, String, i64, Option<i64>, Option<i64>, Option<i64>)> {
    conn.with_query(
        "SELECT version, change_kind, effective_from, effective_to, mrr_amount,
                prev_mrr_amount
         FROM subscription_history
         WHERE subscription_id = ?1
         ORDER BY version",
        params![subscription_id],
        |mut rows| {
            let mut versions = Vec::new();
            while let Some(row) = rows.next()? {
                versions.push((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ));
            }
            Ok(versions)
        },
    )
    .expect("subscription versions")
}

#[test]
fn first_sync_opens_a_created_version_at_the_start_date() {
    let conn = open_migrated_db();
    let mut counts = SyncCounts::default();
    let subscription = monthly_subscription(DAY, DAY * 2);

    let id = upsert_subscription(&conn, &subscription, &mut counts).expect("upsert");

    assert_eq!(counts.records_inserted, 1);
    assert_eq!(
        versions(&conn, id),
        vec![(1, "created".to_string(), DAY, None, Some(DOLLARS_50), None)]
    );
}

#[test]
fn resyncing_unchanged_state_appends_no_version() {
    let conn = open_migrated_db();
    let mut counts = SyncCounts::default();
    let subscription = monthly_subscription(DAY, DAY * 2);
    let id = upsert_subscription(&conn, &subscription, &mut counts).expect("first sync");

    // A later sync of a subscription that only advanced its billing period.
    let mut renewed = monthly_subscription(DAY, DAY * 40);
    renewed.current_period_start = Some(DAY * 31);
    renewed.current_period_end = Some(DAY * 61);
    upsert_subscription(&conn, &renewed, &mut counts).expect("resync");

    assert_eq!(versions(&conn, id).len(), 1);
    assert_eq!(counts.records_updated, 1);
}

#[test]
fn upgrading_closes_the_previous_version_and_records_mrr_movement() {
    let conn = open_migrated_db();
    let mut counts = SyncCounts::default();
    let id = upsert_subscription(&conn, &monthly_subscription(DAY, DAY * 2), &mut counts)
        .expect("first sync");

    let upgraded_at = DAY * 10;
    let mut upgraded = monthly_subscription(DAY, upgraded_at);
    upgraded.plan.provider_price_id = Some("price_pro");
    upgraded.plan.plan_name = Some("Pro");
    upgraded.plan.amount = Some(DOLLARS_100);
    upgraded.current_period_start = Some(upgraded_at);
    upsert_subscription(&conn, &upgraded, &mut counts).expect("upgrade sync");

    assert_eq!(
        versions(&conn, id),
        vec![
            (
                1,
                "created".to_string(),
                DAY,
                Some(upgraded_at),
                Some(DOLLARS_50),
                None
            ),
            (
                2,
                "upgrade".to_string(),
                upgraded_at,
                None,
                Some(DOLLARS_100),
                Some(DOLLARS_50)
            ),
        ]
    );
}

#[test]
fn downgrade_and_cancellation_are_distinguished_by_change_kind() {
    let conn = open_migrated_db();
    let mut counts = SyncCounts::default();
    let id = upsert_subscription(&conn, &monthly_subscription(DAY, DAY * 2), &mut counts)
        .expect("first sync");

    let mut downgraded = monthly_subscription(DAY, DAY * 10);
    downgraded.plan.amount = Some(DOLLARS_50 / 2);
    downgraded.current_period_start = Some(DAY * 10);
    upsert_subscription(&conn, &downgraded, &mut counts).expect("downgrade sync");

    let canceled_at = DAY * 20;
    let mut canceled = monthly_subscription(DAY, canceled_at);
    canceled.plan.amount = Some(DOLLARS_50 / 2);
    canceled.status = Some("canceled");
    canceled.canceled_at = Reported::Value(canceled_at);
    upsert_subscription(&conn, &canceled, &mut counts).expect("cancel sync");

    let kinds: Vec<String> = versions(&conn, id)
        .into_iter()
        .map(|version| version.1)
        .collect();
    assert_eq!(kinds, ["created", "downgrade", "canceled"]);
}

/// Square and Lemon Squeezy report only the date a cancellation takes
/// effect, never the time it was requested, so the schedule alone has to be
/// enough to recognize a cancellation and its reversal.
#[test]
fn a_cancellation_scheduled_for_period_end_is_a_cancellation_and_can_be_undone() {
    let conn = open_migrated_db();
    let mut counts = SyncCounts::default();
    let id = upsert_subscription(&conn, &monthly_subscription(DAY, DAY * 2), &mut counts)
        .expect("first sync");

    let mut scheduled = monthly_subscription(DAY, DAY * 10);
    scheduled.cancel_at = Reported::Value(DAY * 30);
    upsert_subscription(&conn, &scheduled, &mut counts).expect("schedule cancel sync");

    // A later full payload reporting no schedule, which is how a provider
    // says the cancellation was reversed.
    let uncanceled = monthly_subscription(DAY, DAY * 12);
    upsert_subscription(&conn, &uncanceled, &mut counts).expect("uncancel sync");

    let kinds: Vec<String> = versions(&conn, id)
        .into_iter()
        .map(|version| version.1)
        .collect();
    assert_eq!(kinds, ["created", "canceled", "resumed"]);
}

/// A mid-append failure must not leave the open version closed or the
/// anchor updated: the close, insert, and anchor write share one transaction.
#[test]
fn a_failed_history_append_rolls_back_the_anchor_and_keeps_the_open_version() {
    let conn = open_migrated_db();
    let mut counts = SyncCounts::default();
    let id = upsert_subscription(&conn, &monthly_subscription(DAY, DAY * 2), &mut counts)
        .expect("first sync");

    // Occupy the next version number so the successor INSERT fails after
    // the open version has been closed inside the transaction.
    conn.execute(
        "INSERT INTO subscription_history (
            subscription_id, version, effective_from, effective_to, observed_at,
            change_kind, status, quantity, currency, plan_amount, mrr_amount,
            interval_unit, interval_count, fingerprint
         ) VALUES (
            ?1, 2, ?2, ?3, ?2,
            'upgrade', 'active', 1, 'USD', ?4, ?4,
            'month', 1, 'poison'
         )",
        params![id, DAY * 5, DAY * 6, DOLLARS_100],
    )
    .expect("poison next version slot");

    let mut upgraded = monthly_subscription(DAY, DAY * 10);
    upgraded.plan.amount = Some(DOLLARS_100);
    upgraded.current_period_start = Some(DAY * 10);
    assert!(
        upsert_subscription(&conn, &upgraded, &mut counts).is_err(),
        "successor insert must fail on the poisoned version number"
    );

    assert_eq!(
        versions(&conn, id),
        vec![
            (1, "created".to_string(), DAY, None, Some(DOLLARS_50), None),
            (
                2,
                "upgrade".to_string(),
                DAY * 5,
                Some(DAY * 6),
                Some(DOLLARS_100),
                None
            ),
        ]
    );
    assert_eq!(
        anchor_plan(&conn, id),
        ("month".to_string(), 1, Some(DOLLARS_50), Some(DOLLARS_50))
    );
    assert_eq!(counts.records_updated, 0);
}

#[test]
fn a_subscription_cannot_have_two_open_history_versions() {
    let conn = open_migrated_db();
    let mut counts = SyncCounts::default();
    let id = upsert_subscription(&conn, &monthly_subscription(DAY, DAY * 2), &mut counts)
        .expect("first sync");

    let err = conn
        .execute(
            "INSERT INTO subscription_history (
                subscription_id, version, effective_from, effective_to, observed_at,
                change_kind, status, quantity, currency, plan_amount, mrr_amount,
                interval_unit, interval_count, fingerprint
             ) VALUES (
                ?1, 2, ?2, NULL, ?2,
                'upgrade', 'active', 1, 'USD', ?3, ?3,
                'month', 1, 'second-open'
             )",
            params![id, DAY * 10, DOLLARS_100],
        )
        .expect_err("second open version must violate the unique index");
    assert!(
        err.to_string().contains("UNIQUE"),
        "expected unique-index failure, got {err}"
    );
}

#[test]
fn provider_status_wording_is_normalized_before_it_is_stored() {
    let conn = open_migrated_db();
    let mut counts = SyncCounts::default();
    // Lemon Squeezy's word for a trial.
    let mut subscription = monthly_subscription(DAY, DAY * 2);
    subscription.status = Some("on_trial");

    let id = upsert_subscription(&conn, &subscription, &mut counts).expect("upsert");

    let anchor: Option<String> = conn
        .query_row(
            "SELECT status FROM subscriptions WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .expect("subscription status");
    let version: Option<String> = conn
        .query_row(
            "SELECT status FROM subscription_history WHERE subscription_id = ?1",
            params![id],
            |row| row.get(0),
        )
        .expect("version status");
    assert_eq!(anchor.as_deref(), Some("trialing"));
    assert_eq!(version.as_deref(), Some("trialing"));
}

#[test]
fn a_status_that_only_differs_in_spelling_appends_no_version() {
    let conn = open_migrated_db();
    let mut counts = SyncCounts::default();
    let mut first = monthly_subscription(DAY, DAY * 2);
    first.status = Some("cancelled");
    let id = upsert_subscription(&conn, &first, &mut counts).expect("first sync");

    // The same state as another provider spells it.
    let mut second = monthly_subscription(DAY, DAY * 3);
    second.status = Some("canceled");
    upsert_subscription(&conn, &second, &mut counts).expect("resync");

    assert_eq!(versions(&conn, id).len(), 1);
}

#[test]
fn annual_plans_are_normalized_to_monthly_mrr() {
    let conn = open_migrated_db();
    let mut counts = SyncCounts::default();
    let mut subscription = monthly_subscription(DAY, DAY * 2);
    subscription.plan.interval = Some(PlanInterval::new("year", 1));
    subscription.plan.amount = Some(DOLLARS_1200);

    let id = upsert_subscription(&conn, &subscription, &mut counts).expect("upsert");

    let mrr: Option<i64> = conn
        .query_row(
            "SELECT mrr_amount FROM subscriptions WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .expect("subscription mrr");
    assert_eq!(mrr, Some(DOLLARS_1200 / 12));
}

#[test]
fn quarterly_plans_divide_mrr_by_the_interval_count() {
    let conn = open_migrated_db();
    let mut counts = SyncCounts::default();
    let mut subscription = monthly_subscription(DAY, DAY * 2);
    subscription.plan.interval = Some(PlanInterval::new("month", 3));
    subscription.plan.amount = Some(DOLLARS_100 * 3);

    let id = upsert_subscription(&conn, &subscription, &mut counts).expect("upsert");

    assert_eq!(versions(&conn, id)[0].4, Some(DOLLARS_100));
}

#[test]
fn a_partial_payload_does_not_erase_known_plan_details() {
    let conn = open_migrated_db();
    let mut counts = SyncCounts::default();
    let mut full = monthly_subscription(DAY, DAY * 2);
    full.metadata = Some(r#"{"seat":"1"}"#);
    let id = upsert_subscription(&conn, &full, &mut counts).expect("full sync");

    // A subscription embedded in an order carries identity but no price.
    let mut partial = partial_subscription(DAY * 3);
    partial.status = Some("active");
    upsert_subscription(&conn, &partial, &mut counts).expect("partial sync");

    let (price_id, amount, metadata): (Option<String>, Option<i64>, Option<String>) = conn
        .query_row(
            "SELECT provider_price_id, plan_amount, metadata
             FROM subscriptions
             WHERE id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("subscription plan");
    assert_eq!(price_id.as_deref(), Some("price_basic"));
    assert_eq!(amount, Some(DOLLARS_50));
    assert_eq!(metadata.as_deref(), Some(r#"{"seat":"1"}"#));
    assert_eq!(versions(&conn, id).len(), 1, "nothing effective changed");
}

/// The anchor used to keep the price a partial payload didn't carry while the
/// version appended alongside it recorded none, so MRR read from the two
/// disagreed.
#[test]
fn a_version_appended_from_a_partial_payload_carries_the_merged_plan() {
    let conn = open_migrated_db();
    let mut counts = SyncCounts::default();
    let mut annual = monthly_subscription(DAY, DAY * 2);
    annual.plan.interval = Some(PlanInterval::new("year", 1));
    annual.plan.amount = Some(DOLLARS_1200);
    let id = upsert_subscription(&conn, &annual, &mut counts).expect("full sync");

    // Enough of a change to warrant a version, carrying no plan of its own.
    let mut partial = partial_subscription(DAY * 3);
    partial.status = Some("past_due");
    upsert_subscription(&conn, &partial, &mut counts).expect("partial sync");

    let expected = (
        "year".to_string(),
        1,
        Some(DOLLARS_1200),
        Some(DOLLARS_1200 / 12),
    );
    assert_eq!(versions(&conn, id).len(), 2);
    assert_eq!(anchor_plan(&conn, id), expected);
    assert_eq!(open_version_plan(&conn, id), expected);
}

/// Square reports no cadence for a subscription whose plan phase it can't
/// resolve, which used to reprice an annual plan as monthly and book twelve
/// times its MRR.
#[test]
fn a_payload_with_no_interval_keeps_the_stored_billing_period() {
    let conn = open_migrated_db();
    let mut counts = SyncCounts::default();
    let mut annual = monthly_subscription(DAY, DAY * 2);
    annual.plan.interval = Some(PlanInterval::new("year", 1));
    annual.plan.amount = Some(DOLLARS_1200);
    let id = upsert_subscription(&conn, &annual, &mut counts).expect("full sync");

    let mut unpriced = monthly_subscription(DAY, DAY * 3);
    unpriced.plan.interval = None;
    unpriced.plan.amount = None;
    upsert_subscription(&conn, &unpriced, &mut counts).expect("resync");

    assert_eq!(
        anchor_plan(&conn, id),
        (
            "year".to_string(),
            1,
            Some(DOLLARS_1200),
            Some(DOLLARS_1200 / 12)
        )
    );
    assert_eq!(versions(&conn, id).len(), 1);
}

/// Only a payload that covers the lifecycle timestamps can clear them. A
/// partial one saying nothing about a cancellation used to read as the
/// customer having resumed.
#[test]
fn a_partial_payload_does_not_resume_a_canceled_subscription() {
    let conn = open_migrated_db();
    let mut counts = SyncCounts::default();
    let canceled_at = DAY * 10;
    let mut canceled = monthly_subscription(DAY, canceled_at);
    canceled.status = Some("canceled");
    canceled.canceled_at = Reported::Value(canceled_at);
    let id = upsert_subscription(&conn, &canceled, &mut counts).expect("cancel sync");

    upsert_subscription(&conn, &partial_subscription(DAY * 12), &mut counts).expect("partial sync");

    let stored: Option<i64> = conn
        .query_row(
            "SELECT canceled_at FROM subscriptions WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .expect("subscription canceled_at");
    assert_eq!(stored, Some(canceled_at));
    let kinds: Vec<String> = versions(&conn, id)
        .into_iter()
        .map(|version| version.1)
        .collect();
    assert_eq!(kinds, ["created"]);
}

#[test]
fn refunds_are_stored_negative_and_zero_value_ones_are_skipped() {
    let conn = open_migrated_db();
    let mut counts = SyncCounts::default();
    let refund = PaymentUpsert::new(
        "stripe",
        "re_1",
        PaymentType::Refund,
        PaymentStatus::Ok,
        DOLLARS_50,
        "USD",
        DAY,
    );
    upsert_payment(&conn, &refund, &mut counts).expect("refund");

    let empty_refund = PaymentUpsert::new(
        "stripe",
        "re_2",
        PaymentType::Refund,
        PaymentStatus::Ok,
        0,
        "USD",
        DAY,
    );
    upsert_payment(&conn, &empty_refund, &mut counts).expect("zero refund");

    let amounts: Vec<i64> = conn
        .with_query("SELECT amount FROM payments ORDER BY id", [], |mut rows| {
            let mut amounts = Vec::new();
            while let Some(row) = rows.next()? {
                amounts.push(row.get(0)?);
            }
            Ok(amounts)
        })
        .expect("payment amounts");
    assert_eq!(amounts, vec![-DOLLARS_50]);
}

#[test]
fn only_subscription_payments_with_a_known_interval_carry_mrr() {
    let conn = open_migrated_db();
    let mut counts = SyncCounts::default();
    let mut annual = PaymentUpsert::new(
        "stripe",
        "pi_1",
        PaymentType::Subscription,
        PaymentStatus::Ok,
        DOLLARS_1200,
        "USD",
        DAY,
    );
    annual.interval = Some(PaymentInterval {
        unit: "year".to_string(),
        count: 1,
    });
    upsert_payment(&conn, &annual, &mut counts).expect("subscription payment");

    let single = PaymentUpsert::new(
        "stripe",
        "pi_2",
        PaymentType::Single,
        PaymentStatus::Ok,
        DOLLARS_100,
        "USD",
        DAY,
    );
    upsert_payment(&conn, &single, &mut counts).expect("single payment");

    let mrr = |provider_payment_id: &str| -> Option<i64> {
        conn.query_row(
            "SELECT mrr_amount FROM payments WHERE provider_payment_id = ?1",
            params![provider_payment_id],
            |row| row.get(0),
        )
        .expect("payment mrr")
    };
    assert_eq!(mrr("pi_1"), Some(DOLLARS_1200 / 12));
    assert_eq!(mrr("pi_2"), None);
}

#[test]
fn minor_units_scale_by_the_currency_decimals() {
    assert_eq!(minor_units_to_thousandths(1_250, "USD"), 12_500);
    assert_eq!(minor_units_to_thousandths(1_250, "JPY"), 1_250_000);
    assert_eq!(minor_units_to_thousandths(1_250, "KWD"), 1_250);
}

#[test]
fn currency_codes_fall_back_to_the_no_currency_code() {
    assert_eq!(currency_code("usd"), "USD");
    assert_eq!(currency_code(" eur "), "EUR");
    assert_eq!(currency_code("dollars"), "XXX");
    assert_eq!(currency_code(""), "XXX");
}
