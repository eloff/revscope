use std::fmt::Debug;
use std::sync::Arc;

use dodopayments::models::{
    Currency, DefaultPageNumberPagination, IntentStatus, PaymentListResponse, RefundListItem,
    RefundStatus, SubscriptionListResponse, SubscriptionStatus, TimeInterval,
};
use log::info;
use serde::Serialize;
use serde_json::{Value, json};

use super::attribution;
use super::schedule::{
    self, PaymentIntegration, SyncOutcome, decrypt_secret, load_due_payment_integrations,
};
use super::store::{
    PaymentStatus, PaymentType, PaymentUpsert, PlanInterval, ProviderObjectRef, Reported,
    SubscriptionPlan, SubscriptionUpsert, minor_units_to_thousandths, store_provider_object,
    subscription_interval, subscription_row_id, upsert_payment, upsert_subscription,
};
use super::sync::{SyncCounts, SyncError, SyncKind, SyncResult, serialize_metadata};
use crate::db::{ProjectCache, ProjectConnection};
use crate::server_context::ServerContext;
use crate::util::{rfc3339_to_unix, unix_to_rfc3339};

const PROVIDER: &str = "dodo";
const DODO_PAGE_SIZE: usize = 100;

type DodoPage<T> = DefaultPageNumberPagination<T>;

/// Dodo list endpoints expose only creation-time filters, so the periodic full
/// syncs (30-day lookback) are what pick up status changes on older
/// subscriptions and payments.
pub(crate) async fn load_due_integrations(
    server_context: Arc<ServerContext>,
) -> SyncResult<Vec<PaymentIntegration>> {
    load_due_payment_integrations(server_context, PROVIDER).await
}

pub(crate) async fn run_sync_with_retries(
    server_context: Arc<ServerContext>,
    integration: PaymentIntegration,
) {
    let sync_kind = integration.sync_kind;
    schedule::run_sync_with_retries(
        server_context.clone(),
        PROVIDER,
        integration.id,
        integration.credentials_version,
        sync_kind,
        || async {
            let counts = sync_integration(server_context.clone(), &integration).await?;
            Ok(SyncOutcome { counts, sync_kind })
        },
    )
    .await;
}

async fn sync_integration(
    server_context: Arc<ServerContext>,
    integration: &PaymentIntegration,
) -> SyncResult<SyncCounts> {
    let project =
        super::sync::ensure_project_cached(server_context.clone(), &integration.project_id).await?;
    let api_key = decrypt_secret(
        integration.encrypted_secret.as_deref(),
        SyncError::MissingDodoSecret,
        || SyncError::InvalidStoredSecret,
    )?;
    let client = build_client(&api_key)?;
    let since = Some(integration.since());

    let mut counts = SyncCounts::default();
    let external_account_id = integration.external_account_id(PROVIDER);

    sync_subscriptions(
        &server_context,
        &project,
        &client,
        &external_account_id,
        since,
        &mut counts,
    )
    .await?;
    sync_payments(
        &server_context,
        &project,
        &client,
        &external_account_id,
        since,
        &mut counts,
    )
    .await?;
    sync_refunds(
        &server_context,
        &project,
        &client,
        &external_account_id,
        since,
        &mut counts,
    )
    .await?;
    if integration.sync_kind == SyncKind::Full {
        attribution::attribute_full_sync(server_context.clone(), &project, "dodo").await?;
    }

    info!(
        "synced Dodo integration {} ({}): inserted={}, updated={}, deleted={}",
        integration.id,
        integration.sync_kind.as_str(),
        counts.records_inserted,
        counts.records_updated,
        counts.records_deleted
    );
    Ok(counts)
}

fn build_client(api_key: &str) -> SyncResult<dodopayments::Client> {
    Ok(dodopayments::Client::new(
        dodopayments::ClientConfig::new(dodopayments::LIVE_MODE_BASE_URL)
            .with_api_key(api_key.to_string()),
    )?)
}

async fn sync_subscriptions(
    server_context: &ServerContext,
    project: &Arc<ProjectCache>,
    client: &dodopayments::Client,
    external_account_id: &str,
    since: Option<i64>,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    let mut page_number = 0;
    loop {
        let page: DodoPage<SubscriptionListResponse> = client
            .subscriptions()
            .list()
            .query(page_query(page_number, since)?)
            .await?;
        let item_count = page.items.len();
        {
            let conn = crate::db::open_project_database_write_for(server_context, project)?;
            for subscription in page.items {
                let created_at = timestamp_to_unix(&subscription.created_at)?;
                let stored = store_provider_object(
                    &conn,
                    PROVIDER,
                    external_account_id,
                    ProviderObjectRef {
                        object_type: "subscription",
                        object_id: &subscription.subscription_id,
                        created_at: Some(created_at),
                        updated_at: None,
                    },
                    &subscription,
                    counts,
                )?;
                if stored.changed {
                    let subscription_id =
                        store_subscription(&conn, stored.id, &subscription, counts)?;
                    attribution::attribute_subscription_by_id(&conn, subscription_id)?;
                }
            }
        }
        if item_count < DODO_PAGE_SIZE {
            break;
        }
        page_number += 1;
    }
    Ok(())
}

async fn sync_payments(
    server_context: &ServerContext,
    project: &Arc<ProjectCache>,
    client: &dodopayments::Client,
    external_account_id: &str,
    since: Option<i64>,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    let mut page_number = 0;
    loop {
        let page: DodoPage<PaymentListResponse> = client
            .payments()
            .list()
            .query(page_query(page_number, since)?)
            .await?;
        let item_count = page.items.len();
        {
            let conn = crate::db::open_project_database_write_for(server_context, project)?;
            for payment in page.items {
                let created_at = timestamp_to_unix(&payment.created_at)?;
                let stored = store_provider_object(
                    &conn,
                    PROVIDER,
                    external_account_id,
                    ProviderObjectRef {
                        object_type: "payment",
                        object_id: &payment.payment_id,
                        created_at: Some(created_at),
                        updated_at: None,
                    },
                    &payment,
                    counts,
                )?;
                if stored.changed {
                    let provider_payment_id = payment.payment_id.clone();
                    store_payment(&conn, stored.id, &payment, counts)?;
                    attribution::attribute_payment_by_provider_id(
                        &conn,
                        PROVIDER,
                        &provider_payment_id,
                    )?;
                }
            }
        }
        if item_count < DODO_PAGE_SIZE {
            break;
        }
        page_number += 1;
    }
    Ok(())
}

async fn sync_refunds(
    server_context: &ServerContext,
    project: &Arc<ProjectCache>,
    client: &dodopayments::Client,
    external_account_id: &str,
    since: Option<i64>,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    let mut page_number = 0;
    loop {
        let page: DodoPage<RefundListItem> = client
            .refunds()
            .list()
            .query(page_query(page_number, since)?)
            .await?;
        let item_count = page.items.len();
        {
            let conn = crate::db::open_project_database_write_for(server_context, project)?;
            for refund in page.items {
                let created_at = timestamp_to_unix(&refund.created_at)?;
                let stored = store_provider_object(
                    &conn,
                    PROVIDER,
                    external_account_id,
                    ProviderObjectRef {
                        object_type: "refund",
                        object_id: &refund.refund_id,
                        created_at: Some(created_at),
                        updated_at: None,
                    },
                    &refund,
                    counts,
                )?;
                if stored.changed {
                    let provider_payment_id = refund.refund_id.clone();
                    store_refund_payment(&conn, stored.id, &refund, counts)?;
                    attribution::attribute_payment_by_provider_id(
                        &conn,
                        PROVIDER,
                        &provider_payment_id,
                    )?;
                }
            }
        }
        if item_count < DODO_PAGE_SIZE {
            break;
        }
        page_number += 1;
    }
    Ok(())
}

fn page_query(page_number: i64, since: Option<i64>) -> SyncResult<Value> {
    let mut query = json!({
        "page_size": DODO_PAGE_SIZE,
        "page_number": page_number,
    });
    if let Some(since) = since {
        query["created_at_gte"] = Value::String(sync_timestamp(since)?);
    }
    Ok(query)
}

fn store_subscription(
    conn: &ProjectConnection,
    provider_object_id: i64,
    subscription: &SubscriptionListResponse,
    counts: &mut SyncCounts,
) -> SyncResult<i64> {
    let metadata = serialize_metadata(subscription.metadata.as_ref())?;
    let status = subscription_status(subscription.status.as_ref());
    let currency = currency_code(subscription.currency.as_ref());
    let interval_unit = interval_unit(subscription.payment_frequency_interval.as_ref());
    let current_period_end = timestamp_to_unix(&subscription.next_billing_date)?;
    let canceled_at = timestamp_to_unix_opt(subscription.cancelled_at.as_deref())?;
    // Dodo has no terminal timestamp, so a failed or expired subscription is
    // treated as having ended when it was cancelled or when its period ran out.
    let ended_at = if matches!(status.as_str(), "failed" | "expired") {
        canceled_at.or(Some(current_period_end))
    } else {
        None
    };
    // A subscription cancelled at the next billing date keeps billing until
    // then, so the cancellation request is not what ends it.
    let cancel_at = subscription
        .cancel_at_next_billing_date
        .then_some(current_period_end);

    let mut upsert = SubscriptionUpsert::new(PROVIDER, &subscription.subscription_id);
    upsert.provider_object_id = Some(provider_object_id);
    upsert.provider_customer_id = Some(&subscription.customer.customer_id);
    upsert.customer_email = Some(&subscription.customer.email);
    upsert.status = Some(&status);
    upsert.plan = SubscriptionPlan {
        interval: Some(PlanInterval::new(
            &interval_unit,
            subscription.payment_frequency_count,
        )),
        provider_price_id: Some(&subscription.product_id),
        plan_name: subscription.product_name.as_deref(),
        quantity: Some(subscription.quantity),
        currency: Some(&currency),
        amount: Some(minor_units_to_thousandths(
            subscription.recurring_pre_tax_amount,
            &currency,
        )),
    };
    upsert.current_period_start = Some(timestamp_to_unix(&subscription.previous_billing_date)?);
    upsert.current_period_end = Some(current_period_end);
    upsert.started_at = Some(timestamp_to_unix(&subscription.created_at)?);
    upsert.ended_at = Reported::or_cleared(ended_at);
    upsert.canceled_at = Reported::or_cleared(canceled_at);
    upsert.cancel_at = Reported::or_cleared(cancel_at);
    upsert.metadata = metadata.as_deref();
    upsert_subscription(conn, &upsert, counts)
}

fn store_payment(
    conn: &ProjectConnection,
    provider_object_id: i64,
    payment: &PaymentListResponse,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    let metadata = serialize_metadata(payment.metadata.as_ref())?;
    let created_at = timestamp_to_unix(&payment.created_at)?;
    let currency = currency_code(payment.currency.as_ref());
    let native_status = payment.status.as_deref().map(enum_value);
    let subscription_id = payment
        .subscription_id
        .as_deref()
        .map(|id| subscription_row_id(conn, PROVIDER, id))
        .transpose()?
        .flatten();
    let payment_type = if payment.subscription_id.is_some() {
        PaymentType::Subscription
    } else {
        PaymentType::Single
    };

    let mut upsert = PaymentUpsert::new(
        PROVIDER,
        &payment.payment_id,
        payment_type,
        payment_status(payment.status.as_deref()),
        minor_units_to_thousandths(payment.total_amount, &currency),
        &currency,
        created_at,
    );
    upsert.provider_object_id = Some(provider_object_id);
    upsert.provider_object_type = Some("payment");
    upsert.subscription_id = subscription_id;
    upsert.interval = subscription_id
        .map(|id| subscription_interval(conn, id))
        .transpose()?
        .flatten();
    upsert.provider_status = native_status.as_deref();
    upsert.provider_invoice_id = payment.invoice_id.as_deref();
    upsert.provider_customer_id = Some(&payment.customer.customer_id);
    upsert.customer_email = Some(&payment.customer.email);
    upsert.metadata = metadata.as_deref();
    upsert_payment(conn, &upsert, counts)
}

fn store_refund_payment(
    conn: &ProjectConnection,
    provider_object_id: i64,
    refund: &RefundListItem,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    let (Some(amount), Some(currency)) = (refund.amount, refund.currency.as_ref()) else {
        return Ok(());
    };
    let currency = currency_code(currency);
    let native_status = enum_value(refund.status.as_ref());
    let created_at = timestamp_to_unix(&refund.created_at)?;

    let mut upsert = PaymentUpsert::new(
        PROVIDER,
        &refund.refund_id,
        PaymentType::Refund,
        refund_status(refund.status.as_ref()),
        minor_units_to_thousandths(amount, &currency),
        &currency,
        created_at,
    );
    upsert.provider_object_id = Some(provider_object_id);
    upsert.provider_object_type = Some("refund");
    upsert.provider_status = Some(&native_status);
    upsert.provider_refund_id = Some(&refund.refund_id);
    upsert.parent_provider_payment_id = Some(&refund.payment_id);
    upsert_payment(conn, &upsert, counts)
}

fn subscription_status(status: &SubscriptionStatus) -> String {
    enum_value(status).to_ascii_lowercase()
}

fn payment_status(status: Option<&IntentStatus>) -> PaymentStatus {
    match status.map(enum_value).as_deref() {
        Some("succeeded")
        | Some("partially_captured")
        | Some("partially_captured_and_capturable") => PaymentStatus::Ok,
        Some("failed") => PaymentStatus::Failed,
        Some("cancelled") => PaymentStatus::Canceled,
        _ => PaymentStatus::Pending,
    }
}

fn refund_status(status: &RefundStatus) -> PaymentStatus {
    match enum_value(status).as_str() {
        "succeeded" => PaymentStatus::Ok,
        "failed" => PaymentStatus::Failed,
        _ => PaymentStatus::Pending,
    }
}

fn interval_unit(interval: &TimeInterval) -> String {
    match enum_value(interval).to_ascii_lowercase().as_str() {
        "day" => "day",
        "week" => "week",
        "month" => "month",
        "year" => "year",
        _ => "month",
    }
    .to_string()
}

fn currency_code(currency: &Currency) -> String {
    super::store::currency_code(&enum_value(currency))
}

fn enum_value<T>(value: &T) -> String
where
    T: Serialize + Debug,
{
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{value:?}"))
}

fn timestamp_to_unix(value: &str) -> SyncResult<i64> {
    rfc3339_to_unix(value).map_err(|error| {
        SyncError::InvalidProviderData(format!("invalid Dodo timestamp {value:?}: {error}"))
    })
}

fn timestamp_to_unix_opt(value: Option<&str>) -> SyncResult<Option<i64>> {
    value.map(timestamp_to_unix).transpose()
}

fn sync_timestamp(timestamp: i64) -> SyncResult<String> {
    unix_to_rfc3339(timestamp).ok_or_else(|| {
        SyncError::InvalidProviderData(format!("invalid sync timestamp {timestamp}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::open_migrated_db;

    fn customer() -> Value {
        json!({ "customer_id": "cus_1", "email": "buyer@example.com", "name": "Ada" })
    }

    fn subscription(overrides: Value) -> SubscriptionListResponse {
        let mut subscription = json!({
            "billing": { "country": "US" },
            "cancel_at_next_billing_date": false,
            "created_at": "2026-01-01T00:00:00Z",
            "currency": "USD",
            "customer": customer(),
            "discounts": [],
            "metadata": {},
            "next_billing_date": "2026-02-01T00:00:00Z",
            "on_demand": false,
            "payment_frequency_count": 1,
            "payment_frequency_interval": "Month",
            "previous_billing_date": "2026-01-01T00:00:00Z",
            "product_id": "pdt_1",
            "quantity": 1,
            "recurring_pre_tax_amount": 2900,
            "status": "active",
            "subscription_id": "sub_1",
            "subscription_period_count": 12,
            "subscription_period_interval": "Month",
            "tax_inclusive": false,
            "trial_period_days": 0,
        });
        for (key, value) in overrides.as_object().expect("subscription fields") {
            subscription[key] = value.clone();
        }
        serde_json::from_value(subscription).expect("subscription")
    }

    fn payment(overrides: Value) -> PaymentListResponse {
        let mut payment = json!({
            "brand_id": "brand_1",
            "created_at": "2026-01-01T00:00:05Z",
            "currency": "USD",
            "customer": customer(),
            "digital_products_delivered": false,
            "has_license_key": false,
            "metadata": {},
            "payment_id": "pay_1",
            "payment_provider": "stripe",
            "total_amount": 2900,
            "status": "succeeded",
        });
        for (key, value) in overrides.as_object().expect("payment fields") {
            payment[key] = value.clone();
        }
        serde_json::from_value(payment).expect("payment")
    }

    fn subscription_row(
        conn: &ProjectConnection,
    ) -> (String, Option<i64>, Option<i64>, Option<i64>) {
        conn.query_row(
            "SELECT status, ended_at, canceled_at, cancel_at FROM subscriptions
             WHERE provider = 'dodo' AND provider_subscription_id = 'sub_1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("subscription")
    }

    const PERIOD_END: i64 = 1_769_904_000;
    const CANCELLED: i64 = 1_768_435_200;

    #[test]
    fn an_active_subscription_stores_its_plan() {
        let conn = open_migrated_db();
        let mut counts = SyncCounts::default();
        store_subscription(&conn, 1, &subscription(json!({})), &mut counts).expect("store");

        let plan: (i64, String, i64, String, String, i64) = conn
            .query_row(
                "SELECT plan_amount, interval_unit, interval_count, provider_price_id,
                        customer_email, current_period_end
                 FROM subscriptions WHERE provider_subscription_id = 'sub_1'",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .expect("subscription");
        assert_eq!(
            plan,
            (
                29_000,
                "month".to_string(),
                1,
                "pdt_1".to_string(),
                "buyer@example.com".to_string(),
                PERIOD_END,
            )
        );
        assert_eq!(
            subscription_row(&conn),
            ("active".to_string(), None, None, None)
        );
    }

    #[test]
    fn cancelling_at_the_next_billing_date_schedules_the_end_without_ending_it() {
        let conn = open_migrated_db();
        let mut counts = SyncCounts::default();
        let scheduled = subscription(json!({
            "cancel_at_next_billing_date": true,
            "cancelled_at": "2026-01-15T00:00:00Z",
        }));
        store_subscription(&conn, 1, &scheduled, &mut counts).expect("store");
        assert_eq!(
            subscription_row(&conn),
            (
                "active".to_string(),
                None,
                Some(CANCELLED),
                Some(PERIOD_END)
            )
        );

        // Resuming clears the schedule on the next sync.
        store_subscription(&conn, 1, &subscription(json!({})), &mut counts).expect("store");
        assert_eq!(
            subscription_row(&conn),
            ("active".to_string(), None, None, None)
        );
    }

    #[test]
    fn a_failed_or_expired_subscription_ends_when_cancelled_or_else_at_its_period_end() {
        // The stored status is normalized across providers.
        for (status, cancelled_at, stored_status, ended_at) in [
            ("expired", None, "canceled", Some(PERIOD_END)),
            (
                "failed",
                Some("2026-01-15T00:00:00Z"),
                "unpaid",
                Some(CANCELLED),
            ),
            ("cancelled", Some("2026-01-15T00:00:00Z"), "canceled", None),
            ("on_hold", None, "paused", None),
        ] {
            let conn = open_migrated_db();
            let mut counts = SyncCounts::default();
            let stored = subscription(json!({ "status": status, "cancelled_at": cancelled_at }));
            store_subscription(&conn, 1, &stored, &mut counts).expect("store");
            let row = subscription_row(&conn);
            assert_eq!(row.0, stored_status, "{status}");
            assert_eq!(row.1, ended_at, "{status}");
        }
    }

    #[test]
    fn a_subscription_payment_links_to_its_subscription_and_interval() {
        let conn = open_migrated_db();
        let mut counts = SyncCounts::default();
        let row_id =
            store_subscription(&conn, 1, &subscription(json!({})), &mut counts).expect("store");
        store_payment(
            &conn,
            2,
            &payment(json!({ "subscription_id": "sub_1", "invoice_id": "inv_1" })),
            &mut counts,
        )
        .expect("store payment");

        let stored: (String, String, i64, Option<i64>, String, String) = conn
            .query_row(
                "SELECT payment_type, status, amount, subscription_id, interval_unit,
                        provider_invoice_id
                 FROM payments WHERE provider = 'dodo' AND provider_payment_id = 'pay_1'",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .expect("payment");
        assert_eq!(
            stored,
            (
                "subscription".to_string(),
                "ok".to_string(),
                29_000,
                Some(row_id),
                "month".to_string(),
                "inv_1".to_string(),
            )
        );
    }

    #[test]
    fn a_refund_without_an_amount_or_currency_is_skipped() {
        let conn = open_migrated_db();
        let mut counts = SyncCounts::default();
        let refund = |amount: Value, currency: Value| -> RefundListItem {
            serde_json::from_value(json!({
                "business_id": "biz_1",
                "created_at": "2026-01-03T00:00:00Z",
                "is_partial": false,
                "payment_id": "pay_1",
                "refund_id": "ref_1",
                "status": "succeeded",
                "amount": amount,
                "currency": currency,
            }))
            .expect("refund")
        };

        store_refund_payment(&conn, 1, &refund(Value::Null, json!("USD")), &mut counts)
            .expect("no amount");
        store_refund_payment(&conn, 1, &refund(json!(500), Value::Null), &mut counts)
            .expect("no currency");
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM payments", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 0);

        store_refund_payment(&conn, 1, &refund(json!(500), json!("USD")), &mut counts)
            .expect("refund");
        let stored: (String, String, i64, String) = conn
            .query_row(
                "SELECT payment_type, status, amount, parent_provider_payment_id FROM payments
                 WHERE provider_payment_id = 'ref_1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("refund payment");
        assert_eq!(
            stored,
            (
                "refund".to_string(),
                "ok".to_string(),
                -5_000,
                "pay_1".to_string()
            )
        );
    }

    #[test]
    fn provider_statuses_map_to_payment_statuses() {
        let intent =
            |value: &str| -> IntentStatus { serde_json::from_value(json!(value)).unwrap() };
        for (status, expected) in [
            ("succeeded", PaymentStatus::Ok),
            ("partially_captured", PaymentStatus::Ok),
            ("partially_captured_and_capturable", PaymentStatus::Ok),
            ("failed", PaymentStatus::Failed),
            ("cancelled", PaymentStatus::Canceled),
            ("processing", PaymentStatus::Pending),
            ("requires_capture", PaymentStatus::Pending),
            ("something_new", PaymentStatus::Pending),
        ] {
            assert_eq!(payment_status(Some(&intent(status))), expected, "{status}");
        }
        assert_eq!(payment_status(None), PaymentStatus::Pending);

        let refund =
            |value: &str| -> RefundStatus { serde_json::from_value(json!(value)).unwrap() };
        assert_eq!(refund_status(&refund("succeeded")), PaymentStatus::Ok);
        assert_eq!(refund_status(&refund("failed")), PaymentStatus::Failed);
        assert_eq!(refund_status(&refund("review")), PaymentStatus::Pending);
    }

    #[test]
    fn intervals_are_lowercased_and_unknown_ones_default_to_monthly() {
        for (interval, expected) in [
            ("Day", "day"),
            ("Week", "week"),
            ("Month", "month"),
            ("Year", "year"),
            ("Fortnight", "month"),
        ] {
            let interval: TimeInterval = serde_json::from_value(json!(interval)).unwrap();
            assert_eq!(interval_unit(&interval), expected);
        }
    }

    #[test]
    fn incremental_pages_filter_on_creation_time() {
        assert_eq!(
            page_query(2, None).unwrap(),
            json!({ "page_size": DODO_PAGE_SIZE, "page_number": 2 })
        );
        assert_eq!(
            page_query(0, Some(1_767_225_600)).unwrap()["created_at_gte"],
            json!("2026-01-01T00:00:00Z")
        );
        assert!(timestamp_to_unix("yesterday").is_err());
    }
}
