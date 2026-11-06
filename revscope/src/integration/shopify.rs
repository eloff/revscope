use std::sync::Arc;

use log::info;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;

use super::attribution;
use super::schedule::{
    self, PAYMENT_FULL_SYNC_INTERVAL_SECS, SyncOutcome, decrypt_secret,
    due_payment_integrations_sql, payment_sync_since, sync_kind,
};
use super::store::{
    PaymentStatus, PaymentType, PaymentUpsert, PlanInterval, ProviderObjectRef, Reported,
    SubscriptionPlan, SubscriptionUpsert, store_provider_object, subscription_interval,
    subscription_row_id, upsert_payment, upsert_subscription,
};
use super::sync::{SyncCounts, SyncError, SyncKind, SyncResult, serialize_metadata};
use crate::db::{ProjectCache, ProjectConnection, open_main_database_read};
use crate::server_context::ServerContext;
use crate::util::{rfc3339_to_unix, unix_to_rfc3339};

const PROVIDER: &str = "shopify";
const SHOPIFY_API_VERSION: &str = "2026-07";
const SHOPIFY_PAGE_LIMIT: i32 = 100;

const ORDERS_QUERY: &str = r#"
query RevScopeOrders($first: Int!, $after: String, $query: String) {
  orders(first: $first, after: $after, query: $query, sortKey: UPDATED_AT) {
    pageInfo {
      hasNextPage
      endCursor
    }
    nodes {
      id
      name
      createdAt
      updatedAt
      processedAt
      email
      displayFinancialStatus
      currencyCode
      totalReceivedSet {
        shopMoney {
          amount
          currencyCode
        }
      }
      totalRefundedSet {
        shopMoney {
          amount
          currencyCode
        }
      }
      customer {
        id
        email
      }
      customAttributes {
        key
        value
      }
      lineItems(first: 10) {
        nodes {
          contract {
            id
          }
        }
      }
      transactions(first: 50) {
        id
        kind
        status
        gateway
        createdAt
        processedAt
        amountSet {
          shopMoney {
            amount
            currencyCode
          }
        }
        parentTransaction {
          id
        }
      }
      refunds(first: 50) {
        id
        createdAt
        updatedAt
        totalRefundedSet {
          shopMoney {
            amount
            currencyCode
          }
        }
        transactions(first: 50) {
          nodes {
            id
            kind
            status
            gateway
            createdAt
            processedAt
            amountSet {
              shopMoney {
                amount
                currencyCode
              }
            }
            parentTransaction {
              id
            }
          }
        }
      }
    }
  }
}
"#;

const SUBSCRIPTION_CONTRACTS_QUERY: &str = r#"
query RevScopeSubscriptionContracts($first: Int!, $after: String, $query: String) {
  subscriptionContracts(first: $first, after: $after, query: $query, sortKey: UPDATED_AT) {
    pageInfo {
      hasNextPage
      endCursor
    }
    nodes {
      id
      status
      createdAt
      updatedAt
      nextBillingDate
      currencyCode
      customer {
        id
        email
      }
      originOrder {
        id
      }
      billingPolicy {
        interval
        intervalCount
      }
      lines(first: 25) {
        nodes {
          quantity
          currentPrice {
            amount
            currencyCode
          }
        }
      }
      orders(first: 20, reverse: true) {
        nodes {
          id
        }
      }
    }
  }
}
"#;

/// Loads Shopify integrations that are due for sync. `external_account_id`
/// stores the shop domain and `encrypted_api_key` stores the Admin API token.
pub(crate) async fn load_due_integrations(
    server_context: Arc<ServerContext>,
) -> SyncResult<Vec<ShopifyIntegration>> {
    let conn = open_main_database_read(server_context).await?;
    let rows = conn
        .query(
            &due_payment_integrations_sql(
                "id,
                 project_id::text AS project_id,
                 external_account_id,
                 encrypted_api_key,
                 EXTRACT(EPOCH FROM last_sync_success_at)::BIGINT,
                 EXTRACT(EPOCH FROM last_full_sync_success_at)::BIGINT",
                PROVIDER,
                "\n            AND external_account_id IS NOT NULL",
            ),
            &[],
        )
        .await?;

    Ok(rows
        .iter()
        .map(|row| {
            let last_success_started_at: Option<i64> = row.get(4);
            let last_full_success_started_at: Option<i64> = row.get(5);
            ShopifyIntegration {
                id: row.get(0),
                project_id: row.get(1),
                shop_domain: row.get(2),
                encrypted_api_key: row.get(3),
                last_success_started_at,
                sync_kind: sync_kind(
                    last_full_success_started_at,
                    PAYMENT_FULL_SYNC_INTERVAL_SECS,
                ),
            }
        })
        .collect())
}

pub(crate) async fn run_sync_with_retries(
    server_context: Arc<ServerContext>,
    integration: ShopifyIntegration,
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

pub(crate) struct ShopifyIntegration {
    id: i32,
    project_id: String,
    shop_domain: String,
    encrypted_api_key: Option<Vec<u8>>,
    last_success_started_at: Option<i64>,
    sync_kind: SyncKind,
}

async fn sync_integration(
    server_context: Arc<ServerContext>,
    integration: &ShopifyIntegration,
) -> SyncResult<SyncCounts> {
    let project =
        super::sync::ensure_project_cached(server_context.clone(), &integration.project_id).await?;
    let access_token = decrypt_secret(
        integration.encrypted_api_key.as_deref(),
        SyncError::MissingShopifyCredential("access token"),
        || SyncError::InvalidShopifyCredential("access token"),
    )?;
    let client = ShopifyClient::new(
        server_context.http.clone(),
        integration.shop_domain.clone(),
        access_token,
    )?;
    let since = Some(payment_sync_since(
        integration.sync_kind,
        integration.last_success_started_at,
    ));

    let mut counts = SyncCounts::default();
    sync_subscription_contracts(
        &server_context,
        &project,
        &client,
        &integration.shop_domain,
        since,
        &mut counts,
    )
    .await?;
    sync_orders(
        &server_context,
        &project,
        &client,
        &integration.shop_domain,
        since,
        &mut counts,
    )
    .await?;
    if integration.sync_kind == SyncKind::Full {
        attribution::attribute_full_sync(server_context.clone(), &project, "shopify").await?;
    }

    info!(
        "synced Shopify integration {} ({}): inserted={}, updated={}, deleted={}",
        integration.id,
        integration.sync_kind.as_str(),
        counts.records_inserted,
        counts.records_updated,
        counts.records_deleted
    );
    Ok(counts)
}

async fn sync_subscription_contracts(
    server_context: &ServerContext,
    project: &Arc<ProjectCache>,
    client: &ShopifyClient,
    external_account_id: &str,
    since: Option<i64>,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    let mut after = None;
    loop {
        let variables = json!({
            "first": SHOPIFY_PAGE_LIMIT,
            "after": after.clone(),
            "query": search_query("updated_at", since)?,
        });
        let data: SubscriptionContractsData = client
            .graphql(SUBSCRIPTION_CONTRACTS_QUERY, variables)
            .await?;
        let page_info = data.subscription_contracts.page_info;
        {
            let conn = crate::db::open_project_database_write_for(server_context, project)?;
            for contract in data.subscription_contracts.nodes {
                let stored = store_provider_object(
                    &conn,
                    PROVIDER,
                    external_account_id,
                    ProviderObjectRef {
                        object_type: "subscription_contract",
                        object_id: &contract.id,
                        created_at: Some(timestamp_to_unix(&contract.created_at)?),
                        updated_at: Some(timestamp_to_unix(&contract.updated_at)?),
                    },
                    &contract,
                    counts,
                )?;
                if stored.changed {
                    let subscription_id = store_subscription(&conn, stored.id, &contract, counts)?;
                    attribution::attribute_subscription_by_id(&conn, subscription_id)?;
                }
            }
        }
        if !page_info.has_next_page {
            break;
        }
        after = page_info.end_cursor;
    }
    Ok(())
}

async fn sync_orders(
    server_context: &ServerContext,
    project: &Arc<ProjectCache>,
    client: &ShopifyClient,
    external_account_id: &str,
    since: Option<i64>,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    let mut after = None;
    loop {
        let variables = json!({
            "first": SHOPIFY_PAGE_LIMIT,
            "after": after.clone(),
            "query": search_query("updated_at", since)?,
        });
        let data: OrdersData = client.graphql(ORDERS_QUERY, variables).await?;
        let page_info = data.orders.page_info;
        {
            let conn = crate::db::open_project_database_write_for(server_context, project)?;
            for order in data.orders.nodes {
                let stored = store_provider_object(
                    &conn,
                    PROVIDER,
                    external_account_id,
                    ProviderObjectRef {
                        object_type: "order",
                        object_id: &order.id,
                        created_at: Some(timestamp_to_unix(&order.created_at)?),
                        updated_at: Some(timestamp_to_unix(&order.updated_at)?),
                    },
                    &order,
                    counts,
                )?;
                if stored.changed {
                    let provider_payment_id = order_payment_id(&order);
                    store_order_payment(&conn, stored.id, &order, counts)?;
                    attribution::attribute_payment_by_provider_id(
                        &conn,
                        PROVIDER,
                        &provider_payment_id,
                    )?;
                }
                for refund in &order.refunds {
                    let refund_object = ShopifyRefundObject {
                        order: &order,
                        refund,
                    };
                    let stored_refund = store_provider_object(
                        &conn,
                        PROVIDER,
                        external_account_id,
                        ProviderObjectRef {
                            object_type: "refund",
                            object_id: &refund.id,
                            created_at: Some(timestamp_to_unix(&refund.created_at)?),
                            updated_at: timestamp_to_unix_opt(refund.updated_at.as_deref())?,
                        },
                        &refund_object,
                        counts,
                    )?;
                    if stored_refund.changed {
                        let provider_payment_id = refund.id.clone();
                        store_refund_payment(&conn, stored_refund.id, &order, refund, counts)?;
                        attribution::attribute_payment_by_provider_id(
                            &conn,
                            PROVIDER,
                            &provider_payment_id,
                        )?;
                    }
                }
            }
        }
        if !page_info.has_next_page {
            break;
        }
        after = page_info.end_cursor;
    }
    Ok(())
}

fn search_query(field: &str, since: Option<i64>) -> SyncResult<Option<String>> {
    since
        .map(|timestamp| {
            let value = sync_timestamp(timestamp)?;
            Ok(format!("{field}:>{value}"))
        })
        .transpose()
}

struct ShopifyClient {
    client: reqwest::Client,
    endpoint: String,
    access_token: String,
}

impl ShopifyClient {
    fn new(client: reqwest::Client, shop_domain: String, access_token: String) -> SyncResult<Self> {
        let endpoint = shopify_graphql_endpoint(&shop_domain)?;
        Ok(Self {
            client,
            endpoint,
            access_token,
        })
    }

    async fn graphql<T>(&self, query: &str, variables: Value) -> SyncResult<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        let response = self
            .client
            .post(&self.endpoint)
            .header("content-type", "application/json")
            .header("x-shopify-access-token", &self.access_token)
            .json(&GraphqlRequest { query, variables })
            .send()
            .await?;
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            return Err(SyncError::Shopify(format!(
                "Shopify GraphQL request failed with {status}: {text}"
            )));
        }
        let response: GraphqlResponse<T> = serde_json::from_str(&text)?;
        if let Some(errors) = response.errors
            && !errors.is_empty()
        {
            return Err(SyncError::Shopify(format_graphql_errors(&errors)));
        }
        response
            .data
            .ok_or_else(|| SyncError::Shopify("Shopify GraphQL response had no data".to_string()))
    }
}

fn shopify_graphql_endpoint(shop_domain: &str) -> SyncResult<String> {
    let shop_domain = normalize_shop_domain(shop_domain)?;
    Ok(format!(
        "https://{shop_domain}/admin/api/{SHOPIFY_API_VERSION}/graphql.json"
    ))
}

fn normalize_shop_domain(shop_domain: &str) -> SyncResult<String> {
    let value = shop_domain.trim().trim_end_matches('/');
    if value.is_empty() {
        return Err(SyncError::Shopify(
            "Shopify integration has no shop domain".to_string(),
        ));
    }
    if let Ok(url) = Url::parse(value) {
        return url
            .host_str()
            .map(str::to_string)
            .ok_or_else(|| SyncError::Shopify("Shopify shop URL must include a host".to_string()));
    }
    Ok(value.to_string())
}

#[derive(Serialize)]
struct GraphqlRequest<'a> {
    query: &'a str,
    variables: Value,
}

#[derive(Deserialize)]
struct GraphqlResponse<T> {
    data: Option<T>,
    errors: Option<Vec<GraphqlError>>,
}

#[derive(Deserialize)]
struct GraphqlError {
    message: String,
}

fn format_graphql_errors(errors: &[GraphqlError]) -> String {
    errors
        .iter()
        .map(|error| error.message.as_str())
        .collect::<Vec<_>>()
        .join("; ")
}

#[derive(Debug, Deserialize, Serialize)]
struct OrdersData {
    orders: ShopifyConnection<ShopifyOrder>,
}

#[derive(Debug, Deserialize, Serialize)]
struct SubscriptionContractsData {
    #[serde(rename = "subscriptionContracts")]
    subscription_contracts: ShopifyConnection<ShopifySubscriptionContract>,
}

#[derive(Debug, Deserialize, Serialize)]
struct ShopifyConnection<T> {
    #[serde(rename = "pageInfo")]
    page_info: ShopifyPageInfo,
    nodes: Vec<T>,
}

#[derive(Debug, Deserialize, Serialize)]
struct ShopifyNodes<T> {
    nodes: Vec<T>,
}

#[derive(Debug, Deserialize, Serialize)]
struct ShopifyPageInfo {
    #[serde(rename = "hasNextPage")]
    has_next_page: bool,
    #[serde(rename = "endCursor")]
    end_cursor: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct ShopifyOrder {
    id: String,
    name: String,
    #[serde(rename = "createdAt")]
    created_at: String,
    #[serde(rename = "updatedAt")]
    updated_at: String,
    #[serde(rename = "processedAt")]
    processed_at: Option<String>,
    email: Option<String>,
    #[serde(rename = "customAttributes", default)]
    custom_attributes: Vec<ShopifyAttribute>,
    #[serde(rename = "displayFinancialStatus")]
    display_financial_status: Option<String>,
    #[serde(rename = "currencyCode")]
    currency_code: String,
    #[serde(rename = "totalReceivedSet")]
    total_received_set: ShopifyMoneyBag,
    #[serde(rename = "totalRefundedSet")]
    total_refunded_set: ShopifyMoneyBag,
    customer: Option<ShopifyCustomer>,
    #[serde(rename = "lineItems")]
    line_items: ShopifyNodes<ShopifyLineItem>,
    transactions: Vec<ShopifyTransaction>,
    refunds: Vec<ShopifyRefund>,
}

#[derive(Debug, Deserialize, Serialize)]
struct ShopifySubscriptionContract {
    id: String,
    status: String,
    #[serde(rename = "createdAt")]
    created_at: String,
    #[serde(rename = "updatedAt")]
    updated_at: String,
    #[serde(rename = "nextBillingDate")]
    next_billing_date: Option<String>,
    #[serde(rename = "currencyCode")]
    currency_code: String,
    customer: Option<ShopifyCustomer>,
    #[serde(rename = "originOrder")]
    origin_order: Option<ShopifyOrderRef>,
    #[serde(rename = "billingPolicy")]
    billing_policy: ShopifyBillingPolicy,
    #[serde(default)]
    lines: Option<ShopifyNodes<ShopifySubscriptionLine>>,
    orders: ShopifyNodes<ShopifyOrderRef>,
}

#[derive(Debug, Deserialize, Serialize)]
struct ShopifyBillingPolicy {
    interval: String,
    #[serde(rename = "intervalCount")]
    interval_count: i64,
}

#[derive(Debug, Deserialize, Serialize)]
struct ShopifySubscriptionLine {
    quantity: Option<i64>,
    #[serde(rename = "currentPrice")]
    current_price: Option<ShopifyMoney>,
}

#[derive(Debug, Deserialize, Serialize)]
struct ShopifyLineItem {
    contract: Option<ShopifyOrderRef>,
}

#[derive(Debug, Deserialize, Serialize)]
struct ShopifyOrderRef {
    id: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct ShopifyCustomer {
    id: String,
    email: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct ShopifyAttribute {
    key: String,
    value: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct ShopifyRefund {
    id: String,
    #[serde(rename = "createdAt")]
    created_at: String,
    #[serde(rename = "updatedAt")]
    updated_at: Option<String>,
    #[serde(rename = "totalRefundedSet")]
    total_refunded_set: ShopifyMoneyBag,
    transactions: ShopifyNodes<ShopifyTransaction>,
}

#[derive(Serialize)]
struct ShopifyRefundObject<'a> {
    order: &'a ShopifyOrder,
    refund: &'a ShopifyRefund,
}

#[derive(Debug, Deserialize, Serialize)]
struct ShopifyTransaction {
    id: String,
    kind: String,
    status: String,
    gateway: Option<String>,
    #[serde(rename = "createdAt")]
    created_at: Option<String>,
    #[serde(rename = "processedAt")]
    processed_at: Option<String>,
    #[serde(rename = "amountSet")]
    amount_set: ShopifyMoneyBag,
    #[serde(rename = "parentTransaction")]
    parent_transaction: Option<ShopifyOrderRef>,
}

#[derive(Debug, Deserialize, Serialize)]
struct ShopifyMoneyBag {
    #[serde(rename = "shopMoney")]
    shop_money: ShopifyMoney,
}

#[derive(Debug, Deserialize, Serialize)]
struct ShopifyMoney {
    amount: String,
    #[serde(rename = "currencyCode")]
    currency_code: String,
}

fn store_subscription(
    conn: &ProjectConnection,
    provider_object_id: i64,
    contract: &ShopifySubscriptionContract,
    counts: &mut SyncCounts,
) -> SyncResult<i64> {
    let customer_id = contract
        .customer
        .as_ref()
        .map(|customer| customer.id.as_str());
    let customer_email = contract
        .customer
        .as_ref()
        .and_then(|customer| customer.email.as_deref());
    let next_billing_at = timestamp_to_unix_opt(contract.next_billing_date.as_deref())?;
    let created_at = timestamp_to_unix(&contract.created_at)?;
    let origin_order_id = contract
        .origin_order
        .as_ref()
        .map(|order| order.id.as_str())
        .or_else(|| contract.orders.nodes.last().map(|order| order.id.as_str()));
    // Shopify has no cancellation or expiry timestamp on the contract, only a
    // status, so the last update is the best available date for either.
    let status = contract.status.to_ascii_lowercase();
    let canceled_at = if matches!(status.as_str(), "cancelled" | "canceled") {
        Some(timestamp_to_unix(&contract.updated_at)?)
    } else {
        None
    };
    let ended_at = if matches!(status.as_str(), "expired" | "failed") {
        Some(timestamp_to_unix(&contract.updated_at)?)
    } else {
        None
    };
    let interval_unit = shopify_interval_unit(&contract.billing_policy.interval);
    let currency = currency_code(&contract.currency_code);
    let (quantity, amount) = subscription_line_totals(contract)?;

    let mut upsert = SubscriptionUpsert::new(PROVIDER, &contract.id);
    upsert.provider_object_id = Some(provider_object_id);
    upsert.provider_order_id = origin_order_id;
    upsert.provider_customer_id = customer_id;
    upsert.customer_email = customer_email;
    upsert.status = Some(&contract.status);
    upsert.plan = SubscriptionPlan {
        interval: Some(PlanInterval::new(
            &interval_unit,
            contract.billing_policy.interval_count,
        )),
        provider_price_id: None,
        plan_name: None,
        quantity,
        currency: Some(&currency),
        amount,
    };
    upsert.current_period_start = Some(created_at);
    upsert.next_billing_at = next_billing_at;
    upsert.started_at = Some(created_at);
    upsert.ended_at = Reported::or_cleared(ended_at);
    upsert.canceled_at = Reported::or_cleared(canceled_at);
    upsert_subscription(conn, &upsert, counts)
}

/// Total quantity and recurring amount across a contract's lines.
fn subscription_line_totals(
    contract: &ShopifySubscriptionContract,
) -> SyncResult<(Option<i64>, Option<i64>)> {
    let Some(lines) = contract.lines.as_ref() else {
        return Ok((None, None));
    };
    let mut quantity = None;
    let mut amount = None;
    for line in &lines.nodes {
        if let Some(line_quantity) = line.quantity {
            quantity = Some(quantity.unwrap_or(0_i64).saturating_add(line_quantity));
        }
        if let Some(price) = line.current_price.as_ref() {
            // `currentPrice` is the per-unit price; multiply by quantity.
            let line_quantity = line.quantity.unwrap_or(1);
            let line_amount = amount_to_thousandths(&price.amount)?.saturating_mul(line_quantity);
            amount = Some(amount.unwrap_or(0_i64).saturating_add(line_amount));
        }
    }
    Ok((quantity, amount))
}

fn store_order_payment(
    conn: &ProjectConnection,
    provider_object_id: i64,
    order: &ShopifyOrder,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    if !order_has_positive_payment(order)? {
        return Ok(());
    }

    let provider_payment_id = order_payment_id(order);
    let subscription_id = order_subscription_row_id(conn, order)?;
    let payment_type = if subscription_id.is_some() {
        PaymentType::Subscription
    } else {
        PaymentType::Single
    };
    let metadata = serialize_metadata(&order.custom_attributes)?;
    let money = &order.total_received_set.shop_money;
    let currency = currency_code(&money.currency_code);
    let customer_id = order.customer.as_ref().map(|customer| customer.id.as_str());
    let customer_email = order
        .customer
        .as_ref()
        .and_then(|customer| customer.email.as_deref());

    let mut upsert = PaymentUpsert::new(
        PROVIDER,
        &provider_payment_id,
        payment_type,
        order_payment_status(order),
        amount_to_thousandths(&money.amount)?,
        &currency,
        order_paid_at(order)?,
    );
    upsert.provider_object_id = Some(provider_object_id);
    upsert.provider_object_type = Some("order");
    upsert.subscription_id = subscription_id;
    upsert.interval = subscription_id
        .map(|id| subscription_interval(conn, id))
        .transpose()?
        .flatten();
    upsert.provider_status = order.display_financial_status.as_deref();
    upsert.provider_order_id = Some(&order.id);
    upsert.provider_customer_id = customer_id;
    upsert.customer_email = customer_email;
    upsert.order_email = order.email.as_deref();
    upsert.created_at = Some(timestamp_to_unix(&order.created_at)?);
    upsert.metadata = metadata.as_deref();
    upsert_payment(conn, &upsert, counts)
}

fn store_refund_payment(
    conn: &ProjectConnection,
    provider_object_id: i64,
    order: &ShopifyOrder,
    refund: &ShopifyRefund,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    let money = &refund.total_refunded_set.shop_money;
    let currency = currency_code(&money.currency_code);
    let created_at = timestamp_to_unix(&refund.created_at)?;
    let parent_payment_id = refund
        .transactions
        .nodes
        .iter()
        .filter_map(|transaction| transaction.parent_transaction.as_ref())
        .map(|transaction| transaction.id.clone())
        .next()
        .unwrap_or_else(|| order_payment_id(order));
    let provider_status = refund
        .transactions
        .nodes
        .first()
        .map(|transaction| transaction.status.as_str());

    let mut upsert = PaymentUpsert::new(
        PROVIDER,
        &refund.id,
        PaymentType::Refund,
        refund_payment_status(refund),
        amount_to_thousandths(&money.amount)?,
        &currency,
        created_at,
    );
    upsert.provider_object_id = Some(provider_object_id);
    upsert.provider_object_type = Some("refund");
    upsert.provider_status = provider_status;
    upsert.provider_order_id = Some(&order.id);
    upsert.provider_refund_id = Some(&refund.id);
    upsert.parent_provider_payment_id = Some(&parent_payment_id);
    upsert.provider_customer_id = order.customer.as_ref().map(|customer| customer.id.as_str());
    upsert.customer_email = order
        .customer
        .as_ref()
        .and_then(|customer| customer.email.as_deref());
    upsert.order_email = order.email.as_deref();
    upsert_payment(conn, &upsert, counts)
}

fn order_subscription_row_id(
    conn: &ProjectConnection,
    order: &ShopifyOrder,
) -> SyncResult<Option<i64>> {
    for line_item in &order.line_items.nodes {
        if let Some(contract) = &line_item.contract
            && let Some(id) = subscription_row_id(conn, PROVIDER, &contract.id)?
        {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

fn order_has_positive_payment(order: &ShopifyOrder) -> SyncResult<bool> {
    Ok(amount_to_thousandths(&order.total_received_set.shop_money.amount)? > 0)
}

fn order_payment_id(order: &ShopifyOrder) -> String {
    order
        .transactions
        .iter()
        .find(|transaction| {
            matches!(
                transaction.kind.as_str(),
                "SALE" | "CAPTURE" | "AUTHORIZATION"
            ) && matches!(transaction.status.as_str(), "SUCCESS" | "PENDING")
        })
        .map(|transaction| transaction.id.clone())
        .unwrap_or_else(|| order.id.clone())
}

fn order_paid_at(order: &ShopifyOrder) -> SyncResult<i64> {
    for transaction in &order.transactions {
        if matches!(transaction.kind.as_str(), "SALE" | "CAPTURE")
            && transaction.status == "SUCCESS"
            && let Some(processed_at) = transaction.processed_at.as_deref()
        {
            return timestamp_to_unix(processed_at);
        }
    }
    timestamp_to_unix(
        order
            .processed_at
            .as_deref()
            .unwrap_or(order.created_at.as_str()),
    )
}

fn order_payment_status(order: &ShopifyOrder) -> PaymentStatus {
    match order
        .display_financial_status
        .as_deref()
        .unwrap_or_default()
    {
        "PAID" | "PARTIALLY_REFUNDED" | "REFUNDED" | "PARTIALLY_PAID" => PaymentStatus::Ok,
        "VOIDED" => PaymentStatus::Canceled,
        "EXPIRED" => PaymentStatus::Failed,
        "PENDING" | "AUTHORIZED" => PaymentStatus::Pending,
        _ if order
            .transactions
            .iter()
            .any(|transaction| transaction.status == "SUCCESS") =>
        {
            PaymentStatus::Ok
        }
        _ => PaymentStatus::Pending,
    }
}

fn refund_payment_status(refund: &ShopifyRefund) -> PaymentStatus {
    if refund.transactions.nodes.is_empty() {
        return PaymentStatus::Pending;
    }
    if refund
        .transactions
        .nodes
        .iter()
        .any(|transaction| transaction.status == "SUCCESS")
    {
        PaymentStatus::Ok
    } else if refund
        .transactions
        .nodes
        .iter()
        .any(|transaction| matches!(transaction.status.as_str(), "FAILURE" | "ERROR"))
    {
        PaymentStatus::Failed
    } else {
        PaymentStatus::Pending
    }
}

fn shopify_interval_unit(interval: &str) -> String {
    match interval.to_ascii_lowercase().as_str() {
        "day" | "daily" => "day",
        "week" | "weekly" => "week",
        "month" | "monthly" => "month",
        "year" | "annual" | "yearly" => "year",
        _ => "month",
    }
    .to_string()
}

fn currency_code(currency: &str) -> String {
    super::store::currency_code(currency)
}

fn amount_to_thousandths(amount: &str) -> SyncResult<i64> {
    let value = amount.trim();
    if value.is_empty() {
        return Err(SyncError::Shopify("empty Shopify money amount".to_string()));
    }

    let negative = value.starts_with('-');
    let unsigned = value
        .strip_prefix('-')
        .or_else(|| value.strip_prefix('+'))
        .unwrap_or(value);
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    let whole = whole
        .parse::<i64>()
        .map_err(|error| SyncError::Shopify(format!("invalid Shopify money amount: {error}")))?;
    let mut fraction = fraction.chars().take(3).collect::<String>();
    while fraction.len() < 3 {
        fraction.push('0');
    }
    let fraction = fraction
        .parse::<i64>()
        .map_err(|error| SyncError::Shopify(format!("invalid Shopify money amount: {error}")))?;
    let amount = whole.saturating_mul(1000).saturating_add(fraction);
    Ok(if negative { -amount } else { amount })
}

fn timestamp_to_unix(value: &str) -> SyncResult<i64> {
    rfc3339_to_unix(value)
        .map_err(|error| SyncError::Shopify(format!("invalid Shopify timestamp: {error}")))
}

fn timestamp_to_unix_opt(value: Option<&str>) -> SyncResult<Option<i64>> {
    value.map(timestamp_to_unix).transpose()
}

fn sync_timestamp(timestamp: i64) -> SyncResult<String> {
    unix_to_rfc3339(timestamp)
        .ok_or_else(|| SyncError::Shopify(format!("invalid sync timestamp {timestamp}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_shopify_money_as_thousandths() {
        assert_eq!(amount_to_thousandths("12").unwrap(), 12_000);
        assert_eq!(amount_to_thousandths("12.3").unwrap(), 12_300);
        assert_eq!(amount_to_thousandths("12.34").unwrap(), 12_340);
        assert_eq!(amount_to_thousandths("12.3456").unwrap(), 12_345);
        assert_eq!(amount_to_thousandths("-1.20").unwrap(), -1_200);
    }

    #[test]
    fn builds_shopify_graphql_endpoint() {
        assert_eq!(
            shopify_graphql_endpoint("example.myshopify.com").unwrap(),
            "https://example.myshopify.com/admin/api/2026-07/graphql.json"
        );
        assert_eq!(
            shopify_graphql_endpoint("https://example.myshopify.com/").unwrap(),
            "https://example.myshopify.com/admin/api/2026-07/graphql.json"
        );
    }
}
