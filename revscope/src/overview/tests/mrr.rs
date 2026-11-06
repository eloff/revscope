use super::*;

#[test]
fn fetch_mrr_is_zero_on_empty_database() {
    let conn = open_migrated_db();
    let mrr = fetch_mrr(&conn, DAY).expect("fetch mrr");
    assert_eq!(mrr, 0);
}

#[test]
fn fetch_mrr_sums_active_monthly_subscriptions() {
    let conn = open_migrated_db();
    let sub_id = insert_subscription(&conn, "sub_monthly", "month", 0, None, None);
    insert_payment(
        &conn,
        Some(sub_id),
        None,
        "pay_monthly_1",
        "subscription",
        DOLLARS_100,
        DAY,
    );

    let mrr = fetch_mrr(&conn, DAY * 2).expect("fetch mrr");
    assert_eq!(mrr, DOLLARS_100);
}

#[test]
fn fetch_mrr_normalizes_annual_payments_to_monthly() {
    let conn = open_migrated_db();
    let sub_id = insert_subscription(&conn, "sub_annual", "year", 0, None, None);
    insert_payment(
        &conn,
        Some(sub_id),
        None,
        "pay_annual_1",
        "subscription",
        DOLLARS_1200,
        DAY,
    );

    let mrr = fetch_mrr(&conn, DAY * 2).expect("fetch mrr");
    assert_eq!(mrr, DOLLARS_1200 / 12);
}

#[test]
fn fetch_mrr_excludes_subscriptions_canceled_before_as_of() {
    let conn = open_migrated_db();
    let sub_id = insert_subscription(&conn, "sub_churned", "month", 0, Some(DAY * 5), None);
    insert_payment(
        &conn,
        Some(sub_id),
        None,
        "pay_churned_1",
        "subscription",
        DOLLARS_200,
        DAY,
    );

    assert_eq!(
        fetch_mrr(&conn, DAY * 4).expect("before cancel"),
        DOLLARS_200
    );
    assert_eq!(fetch_mrr(&conn, DAY * 6).expect("after cancel"), 0);
}

/// Cancelling at period end leaves the customer paid up until the period
/// runs out, so the request itself takes nothing away.
#[test]
fn fetch_mrr_keeps_a_subscription_that_is_paid_through_a_scheduled_cancellation() {
    let conn = open_migrated_db();
    let cancel_at = DAY * 30;
    let mut subscription = SubscriptionFixture::new("sub_scheduled");
    subscription.canceled_at = Some(DAY * 5);
    subscription.cancel_at = Some(cancel_at);
    let sub_id = insert_test_subscription(&conn, subscription);
    insert_payment(
        &conn,
        Some(sub_id),
        None,
        "pay_scheduled_1",
        "subscription",
        DOLLARS_200,
        DAY,
    );

    assert_eq!(
        fetch_mrr(&conn, DAY * 10).expect("after the cancellation was requested"),
        DOLLARS_200
    );
    assert_eq!(
        fetch_mrr(&conn, cancel_at + DAY).expect("after the cancellation took effect"),
        0
    );
}

/// Some providers flip the status to canceled the moment it is requested,
/// while the customer keeps the subscription until the period they paid for
/// runs out. The dates decide when the revenue stops, not the status.
#[test]
fn fetch_mrr_keeps_a_scheduled_cancellation_the_provider_already_calls_canceled() {
    let conn = open_migrated_db();
    let requested_at = DAY * 5;
    let cancel_at = DAY * 30;

    let mut subscription = SubscriptionFixture::new("sub_called_canceled");
    subscription.plan_amount = Some(DOLLARS_200);
    let sub_id = insert_test_subscription(&conn, subscription);
    insert_payment(
        &conn,
        Some(sub_id),
        None,
        "pay_called_canceled",
        "subscription",
        DOLLARS_200,
        DAY,
    );

    let mut canceled = SubscriptionVersionFixture::new(requested_at);
    canceled.change_kind = "canceled".to_string();
    canceled.status = Some(subscription_status::CANCELED.to_string());
    canceled.plan_amount = Some(DOLLARS_200);
    canceled.mrr_amount = Some(DOLLARS_200);
    canceled.canceled_at = Some(requested_at);
    canceled.cancel_at = Some(cancel_at);
    insert_subscription_version(&conn, sub_id, canceled);

    assert_eq!(
        fetch_mrr(&conn, requested_at + DAY).expect("paid through the period"),
        DOLLARS_200
    );
    assert_eq!(
        fetch_mrr(&conn, cancel_at + DAY).expect("after the cancellation took effect"),
        0
    );
}

#[test]
fn fetch_mrr_uses_latest_subscription_payment() {
    let conn = open_migrated_db();
    let sub_id = insert_subscription(&conn, "sub_upgraded", "month", 0, None, None);
    insert_payment(
        &conn,
        Some(sub_id),
        None,
        "pay_upgrade_1",
        "subscription",
        DOLLARS_50,
        DAY,
    );
    insert_payment(
        &conn,
        Some(sub_id),
        None,
        "pay_upgrade_2",
        "subscription",
        DOLLARS_100,
        DAY * 2,
    );

    let mrr = fetch_mrr(&conn, DAY * 3).expect("fetch mrr");
    assert_eq!(mrr, DOLLARS_100);
}

#[test]
fn fetch_mrr_prefers_recurring_price_over_last_payment() {
    let conn = open_migrated_db();
    let mut subscription = SubscriptionFixture::new("sub_priced");
    subscription.started_at = Some(0);
    subscription.plan_amount = Some(DOLLARS_100);
    let sub_id = insert_test_subscription(&conn, subscription);
    // A prorated first invoice, which is not what the subscription costs.
    insert_payment(
        &conn,
        Some(sub_id),
        None,
        "pay_prorated",
        "subscription",
        DOLLARS_25,
        DAY,
    );

    assert_eq!(fetch_mrr(&conn, DAY * 2).expect("fetch mrr"), DOLLARS_100);
}

#[test]
fn fetch_mrr_follows_plan_changes_across_versions() {
    let conn = open_migrated_db();
    let upgraded_at = DAY * 10;
    let mut subscription = SubscriptionFixture::new("sub_versioned");
    subscription.started_at = Some(0);
    subscription.plan_amount = Some(DOLLARS_50);
    let sub_id = insert_test_subscription(&conn, subscription);

    insert_payment(
        &conn,
        Some(sub_id),
        None,
        "pay_versioned",
        "subscription",
        DOLLARS_50,
        DAY,
    );

    let mut upgrade = SubscriptionVersionFixture::new(upgraded_at);
    upgrade.change_kind = "upgrade".to_string();
    upgrade.plan_amount = Some(DOLLARS_200);
    upgrade.mrr_amount = Some(DOLLARS_200);
    insert_subscription_version(&conn, sub_id, upgrade);

    assert_eq!(
        fetch_mrr(&conn, upgraded_at - DAY).expect("before upgrade"),
        DOLLARS_50
    );
    assert_eq!(
        fetch_mrr(&conn, upgraded_at + DAY).expect("after upgrade"),
        DOLLARS_200
    );
}

#[test]
fn fetch_mrr_normalizes_annual_recurring_price_to_monthly() {
    let conn = open_migrated_db();
    let mut subscription = SubscriptionFixture::new("sub_annual_priced");
    subscription.interval_unit = "year".to_string();
    subscription.started_at = Some(0);
    subscription.plan_amount = Some(DOLLARS_1200);
    let sub_id = insert_test_subscription(&conn, subscription);
    // A partial first year, which is not what the subscription costs.
    insert_payment(
        &conn,
        Some(sub_id),
        None,
        "pay_annual_partial",
        "subscription",
        DOLLARS_200,
        0,
    );

    assert_eq!(fetch_mrr(&conn, DAY).expect("fetch mrr"), DOLLARS_1200 / 12);
}

#[test]
fn fetch_mrr_excludes_a_trial_until_it_is_charged() {
    let conn = open_migrated_db();
    let charged_at = DAY * 14;
    let mut subscription = SubscriptionFixture::new("sub_trial");
    subscription.started_at = Some(0);
    subscription.plan_amount = Some(DOLLARS_100);
    let sub_id = insert_test_subscription(&conn, subscription);
    // Providers open a trial with an invoice for nothing, settled up front.
    insert_payment(
        &conn,
        Some(sub_id),
        None,
        "pay_trial_invoice",
        "subscription",
        0,
        0,
    );

    assert_eq!(
        fetch_mrr(&conn, DAY).expect("during trial"),
        0,
        "a trial's future price is not revenue until it is charged"
    );

    insert_payment(
        &conn,
        Some(sub_id),
        None,
        "pay_trial_conversion",
        "subscription",
        DOLLARS_100,
        charged_at,
    );

    assert_eq!(
        fetch_mrr(&conn, charged_at + DAY).expect("after conversion"),
        DOLLARS_100
    );
}

#[test]
fn fetch_mrr_counts_only_states_that_are_being_billed() {
    /// MRR of one paid-up subscription left in `status`.
    fn mrr_in_status(status: &str) -> i64 {
        let conn = open_migrated_db();
        let mut subscription = SubscriptionFixture::new("sub_status");
        subscription.started_at = Some(0);
        subscription.plan_amount = Some(DOLLARS_100);
        let sub_id = insert_test_subscription(&conn, subscription);
        insert_payment(
            &conn,
            Some(sub_id),
            None,
            "pay_status",
            "subscription",
            DOLLARS_100,
            DAY,
        );

        let mut version = SubscriptionVersionFixture::new(DAY * 10);
        version.change_kind = "status_change".to_string();
        version.status = Some(status.to_string());
        version.plan_amount = Some(DOLLARS_100);
        version.mrr_amount = Some(DOLLARS_100);
        insert_subscription_version(&conn, sub_id, version);

        fetch_mrr(&conn, DAY * 11).expect("fetch mrr")
    }

    for status in subscription_status::NON_BILLING {
        assert_eq!(
            mrr_in_status(status),
            0,
            "nobody is being charged for a {status} subscription"
        );
    }
    assert_eq!(mrr_in_status(subscription_status::ACTIVE), DOLLARS_100);
    assert_eq!(
        mrr_in_status(subscription_status::PAST_DUE),
        DOLLARS_100,
        "a subscription in dunning is still being billed"
    );
    assert_eq!(
        mrr_in_status(subscription_status::UNKNOWN),
        DOLLARS_100,
        "a status we could not map must not erase revenue"
    );
}

#[test]
fn fetch_mrr_drops_a_paused_subscription_until_it_resumes() {
    let conn = open_migrated_db();
    let paused_at = DAY * 10;
    let resumed_at = DAY * 20;

    let mut subscription = SubscriptionFixture::new("sub_paused");
    subscription.started_at = Some(0);
    subscription.plan_amount = Some(DOLLARS_100);
    let sub_id = insert_test_subscription(&conn, subscription);
    insert_payment(
        &conn,
        Some(sub_id),
        None,
        "pay_paused",
        "subscription",
        DOLLARS_100,
        DAY,
    );

    let mut paused = SubscriptionVersionFixture::new(paused_at);
    paused.change_kind = "status_change".to_string();
    paused.status = Some(subscription_status::PAUSED.to_string());
    paused.plan_amount = Some(DOLLARS_100);
    paused.mrr_amount = Some(DOLLARS_100);
    insert_subscription_version(&conn, sub_id, paused);

    let mut resumed = SubscriptionVersionFixture::new(resumed_at);
    resumed.change_kind = "status_change".to_string();
    resumed.plan_amount = Some(DOLLARS_100);
    resumed.mrr_amount = Some(DOLLARS_100);
    insert_subscription_version(&conn, sub_id, resumed);

    assert_eq!(
        fetch_mrr(&conn, paused_at - DAY).expect("before pause"),
        DOLLARS_100
    );
    assert_eq!(fetch_mrr(&conn, paused_at + DAY).expect("while paused"), 0);
    assert_eq!(
        fetch_mrr(&conn, resumed_at + DAY).expect("after resume"),
        DOLLARS_100
    );
}

#[test]
fn fetch_mrr_ignores_refund_and_chargeback_payment_rows() {
    let conn = open_migrated_db();
    let sub_id = insert_subscription(&conn, "sub_refunded", "month", 0, None, None);
    insert_payment(
        &conn,
        Some(sub_id),
        None,
        "pay_sub",
        "subscription",
        DOLLARS_100,
        DAY,
    );
    insert_payment(
        &conn,
        Some(sub_id),
        None,
        "pay_refund",
        "refund",
        -DOLLARS_100,
        DAY * 2,
    );
    insert_payment(
        &conn,
        Some(sub_id),
        None,
        "pay_chargeback",
        "chargeback",
        -DOLLARS_100,
        DAY * 3,
    );

    let mrr = fetch_mrr(&conn, DAY * 4).expect("fetch mrr");
    assert_eq!(
        mrr, DOLLARS_100,
        "MRR uses latest ok subscription payment; refund/chargeback rows do not reduce it"
    );
}

#[test]
fn fetch_mrr_drops_after_subscription_cancel_even_with_only_refund_after() {
    let conn = open_migrated_db();
    let sub_id = insert_subscription(
        &conn,
        "sub_refund_then_cancel",
        "month",
        0,
        Some(DAY * 4),
        None,
    );
    insert_payment(
        &conn,
        Some(sub_id),
        None,
        "pay_sub",
        "subscription",
        DOLLARS_100,
        DAY,
    );
    insert_payment(
        &conn,
        Some(sub_id),
        None,
        "pay_refund",
        "refund",
        -DOLLARS_100,
        DAY * 2,
    );

    assert_eq!(
        fetch_mrr(&conn, DAY * 3).expect("before cancel"),
        DOLLARS_100
    );
    assert_eq!(
        fetch_mrr(&conn, DAY * 5).expect("after cancel"),
        0,
        "canceled_at removes the sub from MRR; refunds alone do not"
    );
}
