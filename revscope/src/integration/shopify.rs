use std::sync::Arc;

use log::info;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

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
use crate::settings::{hostname_from_input, is_dns_hostname};
use crate::util::{rfc3339_to_unix, truncate_chars, unix_to_rfc3339};

const PROVIDER: &str = "shopify";
const SHOPIFY_API_VERSION: &str = "2026-07";
const SHOPIFY_PAGE_LIMIT: i32 = 100;
const MYSHOPIFY_SUFFIX: &str = ".myshopify.com";
/// Long enough for Shopify's error JSON, short enough to show in the settings page.
const MAX_ERROR_BODY_CHARS: usize = 1000;

pub(crate) const INVALID_SHOP_DOMAIN: &str =
    "Shop domain must be the store's myshopify.com domain, like example.myshopify.com";

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
/// stores the shop domain, `encrypted_api_key` the Dev Dashboard app's client
/// ID, and `encrypted_secret` its client secret.
pub(crate) async fn load_due_integrations(
    server_context: Arc<ServerContext>,
) -> SyncResult<Vec<ShopifyIntegration>> {
    let conn = open_main_database_read(server_context).await?;
    let rows = conn.query(&due_integrations_sql(), &[]).await?;

    Ok(rows
        .iter()
        .map(|row| {
            let last_success_started_at: Option<i64> = row.get(5);
            let last_full_success_started_at: Option<i64> = row.get(6);
            ShopifyIntegration {
                id: row.get(0),
                project_id: row.get(1),
                // Blank, the sync reports the missing domain like a missing credential.
                shop_domain: row.get::<_, Option<String>>(2).unwrap_or_default(),
                encrypted_client_id: row.get(3),
                encrypted_client_secret: row.get(4),
                credentials_version: row.get(7),
                last_success_started_at,
                sync_kind: sync_kind(
                    last_full_success_started_at,
                    PAYMENT_FULL_SYNC_INTERVAL_SECS,
                ),
            }
        })
        .collect())
}

fn due_integrations_sql() -> String {
    due_payment_integrations_sql(
        "id,
         project_id::text AS project_id,
         external_account_id,
         encrypted_api_key,
         encrypted_secret,
         EXTRACT(EPOCH FROM last_sync_success_at)::BIGINT,
         EXTRACT(EPOCH FROM last_full_sync_success_at)::BIGINT,
         credentials_version",
        PROVIDER,
        // Any one credential counts as connected, so a row missing the shop or the
        // other credential still loads and the sync reports which, instead of the
        // connection sitting silently idle.
        "(encrypted_api_key IS NOT NULL OR encrypted_secret IS NOT NULL)",
    )
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
        integration.credentials_version,
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
    encrypted_client_id: Option<Vec<u8>>,
    encrypted_client_secret: Option<Vec<u8>>,
    credentials_version: i64,
    last_success_started_at: Option<i64>,
    sync_kind: SyncKind,
}

async fn sync_integration(
    server_context: Arc<ServerContext>,
    integration: &ShopifyIntegration,
) -> SyncResult<SyncCounts> {
    let project =
        super::sync::ensure_project_cached(server_context.clone(), &integration.project_id).await?;
    let client_id = decrypt_secret(
        integration.encrypted_client_id.as_deref(),
        SyncError::MissingShopifyCredential("client ID"),
        || SyncError::InvalidShopifyCredential("client ID"),
    )?;
    let client_secret = decrypt_secret(
        integration.encrypted_client_secret.as_deref(),
        SyncError::MissingShopifyCredential("client secret"),
        || SyncError::InvalidShopifyCredential("client secret"),
    )?;
    let client = ShopifyClient::connect(
        server_context.http.clone(),
        &integration.shop_domain,
        &client_id,
        &client_secret,
    )
    .await?;
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
    /// Trades the app's client credentials for an Admin API token. A token lasts
    /// 24 hours, far longer than a sync, so each sync attempt gets a fresh one
    /// rather than storing it.
    async fn connect(
        client: reqwest::Client,
        shop_domain: &str,
        client_id: &str,
        client_secret: &str,
    ) -> SyncResult<Self> {
        let shop_domain = normalize_shop_domain(shop_domain)?;
        let request = token_request(&client, &shop_domain, client_id, client_secret)?;
        let response = client.execute(request).await?;
        let status = response.status();
        let text = response.text().await?;
        let access_token = access_token_from_response(status, &text)?;
        Ok(Self {
            client,
            endpoint: graphql_endpoint(&shop_domain),
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
            return Err(SyncError::Shopify(request_failed_message(status, &text)));
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

fn graphql_endpoint(shop_domain: &str) -> String {
    format!("https://{shop_domain}/admin/api/{SHOPIFY_API_VERSION}/graphql.json")
}

/// Shopify's client credentials grant, which takes no scope: the token carries
/// whatever the store granted the installed app.
fn token_request(
    client: &reqwest::Client,
    shop_domain: &str,
    client_id: &str,
    client_secret: &str,
) -> SyncResult<reqwest::Request> {
    let body = serde_urlencoded::to_string([
        ("grant_type", "client_credentials"),
        ("client_id", client_id),
        ("client_secret", client_secret),
    ])
    .map_err(|error| SyncError::Shopify(format!("encoding Shopify token request: {error}")))?;
    Ok(client
        .post(format!("https://{shop_domain}/admin/oauth/access_token"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
        .build()?)
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

#[derive(Deserialize)]
struct TokenError {
    error: Option<String>,
    error_description: Option<String>,
}

fn access_token_from_response(status: reqwest::StatusCode, body: &str) -> SyncResult<String> {
    if status.is_success() {
        return serde_json::from_str::<TokenResponse>(body)
            .map(|token| token.access_token)
            .map_err(|error| {
                SyncError::Shopify(format!(
                    "Shopify's token response had no access token ({error}): {}",
                    truncate_chars(body, MAX_ERROR_BODY_CHARS)
                ))
            });
    }
    let error = serde_json::from_str::<TokenError>(body).ok();
    let code = error.as_ref().and_then(|error| error.error.as_deref());
    // Shopify's own wording for this one doesn't say what to do about it.
    if code == Some("shop_not_permitted") {
        return Err(SyncError::Shopify(
            "Shopify only issues tokens to an app in the same organization as the store. \
             Create the app in the Dev Dashboard of the organization that owns this store."
                .to_string(),
        ));
    }
    let detail = error
        .as_ref()
        .and_then(|error| {
            error
                .error_description
                .as_deref()
                .or(error.error.as_deref())
        })
        .map(str::to_string)
        .unwrap_or_else(|| truncate_chars(body, MAX_ERROR_BODY_CHARS).to_string());
    // Only a rejection should send someone to rotate credentials; an outage or
    // throttle says nothing about them.
    // Shopify answers most OAuth errors with 400, so there it's the code that
    // says whether the credentials were the problem.
    let message = match (status.as_u16(), code) {
        (401 | 403, _) | (400, Some("invalid_client")) => {
            format!("Shopify refused the client ID and secret with {status}: {detail}")
        }
        (400, _) => format!("Shopify rejected the token request with {status}: {detail}"),
        (404, _) => format!(
            "Shopify answered the token request with {status}: {detail}. \
             Check the shop domain and client ID."
        ),
        _ => format!("Shopify token request failed with {status}: {detail}"),
    };
    Err(SyncError::Shopify(message))
}

fn normalize_shop_domain(shop_domain: &str) -> SyncResult<String> {
    if shop_domain.trim().is_empty() {
        return Err(SyncError::Shopify(
            "Shopify integration has no shop domain".to_string(),
        ));
    }
    myshopify_host(shop_domain).ok_or_else(|| SyncError::Shopify(INVALID_SHOP_DOMAIN.to_string()))
}

/// The `<shop>.myshopify.com` host of a domain or URL, or `None` for any other
/// host. Syncs send the app's client secret and the token it buys here from
/// inside our network, so a customer-chosen host must not be reachable,
/// including through rows saved before this check existed.
pub(crate) fn myshopify_host(value: &str) -> Option<String> {
    let host = hostname_from_input(value.trim().trim_end_matches('/'))?;
    let shop = host.strip_suffix(MYSHOPIFY_SUFFIX)?;
    (is_dns_hostname(&host) && !shop.contains('.')).then_some(host)
}

fn request_failed_message(status: reqwest::StatusCode, body: &str) -> String {
    let clipped = truncate_chars(body, MAX_ERROR_BODY_CHARS);
    let ellipsis = if clipped.len() < body.len() {
        "…"
    } else {
        ""
    };
    format!("Shopify GraphQL request failed with {status}: {clipped}{ellipsis}")
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
        for value in ["example.myshopify.com", "https://example.myshopify.com/"] {
            assert_eq!(
                graphql_endpoint(&normalize_shop_domain(value).unwrap()),
                "https://example.myshopify.com/admin/api/2026-07/graphql.json"
            );
        }
    }

    #[test]
    fn reads_the_token_shopify_grants() {
        let body = r#"{"access_token":"shpat_abc","scope":"read_orders","expires_in":86399}"#;
        assert_eq!(
            access_token_from_response(reqwest::StatusCode::OK, body).unwrap(),
            "shpat_abc"
        );
    }

    #[test]
    fn a_refused_token_request_says_why() {
        let error = access_token_from_response(
            reqwest::StatusCode::BAD_REQUEST,
            r#"{"error":"invalid_client","error_description":"Client authentication failed"}"#,
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("refused the client ID and secret"),
            "{error}"
        );
        assert!(error.contains("Client authentication failed"), "{error}");
        assert!(error.contains("400"), "{error}");
    }

    /// The rows `due_integrations_sql` picks among these, with every other
    /// project's Shopify row hidden so its batch limit can't crowd them out.
    async fn due_among(conn: &crate::db::main::Connection, project_ids: &[i32]) -> Vec<i32> {
        enum Probe {
            Db(tokio_postgres::Error),
            Due(Vec<i32>),
        }
        impl From<tokio_postgres::Error> for Probe {
            fn from(error: tokio_postgres::Error) -> Self {
                Self::Db(error)
            }
        }
        // Rolled back by returning `Err`, so the hiding never reaches other tests.
        let probe = conn
            .in_transaction(async |conn| -> Result<(), Probe> {
                conn.execute(
                    "UPDATE integrations SET disabled_at = NOW()
                     WHERE integration_type = 'shopify' AND NOT project_id = ANY($1)",
                    &[&project_ids],
                )
                .await?;
                let rows = conn.query(&due_integrations_sql(), &[]).await?;
                Err(Probe::Due(rows.iter().map(|row| row.get(0)).collect()))
            })
            .await;
        match probe {
            Err(Probe::Due(ids)) => ids,
            Err(Probe::Db(error)) => panic!("query due integrations: {error}"),
            Ok(()) => unreachable!(),
        }
    }

    #[tokio::test]
    async fn a_connection_missing_its_shop_or_a_credential_still_loads_to_report_it() {
        let (_server_context, conn) = crate::test_utils::main_test_db("shopify-due").await;
        let account_id = crate::test_utils::insert_test_account(&conn, "shopify_due").await;
        let secret = crate::crypto::encrypt(b"shpss_x");
        let client_id = crate::crypto::encrypt(b"client_x");
        struct Case<'a> {
            name: &'a str,
            shop: Option<&'a str>,
            client_id: Option<&'a Vec<u8>>,
            secret: Option<&'a Vec<u8>>,
            due: bool,
        }
        let shop = Some("a.myshopify.com");
        let cases = [
            Case {
                name: "secret_only",
                shop,
                client_id: None,
                secret: Some(&secret),
                due: true,
            },
            Case {
                name: "id_only",
                shop,
                client_id: Some(&client_id),
                secret: None,
                due: true,
            },
            Case {
                name: "no_shop",
                shop: None,
                client_id: Some(&client_id),
                secret: Some(&secret),
                due: true,
            },
            Case {
                name: "neither",
                shop,
                client_id: None,
                secret: None,
                due: false,
            },
        ];
        let mut project_ids = Vec::new();
        let mut expected = Vec::new();
        for case in cases {
            let name = format!("p{}{}", account_id.replace('-', ""), case.name);
            let project_id: i32 = conn
                .query_one(
                    "INSERT INTO projects (account_id, name) VALUES ($1::text::uuid, $2) RETURNING id",
                    &[&account_id, &name],
                )
                .await
                .expect("insert project")
                .get(0);
            let integration_id: i32 = conn
                .query_one(
                    "INSERT INTO integrations
                        (project_id, integration_type, external_account_id,
                         encrypted_api_key, encrypted_secret)
                     VALUES ($1, 'shopify', $2, $3, $4)
                     RETURNING id",
                    &[&project_id, &case.shop, &case.client_id, &case.secret],
                )
                .await
                .expect("insert integration")
                .get(0);
            project_ids.push(project_id);
            if case.due {
                expected.push(integration_id);
            }
        }

        let mut loaded = due_among(&conn, &project_ids).await;
        conn.execute(
            "DELETE FROM accounts WHERE id = $1::text::uuid",
            &[&account_id],
        )
        .await
        .expect("delete account");
        loaded.sort_unstable();
        expected.sort_unstable();
        assert_eq!(loaded, expected);
    }

    #[test]
    fn another_400_does_not_blame_the_credentials() {
        let error = access_token_from_response(
            reqwest::StatusCode::BAD_REQUEST,
            r#"{"error":"invalid_request","error_description":"The app isn't installed"}"#,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("rejected the token request"), "{error}");
        assert!(error.contains("isn't installed"), "{error}");
        assert!(!error.contains("client ID and secret"), "{error}");
    }

    #[test]
    fn a_success_without_a_token_says_it_came_from_shopify() {
        let error = access_token_from_response(reqwest::StatusCode::OK, "<html>Closed</html>")
            .unwrap_err()
            .to_string();
        assert!(error.contains("Shopify's token response"), "{error}");
        assert!(error.contains("<html>Closed</html>"), "{error}");
    }

    #[test]
    fn a_missing_store_points_at_the_domain_not_the_secret() {
        let error = access_token_from_response(reqwest::StatusCode::NOT_FOUND, "Not Found")
            .unwrap_err()
            .to_string();
        assert!(error.contains("Check the shop domain"), "{error}");
        assert!(!error.contains("refused"), "{error}");
    }

    #[test]
    fn an_outage_or_throttle_does_not_blame_the_credentials() {
        for status in [
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            reqwest::StatusCode::SERVICE_UNAVAILABLE,
        ] {
            let error = access_token_from_response(status, "try later")
                .unwrap_err()
                .to_string();
            assert!(error.contains("token request failed"), "{error}");
            assert!(!error.contains("refused"), "{error}");
        }
    }

    #[test]
    fn asks_for_a_token_with_the_client_credentials_grant() {
        let request = token_request(
            &reqwest::Client::new(),
            "example.myshopify.com",
            "the id",
            "shpss_&secret",
        )
        .unwrap();
        assert_eq!(request.method(), reqwest::Method::POST);
        assert_eq!(
            request.url().as_str(),
            "https://example.myshopify.com/admin/oauth/access_token"
        );
        assert_eq!(
            request.headers()["content-type"],
            "application/x-www-form-urlencoded"
        );
        let body = request.body().and_then(reqwest::Body::as_bytes).unwrap();
        let mut form: Vec<(String, String)> = serde_urlencoded::from_bytes(body).unwrap();
        form.sort();
        assert_eq!(
            form,
            [
                ("client_id".to_string(), "the id".to_string()),
                ("client_secret".to_string(), "shpss_&secret".to_string()),
                ("grant_type".to_string(), "client_credentials".to_string()),
            ]
        );
    }

    #[test]
    fn a_store_outside_the_apps_organization_says_where_to_create_the_app() {
        let error = access_token_from_response(
            reqwest::StatusCode::BAD_REQUEST,
            r#"{"error":"shop_not_permitted","error_description":"Client credentials cannot be performed on this shop."}"#,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("Dev Dashboard"), "{error}");
    }

    #[test]
    fn accepts_a_myshopify_domain_however_it_is_entered() {
        for value in [
            "example.myshopify.com",
            " Example.MyShopify.com/ ",
            "example.myshopify.com.",
            "https://example.myshopify.com/admin",
            "//example.myshopify.com",
        ] {
            assert_eq!(
                myshopify_host(value).as_deref(),
                Some("example.myshopify.com"),
                "{value}"
            );
        }
    }

    #[test]
    fn refuses_every_host_but_a_single_shop_on_myshopify_com() {
        for value in [
            "evil.example",
            "169.254.169.254",
            "127.0.0.1:8443",
            "[::1]:8443",
            "https://localhost:8443",
            "myshopify.com",
            ".myshopify.com",
            "evilmyshopify.com",
            "a.b.myshopify.com",
            "example.myshopify.com.evil.example",
            "https://admin.shopify.com/store/example",
            "example.myshopify.com:8443",
            "https://example.myshopify.com:8443",
            // Each ends in the suffix, but the host is evil.example.
            "evil.example/x.myshopify.com",
            "evil.example?.myshopify.com",
            "evil.example#.myshopify.com",
            "https://evil.example/x.myshopify.com",
            "https://evil.example#.myshopify.com",
            "https://evil.example\\.myshopify.com",
            "https://example.myshopify.com@evil.example",
            "example.myshopify.com@evil.example",
        ] {
            assert_eq!(myshopify_host(value), None, "{value}");
            assert!(normalize_shop_domain(value).is_err(), "{value}");
        }
    }

    #[test]
    fn an_empty_shop_domain_says_so() {
        let error = normalize_shop_domain("  ").unwrap_err().to_string();
        assert!(error.contains("no shop domain"), "{error}");
    }

    #[test]
    fn a_failed_request_keeps_a_short_body_whole() {
        assert_eq!(
            request_failed_message(reqwest::StatusCode::UNAUTHORIZED, "bad token"),
            "Shopify GraphQL request failed with 401 Unauthorized: bad token"
        );
        let exact = "a".repeat(MAX_ERROR_BODY_CHARS);
        assert!(
            request_failed_message(reqwest::StatusCode::UNAUTHORIZED, &exact).ends_with(&exact)
        );
    }

    #[test]
    fn a_failed_request_clips_a_long_body_on_a_character_boundary() {
        let body = "é".repeat(MAX_ERROR_BODY_CHARS + 500);
        let message = request_failed_message(reqwest::StatusCode::BAD_GATEWAY, &body);
        assert_eq!(
            message,
            format!(
                "Shopify GraphQL request failed with 502 Bad Gateway: {}…",
                "é".repeat(MAX_ERROR_BODY_CHARS)
            )
        );
    }
}
