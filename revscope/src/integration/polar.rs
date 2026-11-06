use std::sync::Arc;

use log::info;
use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

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
use crate::util::rfc3339_to_unix;

const PROVIDER: &str = "polar";
const POLAR_PAGE_LIMIT: i64 = 100;
const POLAR_API_BASE_URL: &str = "https://api.polar.sh";

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
    let access_token = decrypt_secret(
        integration.encrypted_secret.as_deref(),
        SyncError::MissingPolarSecret,
        || SyncError::InvalidPolarSecret,
    )?;
    let client = PolarClient::new(&server_context.http, &access_token);
    let since = Some(integration.since());

    let mut counts = SyncCounts::default();
    let external_account_id = integration.external_account_id(PROVIDER);

    sync_subscriptions(
        &server_context,
        &project,
        &client,
        None,
        &external_account_id,
        &mut counts,
    )
    .await?;
    sync_orders(
        &server_context,
        &project,
        &client,
        None,
        &external_account_id,
        since,
        &mut counts,
    )
    .await?;
    sync_refunds(
        &server_context,
        &project,
        &client,
        None,
        &external_account_id,
        since,
        &mut counts,
    )
    .await?;
    if integration.sync_kind == SyncKind::Full {
        attribution::attribute_full_sync(server_context.clone(), &project, "polar").await?;
    }

    info!(
        "synced Polar integration {} ({}): inserted={}, updated={}, deleted={}",
        integration.id,
        integration.sync_kind.as_str(),
        counts.records_inserted,
        counts.records_updated,
        counts.records_deleted
    );
    Ok(counts)
}

struct PolarClient {
    http: reqwest::Client,
    api_base_url: String,
    access_token: String,
}

impl PolarClient {
    fn new(http: &reqwest::Client, access_token: &str) -> Self {
        Self {
            http: http.clone(),
            api_base_url: POLAR_API_BASE_URL.to_string(),
            access_token: access_token.to_string(),
        }
    }

    async fn get_page<T>(
        &self,
        path: &str,
        mut query: Vec<(&'static str, String)>,
        page: i64,
    ) -> SyncResult<PolarList<T>>
    where
        T: DeserializeOwned,
    {
        query.push(("page", page.to_string()));
        query.push(("limit", POLAR_PAGE_LIMIT.to_string()));

        let response = self
            .http
            .get(self.endpoint_url(path, &query)?)
            .bearer_auth(&self.access_token)
            .send()
            .await?;
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            return Err(polar_http_error(status, path, text));
        }
        Ok(serde_json::from_str(&text)?)
    }

    fn endpoint_url(&self, path: &str, query: &[(&'static str, String)]) -> SyncResult<String> {
        let url = if self.api_base_url.ends_with("/v1") && path.starts_with("/v1/") {
            format!("{}{}", self.api_base_url, &path[3..])
        } else {
            format!("{}{}", self.api_base_url, path)
        };
        let mut url = Url::parse(&url)
            .map_err(|error| SyncError::Polar(format!("invalid Polar API URL {url}: {error}")))?;
        {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in query {
                pairs.append_pair(key, value);
            }
        }
        Ok(url.to_string())
    }
}

fn polar_http_error(status: StatusCode, path: &str, body: String) -> SyncError {
    SyncError::Polar(format!(
        "Polar request to {path} failed with {status}: {body}"
    ))
}

/// Lists all subscriptions without a date lower bound.
///
/// Polar's list endpoint has no `created_at`/`started_at`/`modified_at` filter
/// (only sort by `-started_at`, plus cancellation-date bounds). We could stop
/// paging once `started_at` falls below `since`, but that would miss status and
/// other updates on long-lived subscriptions, so we intentionally fetch the
/// full list every sync.
async fn sync_subscriptions(
    server_context: &ServerContext,
    project: &Arc<ProjectCache>,
    client: &PolarClient,
    organization_id: Option<&str>,
    external_account_id: &str,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    let mut page_number = 1;
    loop {
        let page = client
            .get_page::<PolarSubscription>(
                "/v1/subscriptions/",
                list_query(organization_id, "-started_at"),
                page_number,
            )
            .await?;
        {
            let conn = crate::db::open_project_database_write_for(server_context, project)?;
            for subscription in page.items {
                let object_created_at = parse_timestamp(&subscription.created_at)?;
                let object_updated_at =
                    parse_optional_timestamp(subscription.modified_at.as_deref())?;
                let stored = store_provider_object(
                    &conn,
                    PROVIDER,
                    external_account_id,
                    ProviderObjectRef {
                        object_type: "subscription",
                        object_id: &subscription.id,
                        created_at: Some(object_created_at),
                        updated_at: object_updated_at,
                    },
                    &subscription,
                    counts,
                )?;
                if stored.changed {
                    let subscription_id =
                        store_subscription(&conn, Some(stored.id), &subscription, counts)?;
                    attribution::attribute_subscription_by_id(&conn, subscription_id)?;
                }
            }
        }

        if page_number >= page.pagination.max_page {
            break;
        }
        page_number += 1;
    }
    Ok(())
}

async fn sync_orders(
    server_context: &ServerContext,
    project: &Arc<ProjectCache>,
    client: &PolarClient,
    organization_id: Option<&str>,
    external_account_id: &str,
    since: Option<i64>,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    let mut page_number = 1;
    loop {
        let page = client
            .get_page::<PolarOrder>(
                "/v1/orders/",
                list_query(organization_id, "-created_at"),
                page_number,
            )
            .await?;
        let mut reached_old_items = false;
        {
            let conn = crate::db::open_project_database_write_for(server_context, project)?;
            for order in page.items {
                let object_created_at = parse_timestamp(&order.created_at)?;
                if since.is_some_and(|since| object_created_at < since) {
                    reached_old_items = true;
                    break;
                }

                let object_updated_at = parse_optional_timestamp(order.modified_at.as_deref())?;
                let stored = store_provider_object(
                    &conn,
                    PROVIDER,
                    external_account_id,
                    ProviderObjectRef {
                        object_type: "order",
                        object_id: &order.id,
                        created_at: Some(object_created_at),
                        updated_at: object_updated_at,
                    },
                    &order,
                    counts,
                )?;
                if stored.changed {
                    let provider_payment_id = order.id.clone();
                    store_order_payment(&conn, stored.id, &order, counts)?;
                    attribution::attribute_payment_by_provider_id(
                        &conn,
                        PROVIDER,
                        &provider_payment_id,
                    )?;
                }
            }
        }

        if reached_old_items || page_number >= page.pagination.max_page {
            break;
        }
        page_number += 1;
    }
    Ok(())
}

async fn sync_refunds(
    server_context: &ServerContext,
    project: &Arc<ProjectCache>,
    client: &PolarClient,
    organization_id: Option<&str>,
    external_account_id: &str,
    since: Option<i64>,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    let mut page_number = 1;
    loop {
        let page = client
            .get_page::<PolarRefund>(
                "/v1/refunds/",
                list_query(organization_id, "-created_at"),
                page_number,
            )
            .await?;
        let mut reached_old_items = false;
        {
            let conn = crate::db::open_project_database_write_for(server_context, project)?;
            for refund in page.items {
                let object_created_at = parse_timestamp(&refund.created_at)?;
                if since.is_some_and(|since| object_created_at < since) {
                    reached_old_items = true;
                    break;
                }

                let object_updated_at = parse_optional_timestamp(refund.modified_at.as_deref())?;
                let stored = store_provider_object(
                    &conn,
                    PROVIDER,
                    external_account_id,
                    ProviderObjectRef {
                        object_type: "refund",
                        object_id: &refund.id,
                        created_at: Some(object_created_at),
                        updated_at: object_updated_at,
                    },
                    &refund,
                    counts,
                )?;
                if stored.changed {
                    let provider_payment_id = refund.id.clone();
                    store_refund_payment(&conn, stored.id, &refund, counts)?;
                    attribution::attribute_payment_by_provider_id(
                        &conn,
                        PROVIDER,
                        &provider_payment_id,
                    )?;
                }
            }
        }

        if reached_old_items || page_number >= page.pagination.max_page {
            break;
        }
        page_number += 1;
    }
    Ok(())
}

fn list_query(organization_id: Option<&str>, sorting: &'static str) -> Vec<(&'static str, String)> {
    let mut query = vec![("sorting", sorting.to_string())];
    if let Some(organization_id) = organization_id {
        query.push(("organization_id", organization_id.to_string()));
    }
    query
}

/// The subscription state Polar reports, from either the subscriptions list or
/// the copy embedded in an order.
struct PolarSubscriptionState<'a> {
    id: &'a str,
    customer_id: &'a str,
    customer_email: Option<&'a str>,
    /// Checkout that created the subscription, when Polar still has it.
    checkout_id: Option<&'a str>,
    status: &'a str,
    currency: String,
    amount: i64,
    interval_unit: &'a str,
    interval_count: i64,
    current_period_start: i64,
    current_period_end: i64,
    started_at: i64,
    ended_at: Option<i64>,
    canceled_at: Option<i64>,
    cancel_at: Option<i64>,
    metadata: Option<String>,
}

impl<'a> PolarSubscriptionState<'a> {
    fn from_subscription(subscription: &'a PolarSubscription) -> SyncResult<Self> {
        Ok(Self {
            id: &subscription.id,
            customer_id: &subscription.customer_id,
            customer_email: subscription
                .customer
                .as_ref()
                .and_then(|customer| customer.email.as_deref()),
            checkout_id: subscription.checkout_id.as_deref(),
            status: &subscription.status,
            amount: minor_units_to_thousandths(subscription.amount, &subscription.currency),
            currency: currency_code(&subscription.currency),
            interval_unit: &subscription.recurring_interval,
            interval_count: subscription.recurring_interval_count,
            current_period_start: parse_timestamp(&subscription.current_period_start)?,
            current_period_end: parse_timestamp(&subscription.current_period_end)?,
            started_at: parse_optional_timestamp(subscription.started_at.as_deref())?
                .unwrap_or(parse_timestamp(&subscription.created_at)?),
            ended_at: parse_optional_timestamp(subscription.ended_at.as_deref())?,
            canceled_at: parse_optional_timestamp(subscription.canceled_at.as_deref())?,
            cancel_at: parse_optional_timestamp(subscription.ends_at.as_deref())?,
            metadata: serialize_metadata(&subscription.metadata)?,
        })
    }

    /// `order_checkout_id` fills in when the embedded subscription omits it.
    fn from_order_subscription(
        subscription: &'a PolarOrderSubscription,
        customer: Option<&'a PolarCustomer>,
        order_checkout_id: Option<&'a str>,
    ) -> SyncResult<Self> {
        Ok(Self {
            id: &subscription.id,
            customer_id: &subscription.customer_id,
            customer_email: customer.and_then(|customer| customer.email.as_deref()),
            checkout_id: subscription.checkout_id.as_deref().or(order_checkout_id),
            status: &subscription.status,
            amount: minor_units_to_thousandths(subscription.amount, &subscription.currency),
            currency: currency_code(&subscription.currency),
            interval_unit: &subscription.recurring_interval,
            interval_count: subscription.recurring_interval_count,
            current_period_start: parse_timestamp(&subscription.current_period_start)?,
            current_period_end: parse_timestamp(&subscription.current_period_end)?,
            started_at: parse_optional_timestamp(subscription.started_at.as_deref())?
                .unwrap_or(parse_timestamp(&subscription.created_at)?),
            ended_at: parse_optional_timestamp(subscription.ended_at.as_deref())?,
            canceled_at: parse_optional_timestamp(subscription.canceled_at.as_deref())?,
            cancel_at: parse_optional_timestamp(subscription.ends_at.as_deref())?,
            metadata: serialize_metadata(&subscription.metadata)?,
        })
    }
}

fn store_subscription(
    conn: &ProjectConnection,
    provider_object_id: Option<i64>,
    subscription: &PolarSubscription,
    counts: &mut SyncCounts,
) -> SyncResult<i64> {
    let state = PolarSubscriptionState::from_subscription(subscription)?;
    store_subscription_state(conn, provider_object_id, &state, counts)
}

fn store_subscription_state(
    conn: &ProjectConnection,
    provider_object_id: Option<i64>,
    state: &PolarSubscriptionState<'_>,
    counts: &mut SyncCounts,
) -> SyncResult<i64> {
    let mut upsert = SubscriptionUpsert::new(PROVIDER, state.id);
    upsert.provider_object_id = provider_object_id;
    upsert.provider_customer_id = Some(state.customer_id);
    upsert.provider_checkout_id = state.checkout_id;
    upsert.customer_email = state.customer_email;
    upsert.status = Some(state.status);
    upsert.plan = SubscriptionPlan {
        interval: Some(PlanInterval::new(state.interval_unit, state.interval_count)),
        provider_price_id: None,
        plan_name: None,
        quantity: None,
        currency: Some(&state.currency),
        amount: Some(state.amount),
    };
    upsert.current_period_start = Some(state.current_period_start);
    upsert.current_period_end = Some(state.current_period_end);
    upsert.started_at = Some(state.started_at);
    upsert.ended_at = Reported::or_cleared(state.ended_at);
    upsert.canceled_at = Reported::or_cleared(state.canceled_at);
    upsert.cancel_at = Reported::or_cleared(state.cancel_at);
    upsert.metadata = state.metadata.as_deref();
    upsert_subscription(conn, &upsert, counts)
}

fn store_order_payment(
    conn: &ProjectConnection,
    provider_object_id: i64,
    order: &PolarOrder,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    let subscription_id = order_subscription_row_id(conn, order, counts)?;
    let metadata = serialize_metadata(&order.metadata)?;
    let amount = minor_units_to_thousandths(order.total_amount, &order.currency);
    let fee_amount = order
        .platform_fee_amount
        .map(|fee_amount| minor_units_to_thousandths(fee_amount, &order.currency));
    let currency = currency_code(&order.currency);
    let paid_at = parse_timestamp(&order.created_at)?;
    let payment_type = if subscription_id.is_some()
        || order
            .billing_reason
            .as_deref()
            .is_some_and(|reason| reason.starts_with("subscription_"))
    {
        PaymentType::Subscription
    } else {
        PaymentType::Single
    };

    let mut upsert = PaymentUpsert::new(
        PROVIDER,
        &order.id,
        payment_type,
        order_payment_status(order),
        amount,
        &currency,
        paid_at,
    );
    upsert.provider_object_id = Some(provider_object_id);
    upsert.provider_object_type = Some("order");
    upsert.subscription_id = subscription_id;
    upsert.interval = subscription_id
        .map(|subscription_id| subscription_interval(conn, subscription_id))
        .transpose()?
        .flatten();
    upsert.provider_status = order.status.as_deref();
    upsert.provider_order_id = Some(&order.id);
    upsert.provider_checkout_id = order.checkout_id.as_deref();
    upsert.provider_invoice_id = order.invoice_number.as_deref();
    upsert.provider_customer_id = Some(&order.customer_id);
    upsert.customer_email = order
        .customer
        .as_ref()
        .and_then(|customer| customer.email.as_deref());
    upsert.fee_amount = fee_amount;
    upsert.net_amount = fee_amount.map(|fee_amount| amount.saturating_sub(fee_amount));
    upsert.metadata = metadata.as_deref();
    upsert_payment(conn, &upsert, counts)
}

fn store_refund_payment(
    conn: &ProjectConnection,
    provider_object_id: i64,
    refund: &PolarRefund,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    let metadata = serialize_metadata(&refund.metadata)?;
    let currency = currency_code(&refund.currency);
    let paid_at = parse_timestamp(&refund.created_at)?;
    let subscription_id = refund
        .subscription_id
        .as_deref()
        .map(|id| subscription_row_id(conn, PROVIDER, id))
        .transpose()?
        .flatten();

    let mut upsert = PaymentUpsert::new(
        PROVIDER,
        &refund.id,
        PaymentType::Refund,
        refund_payment_status(refund.status.as_deref()),
        minor_units_to_thousandths(
            refund.amount.saturating_add(refund.tax_amount),
            &refund.currency,
        ),
        &currency,
        paid_at,
    );
    upsert.provider_object_id = Some(provider_object_id);
    upsert.provider_object_type = Some("refund");
    upsert.subscription_id = subscription_id;
    upsert.provider_status = refund.status.as_deref();
    upsert.provider_order_id = Some(&refund.order_id);
    upsert.provider_refund_id = Some(&refund.id);
    upsert.parent_provider_payment_id = Some(&refund.order_id);
    upsert.provider_customer_id = Some(&refund.customer_id);
    upsert.metadata = metadata.as_deref();
    upsert_payment(conn, &upsert, counts)
}

/// Resolves the subscription an order belongs to, storing the copy embedded in
/// the order when the subscriptions list hasn't reached it yet.
fn order_subscription_row_id(
    conn: &ProjectConnection,
    order: &PolarOrder,
    counts: &mut SyncCounts,
) -> SyncResult<Option<i64>> {
    if let Some(subscription_id) = order.subscription_id.as_deref()
        && let Some(row_id) = subscription_row_id(conn, PROVIDER, subscription_id)?
    {
        return Ok(Some(row_id));
    }

    if let Some(subscription) = order.subscription.as_ref() {
        let state = PolarSubscriptionState::from_order_subscription(
            subscription,
            order.customer.as_ref(),
            order.checkout_id.as_deref(),
        )?;
        return Ok(Some(store_subscription_state(conn, None, &state, counts)?));
    }

    Ok(None)
}

fn order_payment_status(order: &PolarOrder) -> PaymentStatus {
    match order.status.as_deref() {
        Some("paid") | Some("refunded") | Some("partially_refunded") if order.paid => {
            PaymentStatus::Ok
        }
        Some("void") => PaymentStatus::Canceled,
        Some("pending") | Some("draft") => PaymentStatus::Pending,
        _ if order.paid => PaymentStatus::Ok,
        _ => PaymentStatus::Pending,
    }
}

fn refund_payment_status(status: Option<&str>) -> PaymentStatus {
    match status {
        Some("succeeded") => PaymentStatus::Ok,
        Some("canceled") => PaymentStatus::Canceled,
        _ => PaymentStatus::Pending,
    }
}

fn currency_code(currency: &str) -> String {
    super::store::currency_code(currency)
}

fn parse_timestamp(value: &str) -> SyncResult<i64> {
    rfc3339_to_unix(value)
        .map_err(|error| SyncError::Polar(format!("invalid Polar timestamp {value}: {error}")))
}

fn parse_optional_timestamp(value: Option<&str>) -> SyncResult<Option<i64>> {
    value.map(parse_timestamp).transpose()
}

#[derive(Debug, Deserialize)]
struct PolarList<T> {
    items: Vec<T>,
    pagination: PolarPagination,
}

#[derive(Debug, Deserialize)]
struct PolarPagination {
    max_page: i64,
}

#[derive(Debug, Deserialize, Serialize)]
struct PolarCustomer {
    id: String,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    external_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct PolarSubscription {
    id: String,
    created_at: String,
    #[serde(default)]
    modified_at: Option<String>,
    #[serde(default)]
    amount: i64,
    currency: String,
    recurring_interval: String,
    #[serde(default = "default_interval_count")]
    recurring_interval_count: i64,
    status: String,
    current_period_start: String,
    current_period_end: String,
    #[serde(default)]
    started_at: Option<String>,
    #[serde(default)]
    ended_at: Option<String>,
    /// When a scheduled cancellation takes effect. Polar sets it alongside
    /// `cancel_at_period_end` and clears it again if the customer uncancels.
    #[serde(default)]
    ends_at: Option<String>,
    #[serde(default)]
    canceled_at: Option<String>,
    customer_id: String,
    /// Present when the subscription was created through a Polar checkout.
    #[serde(default)]
    checkout_id: Option<String>,
    #[serde(default)]
    customer: Option<PolarCustomer>,
    #[serde(default)]
    metadata: Option<Value>,
}

#[derive(Debug, Deserialize, Serialize)]
struct PolarOrder {
    id: String,
    created_at: String,
    #[serde(default)]
    modified_at: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    paid: bool,
    #[serde(default)]
    total_amount: i64,
    currency: String,
    #[serde(default)]
    billing_reason: Option<String>,
    customer_id: String,
    #[serde(default)]
    customer: Option<PolarCustomer>,
    #[serde(default)]
    subscription_id: Option<String>,
    /// Checkout that produced this order, when one did.
    #[serde(default)]
    checkout_id: Option<String>,
    #[serde(default)]
    invoice_number: Option<String>,
    #[serde(default)]
    platform_fee_amount: Option<i64>,
    #[serde(default)]
    subscription: Option<PolarOrderSubscription>,
    #[serde(default)]
    metadata: Option<Value>,
}

#[derive(Debug, Deserialize, Serialize)]
struct PolarOrderSubscription {
    id: String,
    created_at: String,
    #[serde(default)]
    modified_at: Option<String>,
    #[serde(default)]
    amount: i64,
    currency: String,
    recurring_interval: String,
    #[serde(default = "default_interval_count")]
    recurring_interval_count: i64,
    status: String,
    current_period_start: String,
    current_period_end: String,
    #[serde(default)]
    started_at: Option<String>,
    #[serde(default)]
    ended_at: Option<String>,
    #[serde(default)]
    ends_at: Option<String>,
    #[serde(default)]
    canceled_at: Option<String>,
    customer_id: String,
    #[serde(default)]
    checkout_id: Option<String>,
    #[serde(default)]
    metadata: Option<Value>,
}

#[derive(Debug, Deserialize, Serialize)]
struct PolarRefund {
    id: String,
    created_at: String,
    #[serde(default)]
    modified_at: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    amount: i64,
    #[serde(default)]
    tax_amount: i64,
    currency: String,
    order_id: String,
    #[serde(default)]
    subscription_id: Option<String>,
    customer_id: String,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    metadata: Option<Value>,
}

fn default_interval_count() -> i64 {
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::open_migrated_db;

    use serde_json::json;

    fn subscription_json() -> Value {
        json!({
            "id": "sub_1",
            "created_at": "2026-01-01T00:00:00Z",
            "amount": 1900,
            "currency": "usd",
            "recurring_interval": "month",
            "status": "active",
            "current_period_start": "2026-01-01T00:00:00Z",
            "current_period_end": "2026-02-01T00:00:00Z",
            "customer_id": "cus_1",
            "checkout_id": "co_1",
            "customer": { "id": "cus_1", "email": "buyer@example.com" },
        })
    }

    fn order(value: Value) -> PolarOrder {
        let mut order = json!({
            "id": "ord_1",
            "created_at": "2026-01-01T00:00:05Z",
            "status": "paid",
            "paid": true,
            "total_amount": 1900,
            "currency": "usd",
            "customer_id": "cus_1",
        });
        for (key, value) in value.as_object().expect("order fields") {
            order[key] = value.clone();
        }
        serde_json::from_value(order).expect("order")
    }

    fn store_order(conn: &ProjectConnection, order: &PolarOrder, counts: &mut SyncCounts) {
        let stored = store_provider_object(
            conn,
            PROVIDER,
            "org_1",
            ProviderObjectRef {
                object_type: "order",
                object_id: &order.id,
                created_at: Some(parse_timestamp(&order.created_at).unwrap()),
                updated_at: None,
            },
            order,
            counts,
        )
        .expect("store provider object");
        store_order_payment(conn, stored.id, order, counts).expect("store order payment");
    }

    #[test]
    fn an_order_stores_the_subscription_it_embeds_when_the_list_has_not_reached_it() {
        let conn = open_migrated_db();
        let mut counts = SyncCounts::default();
        let mut embedded = subscription_json();
        embedded.as_object_mut().unwrap().remove("customer");
        embedded.as_object_mut().unwrap().remove("checkout_id");
        let order = order(json!({
            "subscription_id": "sub_1",
            "subscription": embedded,
            "checkout_id": "co_from_order",
            "billing_reason": "subscription_create",
            "customer": { "id": "cus_1", "email": "buyer@example.com" },
            "platform_fee_amount": 120,
        }));

        store_order(&conn, &order, &mut counts);

        let subscription: (String, String, i64, String, i64, Option<String>) = conn
            .query_row(
                "SELECT provider_checkout_id, customer_email, plan_amount, interval_unit,
                        interval_count, currency
                 FROM subscriptions WHERE provider = 'polar' AND provider_subscription_id = 'sub_1'",
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
            .expect("subscription stored from the order");
        assert_eq!(subscription.0, "co_from_order");
        assert_eq!(subscription.1, "buyer@example.com");
        assert_eq!(subscription.2, 19_000);
        assert_eq!((subscription.3.as_str(), subscription.4), ("month", 1));
        assert_eq!(subscription.5.as_deref(), Some("USD"));

        let payment: (String, String, i64, i64, i64, String, Option<i64>) = conn
            .query_row(
                "SELECT payment_type, status, amount, fee_amount, net_amount, interval_unit,
                        subscription_id
                 FROM payments WHERE provider = 'polar' AND provider_payment_id = 'ord_1'",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ))
                },
            )
            .expect("payment");
        assert_eq!(payment.0, "subscription");
        assert_eq!(payment.1, "ok");
        assert_eq!((payment.2, payment.3, payment.4), (19_000, 1_200, 17_800));
        assert_eq!(payment.5, "month");
        assert!(payment.6.is_some());
    }

    #[test]
    fn an_order_links_to_a_subscription_already_synced_without_overwriting_it() {
        let conn = open_migrated_db();
        let mut counts = SyncCounts::default();
        let subscription: PolarSubscription =
            serde_json::from_value(subscription_json()).expect("subscription");
        let row_id = store_subscription(&conn, None, &subscription, &mut counts).expect("store");

        let mut stale = subscription_json();
        stale["amount"] = json!(500);
        let order = order(json!({ "subscription_id": "sub_1", "subscription": stale }));
        store_order(&conn, &order, &mut counts);

        let (linked, plan_amount, rows): (i64, i64, i64) = conn
            .query_row(
                "SELECT p.subscription_id, s.plan_amount,
                        (SELECT COUNT(*) FROM subscriptions)
                 FROM payments p JOIN subscriptions s ON s.id = p.subscription_id
                 WHERE p.provider_payment_id = 'ord_1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("linked payment");
        assert_eq!(linked, row_id);
        assert_eq!(
            plan_amount, 19_000,
            "the subscriptions list is authoritative"
        );
        assert_eq!(rows, 1);
    }

    #[test]
    fn a_one_time_order_is_a_single_payment() {
        let conn = open_migrated_db();
        let mut counts = SyncCounts::default();
        let order = order(json!({ "billing_reason": "purchase" }));
        store_order(&conn, &order, &mut counts);

        let (payment_type, subscription_id, fee): (String, Option<i64>, Option<i64>) = conn
            .query_row(
                "SELECT payment_type, subscription_id, fee_amount FROM payments
                 WHERE provider_payment_id = 'ord_1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("payment");
        assert_eq!(payment_type, "single");
        assert_eq!(subscription_id, None);
        assert_eq!(fee, None);
    }

    #[test]
    fn a_refund_counts_its_tax_and_points_at_the_order_it_refunds() {
        // Refunds are stored as negative amounts so revenue sums net them out.
        let conn = open_migrated_db();
        let mut counts = SyncCounts::default();
        let refund: PolarRefund = serde_json::from_value(json!({
            "id": "ref_1",
            "created_at": "2026-01-03T00:00:00Z",
            "status": "succeeded",
            "amount": 1000,
            "tax_amount": 200,
            "currency": "eur",
            "order_id": "ord_1",
            "customer_id": "cus_1",
        }))
        .expect("refund");

        store_refund_payment(&conn, 1, &refund, &mut counts).expect("store refund");

        let row: (String, String, i64, String, String, String) = conn
            .query_row(
                "SELECT payment_type, status, amount, currency, parent_provider_payment_id,
                        provider_refund_id
                 FROM payments WHERE provider_payment_id = 'ref_1'",
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
            .expect("refund payment");
        assert_eq!(
            row,
            (
                "refund".to_string(),
                "ok".to_string(),
                -12_000,
                "EUR".to_string(),
                "ord_1".to_string(),
                "ref_1".to_string(),
            )
        );
    }

    #[test]
    fn a_cancellation_polar_withdraws_is_cleared_on_the_next_sync() {
        let conn = open_migrated_db();
        let mut counts = SyncCounts::default();
        let mut scheduled = subscription_json();
        scheduled["ends_at"] = json!("2026-02-01T00:00:00Z");
        scheduled["canceled_at"] = json!("2026-01-15T00:00:00Z");
        let scheduled: PolarSubscription = serde_json::from_value(scheduled).unwrap();
        store_subscription(&conn, None, &scheduled, &mut counts).expect("store scheduled");

        let uncanceled: PolarSubscription = serde_json::from_value(subscription_json()).unwrap();
        store_subscription(&conn, None, &uncanceled, &mut counts).expect("store uncanceled");

        let (cancel_at, canceled_at): (Option<i64>, Option<i64>) = conn
            .query_row(
                "SELECT cancel_at, canceled_at FROM subscriptions
                 WHERE provider_subscription_id = 'sub_1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("subscription");
        assert_eq!((cancel_at, canceled_at), (None, None));
    }

    #[test]
    fn order_status_maps_to_a_payment_status() {
        for (status, paid, expected) in [
            (Some("paid"), true, PaymentStatus::Ok),
            (Some("refunded"), true, PaymentStatus::Ok),
            (Some("partially_refunded"), true, PaymentStatus::Ok),
            (Some("paid"), false, PaymentStatus::Pending),
            (Some("void"), false, PaymentStatus::Canceled),
            (Some("pending"), false, PaymentStatus::Pending),
            (Some("draft"), false, PaymentStatus::Pending),
            (None, true, PaymentStatus::Ok),
            (None, false, PaymentStatus::Pending),
        ] {
            let mut order = order(json!({ "paid": paid }));
            order.status = status.map(str::to_string);
            assert_eq!(
                order_payment_status(&order),
                expected,
                "{status:?} paid={paid}"
            );
        }
        assert_eq!(refund_payment_status(Some("succeeded")), PaymentStatus::Ok);
        assert_eq!(
            refund_payment_status(Some("canceled")),
            PaymentStatus::Canceled
        );
        assert_eq!(
            refund_payment_status(Some("pending")),
            PaymentStatus::Pending
        );
        assert_eq!(refund_payment_status(None), PaymentStatus::Pending);
    }

    #[test]
    fn timestamps_must_be_rfc3339() {
        assert_eq!(
            parse_timestamp("2026-01-01T00:00:00Z").unwrap(),
            1_767_225_600
        );
        assert_eq!(
            parse_timestamp("2026-01-01T01:00:00+01:00").unwrap(),
            1_767_225_600
        );
        assert!(parse_timestamp("2026-01-01").is_err());
        assert_eq!(parse_optional_timestamp(None).unwrap(), None);
    }
}
