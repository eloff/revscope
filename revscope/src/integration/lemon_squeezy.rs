use std::sync::Arc;

use fxhash::FxHashMap;
use log::info;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::attribution;
use super::schedule::{
    self, PaymentIntegration, SyncOutcome, decrypt_secret, load_due_payment_integrations,
};
use super::store::{
    PaymentStatus, PaymentType, PaymentUpsert, PlanInterval, ProviderObjectRef, Reported,
    SubscriptionPlan, SubscriptionUpsert, store_provider_object, subscription_interval,
    subscription_row_id, upsert_payment, upsert_subscription,
};
use super::sync::{SyncCounts, SyncError, SyncKind, SyncResult, serialize_metadata};
use crate::db::{ProjectCache, ProjectConnection};
use crate::server_context::ServerContext;
use crate::subscription_status;
use crate::util::{rfc3339_to_unix, unix_seconds};

const PROVIDER: &str = "lemon_squeezy";
const LEMON_SQUEEZY_API_BASE_URL: &str = "https://api.lemonsqueezy.com/v1";
const LEMON_SQUEEZY_PAGE_SIZE: i64 = 100;
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
        SyncError::MissingLemonSqueezySecret,
        || SyncError::InvalidLemonSqueezySecret,
    )?;
    let client = LemonSqueezyClient::new(&server_context.http, api_key);
    let since = Some(integration.since());

    let mut counts = SyncCounts::default();
    let external_account_id = integration.external_account_id(PROVIDER);
    let mut price_cache = FxHashMap::default();

    sync_subscriptions(
        &server_context,
        &project,
        &client,
        &external_account_id,
        since,
        &mut price_cache,
        &mut counts,
    )
    .await?;
    sync_subscription_invoices(
        &server_context,
        &project,
        &client,
        &external_account_id,
        since,
        &mut counts,
    )
    .await?;
    if integration.sync_kind == SyncKind::Full {
        attribution::attribute_full_sync(server_context.clone(), &project, "lemon_squeezy").await?;
    }

    info!(
        "synced Lemon Squeezy integration {} ({}): inserted={}, updated={}, deleted={}",
        integration.id,
        integration.sync_kind.as_str(),
        counts.records_inserted,
        counts.records_updated,
        counts.records_deleted
    );
    Ok(counts)
}

async fn sync_subscriptions(
    server_context: &ServerContext,
    project: &Arc<ProjectCache>,
    client: &LemonSqueezyClient,
    fallback_external_account_id: &str,
    since: Option<i64>,
    price_cache: &mut FxHashMap<String, LemonSqueezyPriceAttributes>,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    let mut page_number = 1;
    loop {
        let page = client.list_page("/subscriptions", page_number).await?;
        let page_len = page.data.len();
        let last_page = page
            .meta
            .as_ref()
            .map(|meta| meta.page.last_page)
            .unwrap_or(page_number);
        let mut stop_after_page = false;
        let mut rows = Vec::with_capacity(page_len);

        for resource in page.data {
            let attrs: LemonSqueezySubscriptionAttributes = resource.attributes()?;
            let created_at = parse_lemon_squeezy_time(&attrs.created_at)?;
            if since.is_some_and(|since| created_at < since) {
                stop_after_page = true;
                continue;
            }
            let updated_at = parse_optional_lemon_squeezy_time(attrs.updated_at.as_deref())?;
            let price = subscription_price(client, price_cache, &attrs).await?;
            rows.push(LemonSqueezySubscriptionRow {
                resource,
                attrs,
                created_at,
                updated_at,
                price,
            });
        }

        {
            let conn = crate::db::open_project_database_write_for(server_context, project)?;
            for row in rows {
                let external_account_id =
                    object_external_account_id(row.attrs.store_id, fallback_external_account_id);
                let stored = store_provider_object(
                    &conn,
                    PROVIDER,
                    &external_account_id,
                    ProviderObjectRef {
                        object_type: "subscription",
                        object_id: &row.resource.id,
                        created_at: Some(row.created_at),
                        updated_at: row.updated_at,
                    },
                    &row.resource,
                    counts,
                )?;
                if stored.changed {
                    let subscription_id = store_subscription(&conn, stored.id, &row, counts)?;
                    attribution::attribute_subscription_by_id(&conn, subscription_id)?;
                }
            }
        }

        if stop_after_page || page_len == 0 || page_number >= last_page {
            break;
        }
        page_number += 1;
    }
    Ok(())
}

async fn sync_subscription_invoices(
    server_context: &ServerContext,
    project: &Arc<ProjectCache>,
    client: &LemonSqueezyClient,
    fallback_external_account_id: &str,
    since: Option<i64>,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    let mut page_number = 1;
    loop {
        let page = client
            .list_page("/subscription-invoices", page_number)
            .await?;
        let page_len = page.data.len();
        let last_page = page
            .meta
            .as_ref()
            .map(|meta| meta.page.last_page)
            .unwrap_or(page_number);
        let mut stop_after_page = false;

        {
            let conn = crate::db::open_project_database_write_for(server_context, project)?;
            for resource in page.data {
                let attrs: LemonSqueezySubscriptionInvoiceAttributes = resource.attributes()?;
                let created_at = parse_lemon_squeezy_time(&attrs.created_at)?;
                if since.is_some_and(|since| created_at < since) {
                    stop_after_page = true;
                    continue;
                }
                let updated_at = parse_optional_lemon_squeezy_time(attrs.updated_at.as_deref())?;
                let external_account_id =
                    object_external_account_id(attrs.store_id, fallback_external_account_id);
                let stored = store_provider_object(
                    &conn,
                    PROVIDER,
                    &external_account_id,
                    ProviderObjectRef {
                        object_type: "subscription_invoice",
                        object_id: &resource.id,
                        created_at: Some(created_at),
                        updated_at,
                    },
                    &resource,
                    counts,
                )?;
                if stored.changed {
                    let provider_payment_id = subscription_invoice_payment_id(&resource.id);
                    store_subscription_invoice_payment(
                        &conn, stored.id, &resource, &attrs, created_at, updated_at, counts,
                    )?;
                    attribution::attribute_payment_by_provider_id(
                        &conn,
                        PROVIDER,
                        &provider_payment_id,
                    )?;
                    let provider_refund_id = subscription_invoice_refund_id(&resource.id);
                    store_subscription_invoice_refund(
                        &conn, stored.id, &resource, &attrs, updated_at, counts,
                    )?;
                    attribution::attribute_payment_by_provider_id(
                        &conn,
                        PROVIDER,
                        &provider_refund_id,
                    )?;
                }
            }
        }

        if stop_after_page || page_len == 0 || page_number >= last_page {
            break;
        }
        page_number += 1;
    }
    Ok(())
}

fn store_subscription(
    conn: &ProjectConnection,
    provider_object_id: i64,
    row: &LemonSqueezySubscriptionRow,
    counts: &mut SyncCounts,
) -> SyncResult<i64> {
    let customer_id = row.attrs.customer_id.map(|id| id.to_string());
    let order_id = row.attrs.order_id.map(|id| id.to_string());
    let current_period_end = parse_optional_lemon_squeezy_time(row.attrs.renews_at.as_deref())?;
    // Lemon Squeezy reports a single `ends_at` that means "expired on" or
    // "cancelled effective", depending on status. Either way it is the date the
    // subscription stops, never the date the cancellation was requested, which
    // Lemon Squeezy doesn't report at all.
    let ends_at = parse_optional_lemon_squeezy_time(row.attrs.ends_at.as_deref())?;
    let expired = row.attrs.status == "expired";
    let cancelled = row.attrs.cancelled || row.attrs.status == "cancelled";
    let ended_at = if expired { ends_at } else { None };
    let cancel_at = if cancelled { ends_at } else { None };
    // `cancelled` here is a grace period the customer has already paid for and
    // keeps their benefits through, unlike every other provider that uses the
    // word for a subscription that is over. Resolving that here rather than in
    // `subscription_status` keeps the shared mapping free of provider cases;
    // `expired` is the status that means this one is finally done.
    let status = if cancelled && !expired {
        subscription_status::ACTIVE
    } else {
        row.attrs.status.as_str()
    };
    let metadata = lemon_metadata(&row.resource)?;

    let mut upsert = SubscriptionUpsert::new(PROVIDER, &row.resource.id);
    upsert.provider_object_id = Some(provider_object_id);
    upsert.provider_order_id = order_id.as_deref();
    upsert.provider_customer_id = customer_id.as_deref();
    upsert.customer_email = row.attrs.user_email.as_deref();
    upsert.status = Some(status);
    upsert.plan = SubscriptionPlan {
        interval: row
            .price
            .interval
            .as_ref()
            .map(|(unit, count)| PlanInterval::new(unit, *count)),
        provider_price_id: row.price.id.as_deref(),
        plan_name: None,
        quantity: row.price.quantity,
        // The store's currency isn't on the subscription, only on its invoices,
        // so the plan amount is recorded without one.
        currency: None,
        amount: row.price.amount.map(lemon_amount_to_thousandths),
    };
    upsert.current_period_end = current_period_end;
    upsert.started_at = Some(row.created_at);
    upsert.ended_at = Reported::or_cleared(ended_at);
    upsert.cancel_at = Reported::or_cleared(cancel_at);
    upsert.metadata = metadata.as_deref();
    upsert_subscription(conn, &upsert, counts)
}

fn store_subscription_invoice_payment(
    conn: &ProjectConnection,
    provider_object_id: i64,
    resource: &LemonSqueezyResource,
    attrs: &LemonSqueezySubscriptionInvoiceAttributes,
    created_at: i64,
    updated_at: Option<i64>,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    let Some(subscription_id) =
        subscription_row_id(conn, PROVIDER, &attrs.subscription_id.to_string())?
    else {
        return Ok(());
    };
    let provider_payment_id = subscription_invoice_payment_id(&resource.id);
    let customer_id = attrs.customer_id.to_string();
    let metadata = lemon_metadata(resource)?;
    let currency = currency_code(&attrs.currency);
    let settlement = usd_settlement(attrs.total_usd, attrs.currency_rate.as_deref());

    let mut upsert = PaymentUpsert::new(
        PROVIDER,
        &provider_payment_id,
        PaymentType::Subscription,
        lemon_payment_status(&attrs.status, attrs.total),
        lemon_amount_to_thousandths(attrs.total.max(0)),
        &currency,
        updated_at.unwrap_or(created_at),
    );
    upsert.provider_object_id = Some(provider_object_id);
    upsert.provider_object_type = Some("subscription_invoice");
    upsert.subscription_id = Some(subscription_id);
    upsert.interval = subscription_interval(conn, subscription_id)?;
    upsert.provider_invoice_id = Some(&resource.id);
    upsert.provider_customer_id = Some(&customer_id);
    upsert.customer_email = attrs.user_email.as_deref();
    upsert.settlement_currency = settlement.currency.as_deref();
    upsert.settlement_amount = settlement.amount;
    upsert.exchange_rate = settlement.exchange_rate;
    upsert.created_at = Some(created_at);
    upsert.metadata = metadata.as_deref();
    upsert_payment(conn, &upsert, counts)
}

/// Lemon Squeezy reports refunds as an amount on the invoice rather than as its
/// own object, so the refund gets a derived payment id.
fn store_subscription_invoice_refund(
    conn: &ProjectConnection,
    provider_object_id: i64,
    resource: &LemonSqueezyResource,
    attrs: &LemonSqueezySubscriptionInvoiceAttributes,
    updated_at: Option<i64>,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    if attrs.refunded_amount <= 0 {
        return Ok(());
    }

    let provider_payment_id = subscription_invoice_refund_id(&resource.id);
    let parent_payment_id = subscription_invoice_payment_id(&resource.id);
    let customer_id = attrs.customer_id.to_string();
    let paid_at = parse_optional_lemon_squeezy_time(attrs.refunded_at.as_deref())?
        .or(updated_at)
        .unwrap_or_else(unix_seconds);
    let metadata = lemon_metadata(resource)?;
    let currency = currency_code(&attrs.currency);
    let settlement = usd_settlement(attrs.refunded_amount_usd, attrs.currency_rate.as_deref());

    let mut upsert = PaymentUpsert::new(
        PROVIDER,
        &provider_payment_id,
        PaymentType::Refund,
        PaymentStatus::Ok,
        lemon_amount_to_thousandths(attrs.refunded_amount),
        &currency,
        paid_at,
    );
    upsert.provider_object_id = Some(provider_object_id);
    upsert.provider_object_type = Some("subscription_invoice");
    upsert.provider_refund_id = Some(&provider_payment_id);
    upsert.parent_provider_payment_id = Some(&parent_payment_id);
    upsert.provider_customer_id = Some(&customer_id);
    upsert.customer_email = attrs.user_email.as_deref();
    upsert.settlement_currency = settlement.currency.as_deref();
    upsert.settlement_amount = settlement.amount;
    upsert.exchange_rate = settlement.exchange_rate;
    upsert.metadata = metadata.as_deref();
    upsert_payment(conn, &upsert, counts)
}

/// Resolves the plan a subscription is on. The billing interval and unit prices
/// live on the price object rather than the subscription, so prices are fetched
/// once and cached for the rest of the sync.
async fn subscription_price(
    client: &LemonSqueezyClient,
    price_cache: &mut FxHashMap<String, LemonSqueezyPriceAttributes>,
    attrs: &LemonSqueezySubscriptionAttributes,
) -> SyncResult<LemonSqueezyPrice> {
    let Some(item) = attrs.first_subscription_item.as_ref() else {
        return Ok(LemonSqueezyPrice::unknown());
    };
    let price_id = item.price_id.to_string();

    if !price_cache.contains_key(&price_id) {
        let price = client
            .retrieve_resource(&format!("/prices/{price_id}"))
            .await?;
        let price_attrs: LemonSqueezyPriceAttributes = price.attributes()?;
        price_cache.insert(price_id.clone(), price_attrs);
    }

    let price = price_cache
        .get(&price_id)
        .ok_or(SyncError::MissingUpsertedRow("prices"))?;
    Ok(LemonSqueezyPrice {
        id: Some(price_id),
        interval: Some((
            price
                .renewal_interval_unit
                .clone()
                .unwrap_or_else(|| "month".to_string()),
            price.renewal_interval_quantity.unwrap_or(1).max(1),
        )),
        // A usage-based item reports the usage accumulated so far in `quantity`
        // rather than a subscribed unit count, which is not a plan quantity.
        quantity: (!item.is_usage_based).then(|| item.units()),
        amount: period_amount(price, item),
    })
}

/// The recurring amount in cents charged for one billing period, which depends
/// on the price's scheme as well as on how many units are subscribed for.
///
/// Usage-based prices bill in arrears for usage that hasn't been reported yet,
/// so they have no amount that can be known up front. Setup fees are left out
/// because they are charged once at the start rather than every period.
fn period_amount(
    price: &LemonSqueezyPriceAttributes,
    item: &LemonSqueezySubscriptionItem,
) -> Option<i64> {
    if item.is_usage_based || price.usage_aggregation.is_some() {
        return None;
    }

    let quantity = item.units();
    let tiers = price.tiers.as_deref().unwrap_or_default();
    let amount = match price.scheme.as_deref() {
        Some("volume") => volume_amount(tiers, quantity)?,
        Some("graduated") => graduated_amount(tiers, quantity)?,
        Some("package") => price
            .unit_price?
            .saturating_mul(package_count(quantity, price.package_size.unwrap_or(1))),
        // `standard`, and any scheme Lemon Squeezy adds later.
        _ => price.unit_price?.saturating_mul(quantity),
    };
    // An unpriced plan is left unknown rather than recorded as free.
    (amount > 0).then_some(amount)
}

/// Volume pricing charges every unit at the rate of the one tier the total
/// falls into, plus that tier's fixed fee.
fn volume_amount(tiers: &[LemonSqueezyPriceTier], quantity: i64) -> Option<i64> {
    let tier = tiers
        .iter()
        .find(|tier| {
            tier.last_unit
                .units()
                .is_none_or(|last_unit| quantity <= last_unit)
        })
        .or_else(|| tiers.last())?;
    Some(
        tier.unit_price?
            .saturating_mul(quantity)
            .saturating_add(tier.fixed_fee.unwrap_or(0)),
    )
}

/// Graduated pricing charges each tier's rate only for the units falling within
/// that tier, so the cheaper units a subscription grew through keep their price.
/// A tier's fixed fee is charged when any of its units are billed.
fn graduated_amount(tiers: &[LemonSqueezyPriceTier], quantity: i64) -> Option<i64> {
    let mut amount: i64 = 0;
    let mut billed = 0;
    for tier in tiers {
        if billed >= quantity {
            break;
        }
        // The highest tier's `last_unit` is "inf", covering whatever is left.
        let tier_top = tier.last_unit.units().unwrap_or(quantity).min(quantity);
        let units = tier_top - billed;
        if units <= 0 {
            continue;
        }
        amount = amount
            .saturating_add(tier.unit_price?.saturating_mul(units))
            .saturating_add(tier.fixed_fee.unwrap_or(0));
        billed = tier_top;
    }
    Some(amount)
}

/// The packages needed to cover `quantity` units, since usage past a package's
/// limit is charged as another whole package.
fn package_count(quantity: i64, package_size: i64) -> i64 {
    let package_size = package_size.max(1);
    quantity.saturating_add(package_size - 1) / package_size
}

fn subscription_invoice_payment_id(invoice_id: &str) -> String {
    format!("subscription_invoice_{invoice_id}")
}

fn subscription_invoice_refund_id(invoice_id: &str) -> String {
    format!("subscription_invoice_refund_{invoice_id}")
}

fn lemon_payment_status(status: &str, amount: i64) -> PaymentStatus {
    match status {
        "paid" | "refunded" | "partial_refund" => PaymentStatus::Ok,
        "void" => PaymentStatus::Canceled,
        "failed" | "fraudulent" => PaymentStatus::Failed,
        "pending" => PaymentStatus::Pending,
        _ if amount > 0 => PaymentStatus::Ok,
        _ => PaymentStatus::Pending,
    }
}

fn lemon_amount_to_thousandths(amount_cents: i64) -> i64 {
    amount_cents.saturating_mul(10)
}

fn currency_code(currency: &str) -> String {
    super::store::currency_code(currency)
}

fn usd_settlement(amount_cents: i64, exchange_rate: Option<&str>) -> Settlement {
    Settlement {
        currency: Some("USD".to_string()),
        amount: Some(lemon_amount_to_thousandths(amount_cents)),
        exchange_rate: exchange_rate.and_then(|rate| rate.parse::<f64>().ok()),
    }
}

fn object_external_account_id(store_id: i64, fallback: &str) -> String {
    if store_id > 0 {
        store_id.to_string()
    } else {
        fallback.to_string()
    }
}

fn parse_optional_lemon_squeezy_time(value: Option<&str>) -> SyncResult<Option<i64>> {
    value.map(parse_lemon_squeezy_time).transpose()
}

/// Lemon Squeezy documents RFC 3339 timestamps ending in `Z`. One without any
/// offset is still read as UTC, as this integration always has.
fn parse_lemon_squeezy_time(value: &str) -> SyncResult<i64> {
    let value = value.trim();
    // A date has `-` too, so only what follows it can say whether there's an offset.
    let has_offset = value
        .split_once(['T', 't', ' '])
        .is_some_and(|(_date, time)| time.contains(['Z', 'z', '+', '-']));
    let parsed = if has_offset {
        rfc3339_to_unix(value)
    } else {
        rfc3339_to_unix(&format!("{value}Z"))
    };
    parsed.map_err(|error| SyncError::LemonSqueezy(format!("invalid timestamp {value:?}: {error}")))
}

struct LemonSqueezyClient {
    http: reqwest::Client,
    api_key: String,
    api_base_url: String,
}

impl LemonSqueezyClient {
    fn new(http: &reqwest::Client, api_key: String) -> Self {
        Self {
            http: http.clone(),
            api_key,
            api_base_url: LEMON_SQUEEZY_API_BASE_URL.to_string(),
        }
    }

    async fn list_page(
        &self,
        path: &str,
        page_number: i64,
    ) -> SyncResult<LemonSqueezyListResponse> {
        let request = self
            .http
            .get(self.list_url(path, page_number)?)
            .bearer_auth(&self.api_key)
            .header("accept", "application/vnd.api+json")
            .header("content-type", "application/vnd.api+json");
        self.send_json(request).await
    }

    async fn retrieve_resource(&self, path: &str) -> SyncResult<LemonSqueezyResource> {
        let request = self
            .http
            .get(self.url(path))
            .bearer_auth(&self.api_key)
            .header("accept", "application/vnd.api+json")
            .header("content-type", "application/vnd.api+json");
        let response: LemonSqueezyResourceResponse = self.send_json(request).await?;
        Ok(response.data)
    }

    async fn send_json<T: DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> SyncResult<T> {
        let response = request.send().await?;
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            return Err(SyncError::LemonSqueezy(format!(
                "Lemon Squeezy API request failed with {status}: {text}"
            )));
        }
        Ok(serde_json::from_str(&text)?)
    }

    fn url(&self, path: &str) -> String {
        format!(
            "{}/{}",
            self.api_base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }

    fn list_url(&self, path: &str, page_number: i64) -> SyncResult<String> {
        let query = [
            ("page[number]", page_number.to_string()),
            ("page[size]", LEMON_SQUEEZY_PAGE_SIZE.to_string()),
        ];
        let query = serde_urlencoded::to_string(query)
            .map_err(|error| SyncError::LemonSqueezy(error.to_string()))?;
        Ok(format!("{}?{query}", self.url(path)))
    }
}

struct LemonSqueezySubscriptionRow {
    resource: LemonSqueezyResource,
    attrs: LemonSqueezySubscriptionAttributes,
    created_at: i64,
    updated_at: Option<i64>,
    price: LemonSqueezyPrice,
}

/// The plan a subscription is on, resolved from its price object and the units
/// it is subscribed for.
struct LemonSqueezyPrice {
    id: Option<String>,
    /// The billing period as a `(unit, count)` pair, absent for a subscription
    /// with no priced item to read one from.
    interval: Option<(String, i64)>,
    quantity: Option<i64>,
    /// The full recurring amount in cents per billing interval, covering every
    /// unit subscribed for.
    amount: Option<i64>,
}

impl LemonSqueezyPrice {
    /// The plan of a subscription that carries no priced item: nothing known,
    /// which leaves whatever an earlier sync recorded in place.
    fn unknown() -> Self {
        Self {
            id: None,
            interval: None,
            quantity: None,
            amount: None,
        }
    }
}

struct Settlement {
    currency: Option<String>,
    amount: Option<i64>,
    exchange_rate: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, serde::Serialize)]
struct LemonSqueezyResource {
    #[serde(rename = "type")]
    resource_type: String,
    id: String,
    attributes: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    relationships: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    links: Option<Value>,
}

impl LemonSqueezyResource {
    fn attributes<T: DeserializeOwned>(&self) -> SyncResult<T> {
        Ok(serde_json::from_value(self.attributes.clone())?)
    }
}

fn lemon_metadata(resource: &LemonSqueezyResource) -> SyncResult<Option<String>> {
    let Some(attributes) = resource.attributes.as_object() else {
        return Ok(None);
    };
    for key in ["metadata", "custom_data"] {
        if let Some(value) = attributes.get(key)
            && let Some(metadata) = serialize_metadata(value)?
        {
            return Ok(Some(metadata));
        }
    }
    Ok(None)
}

#[derive(Debug, Deserialize)]
struct LemonSqueezyListResponse {
    data: Vec<LemonSqueezyResource>,
    meta: Option<LemonSqueezyMeta>,
}

#[derive(Debug, Deserialize)]
struct LemonSqueezyResourceResponse {
    data: LemonSqueezyResource,
}

#[derive(Debug, Deserialize)]
struct LemonSqueezyMeta {
    page: LemonSqueezyPageMeta,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LemonSqueezyPageMeta {
    last_page: i64,
}

#[derive(Debug, Deserialize)]
struct LemonSqueezySubscriptionAttributes {
    store_id: i64,
    customer_id: Option<i64>,
    order_id: Option<i64>,
    user_email: Option<String>,
    status: String,
    cancelled: bool,
    first_subscription_item: Option<LemonSqueezySubscriptionItem>,
    renews_at: Option<String>,
    ends_at: Option<String>,
    created_at: String,
    updated_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LemonSqueezySubscriptionItem {
    price_id: i64,
    #[serde(default)]
    quantity: Option<i64>,
    #[serde(default)]
    is_usage_based: bool,
}

impl LemonSqueezySubscriptionItem {
    /// The units subscribed for, which Lemon Squeezy defaults to 1 when a
    /// checkout doesn't set one.
    fn units(&self) -> i64 {
        self.quantity.unwrap_or(1).max(0)
    }
}

#[derive(Debug, Deserialize)]
struct LemonSqueezySubscriptionInvoiceAttributes {
    store_id: i64,
    subscription_id: i64,
    customer_id: i64,
    user_email: Option<String>,
    currency: String,
    currency_rate: Option<String>,
    status: String,
    total: i64,
    #[serde(default)]
    total_usd: i64,
    #[serde(default)]
    refunded_amount: i64,
    #[serde(default)]
    refunded_amount_usd: i64,
    refunded_at: Option<String>,
    created_at: String,
    updated_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LemonSqueezyPriceAttributes {
    renewal_interval_unit: Option<String>,
    renewal_interval_quantity: Option<i64>,
    /// One of `standard`, `package`, `graduated` or `volume`.
    #[serde(default)]
    scheme: Option<String>,
    /// Set only when the variant bills for reported usage instead of a quantity.
    #[serde(default)]
    usage_aggregation: Option<String>,
    /// The price of one unit, or of one package under `package` pricing. Not
    /// used by the tiered schemes, which price units from `tiers` instead.
    #[serde(default)]
    unit_price: Option<i64>,
    /// The units one package covers, and 1 for every other scheme.
    #[serde(default)]
    package_size: Option<i64>,
    /// The tiers of a `graduated` or `volume` price, in ascending order.
    #[serde(default)]
    tiers: Option<Vec<LemonSqueezyPriceTier>>,
}

#[derive(Debug, Deserialize)]
struct LemonSqueezyPriceTier {
    last_unit: LemonSqueezyTierLimit,
    #[serde(default)]
    unit_price: Option<i64>,
    #[serde(default)]
    fixed_fee: Option<i64>,
}

/// The top of a pricing tier, which Lemon Squeezy reports as `"inf"` for the
/// highest tier of a price.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum LemonSqueezyTierLimit {
    Units(i64),
    Unbounded(String),
}

impl LemonSqueezyTierLimit {
    /// The tier's top limit, or `None` when it is unbounded.
    fn units(&self) -> Option<i64> {
        match self {
            Self::Units(units) => Some(*units),
            Self::Unbounded(value) => value.parse().ok(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// One dollar, in the cents Lemon Squeezy prices are quoted in.
    const DOLLAR: i64 = 100;

    /// A subscription price built from the payload Lemon Squeezy would return,
    /// so that tier limits and null unit prices go through deserialization.
    fn price(scheme: &str, attributes: Value) -> LemonSqueezyPriceAttributes {
        let mut price = json!({
            "category": "subscription",
            "scheme": scheme,
            "usage_aggregation": null,
            "unit_price": null,
            "unit_price_decimal": null,
            "setup_fee_enabled": false,
            "setup_fee": null,
            "package_size": 1,
            "tiers": null,
            "renewal_interval_unit": "month",
            "renewal_interval_quantity": 1,
        });
        let overrides = attributes.as_object().expect("price attributes").clone();
        price
            .as_object_mut()
            .expect("price attributes")
            .extend(overrides);
        serde_json::from_value(price).expect("price attributes")
    }

    fn item(quantity: i64) -> LemonSqueezySubscriptionItem {
        serde_json::from_value(json!({
            "subscription_id": 1,
            "price_id": 1,
            "quantity": quantity,
            "is_usage_based": false,
        }))
        .expect("subscription item")
    }

    #[test]
    fn standard_pricing_charges_the_unit_price_for_every_unit() {
        let price = price("standard", json!({ "unit_price": 9 * DOLLAR }));

        assert_eq!(period_amount(&price, &item(3)), Some(27 * DOLLAR));
    }

    /// The quantity is optional at checkout, and one unit is what Lemon Squeezy
    /// bills when it is left out.
    #[test]
    fn a_missing_quantity_is_billed_as_a_single_unit() {
        let price = price("standard", json!({ "unit_price": 9 * DOLLAR }));
        let item = serde_json::from_value(json!({ "price_id": 1 })).expect("subscription item");

        assert_eq!(period_amount(&price, &item), Some(9 * DOLLAR));
    }

    /// A setup fee is charged once at the start of the subscription, so it is no
    /// part of the recurring amount.
    #[test]
    fn a_setup_fee_is_left_out_of_the_recurring_amount() {
        let price = price(
            "standard",
            json!({
                "unit_price": 10 * DOLLAR,
                "setup_fee_enabled": true,
                "setup_fee": 15 * DOLLAR,
            }),
        );

        assert_eq!(period_amount(&price, &item(1)), Some(10 * DOLLAR));
    }

    #[test]
    fn package_pricing_charges_for_every_package_started() {
        let price = price(
            "package",
            json!({ "unit_price": 10 * DOLLAR, "package_size": 100 }),
        );

        assert_eq!(period_amount(&price, &item(100)), Some(10 * DOLLAR));
        assert_eq!(period_amount(&price, &item(120)), Some(20 * DOLLAR));
        assert_eq!(period_amount(&price, &item(201)), Some(30 * DOLLAR));
    }

    /// $5 per seat up to 5 seats and $4 per seat beyond that, which prices 8
    /// seats at 8 × $4 rather than at a blend of the two rates.
    #[test]
    fn volume_pricing_charges_every_unit_at_the_rate_of_the_tier_reached() {
        let price = price(
            "volume",
            json!({
                "tiers": [
                    { "last_unit": 5, "unit_price": 5 * DOLLAR, "fixed_fee": 0 },
                    { "last_unit": "inf", "unit_price": 4 * DOLLAR, "fixed_fee": 0 },
                ],
            }),
        );

        assert_eq!(period_amount(&price, &item(5)), Some(25 * DOLLAR));
        assert_eq!(period_amount(&price, &item(8)), Some(32 * DOLLAR));
    }

    #[test]
    fn volume_pricing_adds_the_fixed_fee_of_the_tier_reached() {
        let price = price(
            "volume",
            json!({
                "tiers": [
                    { "last_unit": 5, "unit_price": 5 * DOLLAR, "fixed_fee": 10 * DOLLAR },
                    { "last_unit": "inf", "unit_price": 4 * DOLLAR, "fixed_fee": 20 * DOLLAR },
                ],
            }),
        );

        assert_eq!(period_amount(&price, &item(8)), Some(52 * DOLLAR));
    }

    /// A $5 base fee plus $3 for each of the first 50 orders, $2 for the next 50
    /// and $1 for the rest, which prices 180 orders at $335.
    #[test]
    fn graduated_pricing_charges_each_tier_for_its_own_units() {
        let price = price(
            "graduated",
            json!({
                "tiers": [
                    { "last_unit": 50, "unit_price": 3 * DOLLAR, "fixed_fee": 5 * DOLLAR },
                    { "last_unit": 100, "unit_price": 2 * DOLLAR, "fixed_fee": 0 },
                    { "last_unit": "inf", "unit_price": DOLLAR, "fixed_fee": 0 },
                ],
            }),
        );

        assert_eq!(period_amount(&price, &item(180)), Some(335 * DOLLAR));
    }

    /// Tiers a subscription hasn't grown into cost nothing, not even their fee.
    #[test]
    fn graduated_pricing_only_charges_the_tiers_it_reaches() {
        let price = price(
            "graduated",
            json!({
                "tiers": [
                    { "last_unit": 50, "unit_price": 3 * DOLLAR, "fixed_fee": 5 * DOLLAR },
                    { "last_unit": "inf", "unit_price": DOLLAR, "fixed_fee": 40 * DOLLAR },
                ],
            }),
        );

        assert_eq!(period_amount(&price, &item(10)), Some(35 * DOLLAR));
        assert_eq!(period_amount(&price, &item(50)), Some(155 * DOLLAR));
        assert_eq!(period_amount(&price, &item(51)), Some(196 * DOLLAR));
    }

    /// Usage-based prices are billed in arrears for usage that hasn't been
    /// reported yet, so no recurring amount can be committed to. Their tiers
    /// carry `unit_price_decimal` instead of a `unit_price`.
    #[test]
    fn usage_based_prices_have_no_recurring_amount() {
        let price = price(
            "graduated",
            json!({
                "usage_aggregation": "sum",
                "tiers": [
                    { "last_unit": "inf", "unit_price": null, "unit_price_decimal": "3.5", "fixed_fee": 0 },
                ],
            }),
        );
        let item = serde_json::from_value(json!({
            "price_id": 1,
            "quantity": 0,
            "is_usage_based": true,
        }))
        .expect("subscription item");

        assert_eq!(period_amount(&price, &item), None);
    }

    /// A usage-based item on a price whose scheme still quotes a unit price is
    /// billed on usage all the same.
    #[test]
    fn a_usage_based_item_has_no_recurring_amount() {
        let price = price("standard", json!({ "unit_price": 9 * DOLLAR }));
        let item = serde_json::from_value(json!({
            "price_id": 1,
            "quantity": 4,
            "is_usage_based": true,
        }))
        .expect("subscription item");

        assert_eq!(period_amount(&price, &item), None);
    }

    /// An unpriced plan is left unknown rather than counted as $0 of revenue.
    #[test]
    fn a_price_without_an_amount_is_unknown() {
        assert_eq!(period_amount(&price("standard", json!({})), &item(3)), None);
        assert_eq!(
            period_amount(&price("standard", json!({ "unit_price": 0 })), &item(3)),
            None
        );
        assert_eq!(
            period_amount(&price("volume", json!({ "tiers": [] })), &item(3)),
            None
        );
    }

    #[test]
    fn timestamps_are_read_at_their_own_offset() {
        let noon_utc = 1_628_683_200;
        for value in [
            "2021-08-11T12:00:00.000000Z",
            " 2021-08-11T12:00:00Z ",
            "2021-08-11T14:00:00+02:00",
            "2021-08-11T07:00:00.5-05:00",
            "2021-08-11T12:00:00.000000",
        ] {
            assert_eq!(
                parse_lemon_squeezy_time(value).ok(),
                Some(noon_utc),
                "{value}"
            );
        }
        for value in [
            "",
            "2021-08-11",
            "2021-08-11T12:00:00.12345\u{e9}Z",
            "2021-13-11T12:00:00Z",
        ] {
            assert!(parse_lemon_squeezy_time(value).is_err(), "{value}");
        }
        // The error names what's wrong with the value, with or without an offset.
        for value in [
            "2021-13-11T12:00:00Z",
            "2021-13-11T12:00:00",
            "2021-13-11T12:00:00+02:00",
        ] {
            let SyncError::LemonSqueezy(message) =
                parse_lemon_squeezy_time(value).expect_err("month 13")
            else {
                panic!("a Lemon Squeezy error");
            };
            assert!(message.contains("month"), "{message}");
        }
    }
}
