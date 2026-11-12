use std::sync::Arc;

use fxhash::{FxHashMap, FxHashSet};
use log::{info, warn};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use time::{Date, Month, OffsetDateTime};

use super::attribution;
use super::schedule::{
    self, PaymentIntegration, SyncOutcome, decrypt_secret, load_due_payment_integrations,
};
use super::store::{
    PaymentStatus, PaymentType, PaymentUpsert, PlanInterval, ProviderObjectRef, Reported,
    SubscriptionPlan, SubscriptionUpsert, minor_units_to_thousandths, store_provider_object,
    subscription_interval, upsert_payment, upsert_subscription,
};
use super::sync::{SyncCounts, SyncError, SyncKind, SyncResult};
use crate::db::{ProjectCache, ProjectConnection};
use crate::server_context::ServerContext;
use crate::util::{date_to_unix_utc_midnight, parse_iso_date, rfc3339_to_unix, unix_to_rfc3339};

const PROVIDER: &str = "square";
const SQUARE_API_BASE_URL: &str = "https://connect.squareup.com/v2";
const SQUARE_API_VERSION: &str = "2026-05-20";
const SQUARE_PAYMENTS_PAGE_LIMIT: i32 = 100;
const SQUARE_REFUNDS_PAGE_LIMIT: i32 = 100;
const SQUARE_SUBSCRIPTIONS_PAGE_LIMIT: i32 = 100;
const SQUARE_INVOICES_PAGE_LIMIT: i32 = 200;

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
    let access_token = decrypt_secret(
        integration.encrypted_secret.as_deref(),
        SyncError::MissingSquareSecret,
        || SyncError::InvalidSquareSecret,
    )?;
    let client = SquareClient::new(&server_context.http, &access_token);
    let since = Some(integration.since());

    let mut counts = SyncCounts::default();
    let external_account_id = integration.external_account_id(PROVIDER);

    if integration.sync_kind == SyncKind::Full {
        sync_subscriptions(
            &server_context,
            &project,
            &client,
            &external_account_id,
            None,
            &mut counts,
        )
        .await?;
        sync_invoices_for_all_locations(&server_context, &project, &client, &mut counts).await?;
    }

    sync_payments(
        &server_context,
        &project,
        &client,
        &external_account_id,
        None,
        since,
        &mut counts,
    )
    .await?;
    sync_refunds(
        &server_context,
        &project,
        &client,
        &external_account_id,
        None,
        since,
        &mut counts,
    )
    .await?;
    if integration.sync_kind == SyncKind::Full {
        attribution::attribute_full_sync(server_context.clone(), &project, "square").await?;
    }

    info!(
        "synced Square integration {} ({}): inserted={}, updated={}, deleted={}",
        integration.id,
        integration.sync_kind.as_str(),
        counts.records_inserted,
        counts.records_updated,
        counts.records_deleted
    );
    Ok(counts)
}

struct SquareClient {
    http: reqwest::Client,
    access_token: String,
    api_base_url: String,
}

impl SquareClient {
    fn new(http: &reqwest::Client, access_token: &str) -> Self {
        Self {
            http: http.clone(),
            access_token: access_token.to_string(),
            api_base_url: SQUARE_API_BASE_URL.to_string(),
        }
    }

    async fn get<T>(&self, path: &str, query: &[(&str, String)]) -> SyncResult<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        let response = self
            .http
            .get(self.url_with_query(path, query)?)
            .header("authorization", format!("Bearer {}", self.access_token))
            .header("square-version", SQUARE_API_VERSION)
            .send()
            .await?;
        parse_square_response(response).await
    }

    async fn post<T, B>(&self, path: &str, body: &B) -> SyncResult<T>
    where
        T: for<'de> Deserialize<'de>,
        B: Serialize + ?Sized,
    {
        let response = self
            .http
            .post(self.url(path))
            .header("authorization", format!("Bearer {}", self.access_token))
            .header("square-version", SQUARE_API_VERSION)
            .json(body)
            .send()
            .await?;
        parse_square_response(response).await
    }

    fn url(&self, path: &str) -> String {
        format!(
            "{}/{}",
            self.api_base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }

    fn url_with_query(&self, path: &str, query: &[(&str, String)]) -> SyncResult<String> {
        let url = self.url(path);
        if query.is_empty() {
            return Ok(url);
        }
        let query = serde_urlencoded::to_string(query).map_err(|error| {
            SyncError::Square(format!("failed to encode Square query: {error}"))
        })?;
        Ok(format!("{url}?{query}"))
    }
}

async fn parse_square_response<T>(response: reqwest::Response) -> SyncResult<T>
where
    T: for<'de> Deserialize<'de>,
{
    let status = response.status();
    let text = response.text().await?;
    if !status.is_success() {
        return Err(SyncError::Square(format!(
            "Square API request failed with {status}: {text}"
        )));
    }
    serde_json::from_str(&text).map_err(Into::into)
}

async fn sync_payments(
    server_context: &ServerContext,
    project: &Arc<ProjectCache>,
    client: &SquareClient,
    external_account_id: &str,
    location_id: Option<&str>,
    since: Option<i64>,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    let mut cursor: Option<String> = None;
    loop {
        let mut query = vec![
            ("limit", SQUARE_PAYMENTS_PAGE_LIMIT.to_string()),
            ("sort_order", "ASC".to_string()),
        ];
        if let Some(cursor) = cursor.as_deref() {
            query.push(("cursor", cursor.to_string()));
        }
        if let Some(location_id) = location_id {
            query.push(("location_id", location_id.to_string()));
        }
        if let Some(since) = since {
            query.push(("updated_at_begin_time", format_square_time(since)?));
            query.push(("sort_field", "UPDATED_AT".to_string()));
        } else {
            query.push(("begin_time", format_square_time(0)?));
        }

        let page: ListPaymentsResponse = client.get("payments", &query).await?;
        {
            let conn = crate::db::open_project_database_write_for(server_context, project)?;
            for payment in page.payments {
                let stored = store_provider_object(
                    &conn,
                    PROVIDER,
                    external_account_id,
                    ProviderObjectRef {
                        object_type: "payment",
                        object_id: &payment.id,
                        created_at: Some(parse_square_timestamp(payment.created_at.as_deref())?),
                        updated_at: parse_optional_square_timestamp(payment.updated_at.as_deref())?,
                    },
                    &payment,
                    counts,
                )?;
                if stored.changed {
                    let provider_payment_id = payment.id.clone();
                    store_payment(&conn, stored.id, &payment, counts)?;
                    attribution::attribute_payment_by_provider_id(
                        &conn,
                        PROVIDER,
                        &provider_payment_id,
                    )?;
                }
            }
        }
        if page.cursor.is_none() {
            break;
        }
        cursor = page.cursor;
    }
    Ok(())
}

async fn sync_refunds(
    server_context: &ServerContext,
    project: &Arc<ProjectCache>,
    client: &SquareClient,
    external_account_id: &str,
    location_id: Option<&str>,
    since: Option<i64>,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    let mut cursor: Option<String> = None;
    loop {
        let mut query = vec![
            ("limit", SQUARE_REFUNDS_PAGE_LIMIT.to_string()),
            ("sort_order", "ASC".to_string()),
        ];
        if let Some(cursor) = cursor.as_deref() {
            query.push(("cursor", cursor.to_string()));
        }
        if let Some(location_id) = location_id {
            query.push(("location_id", location_id.to_string()));
        }
        if let Some(since) = since {
            query.push(("begin_time", format_square_time(since)?));
        } else {
            query.push(("begin_time", format_square_time(0)?));
        }

        let page: ListRefundsResponse = client.get("refunds", &query).await?;
        {
            let conn = crate::db::open_project_database_write_for(server_context, project)?;
            for refund in page.refunds {
                let stored = store_provider_object(
                    &conn,
                    PROVIDER,
                    external_account_id,
                    ProviderObjectRef {
                        object_type: "refund",
                        object_id: &refund.id,
                        created_at: Some(parse_square_timestamp(refund.created_at.as_deref())?),
                        updated_at: parse_optional_square_timestamp(refund.updated_at.as_deref())?,
                    },
                    &refund,
                    counts,
                )?;
                if stored.changed {
                    let provider_payment_id = refund.id.clone();
                    store_refund(&conn, stored.id, &refund, counts)?;
                    attribution::attribute_payment_by_provider_id(
                        &conn,
                        PROVIDER,
                        &provider_payment_id,
                    )?;
                }
            }
        }
        if page.cursor.is_none() {
            break;
        }
        cursor = page.cursor;
    }
    Ok(())
}

/// Lists all subscriptions without a date lower bound.
///
/// Square's SearchSubscriptions API only filters by customer/location/source —
/// not by created or updated time. Client-side early-stop by `created_at` would
/// also miss status and other updates on long-lived subscriptions, so on full
/// syncs we intentionally fetch the full list.
async fn sync_subscriptions(
    server_context: &ServerContext,
    project: &Arc<ProjectCache>,
    client: &SquareClient,
    external_account_id: &str,
    location_id: Option<&str>,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    let mut plan_variations = FxHashMap::default();
    let mut cursor: Option<String> = None;
    loop {
        let body = SearchSubscriptionsRequest::new(cursor.as_deref(), location_id);
        let page: SearchSubscriptionsResponse = client.post("subscriptions/search", &body).await?;
        load_plan_variations(client, &page.subscriptions, &mut plan_variations).await;
        {
            let conn = crate::db::open_project_database_write_for(server_context, project)?;
            for subscription in page.subscriptions {
                let stored = store_provider_object(
                    &conn,
                    PROVIDER,
                    external_account_id,
                    ProviderObjectRef {
                        object_type: "subscription",
                        object_id: &subscription.id,
                        created_at: Some(parse_square_timestamp(
                            subscription.created_at.as_deref(),
                        )?),
                        updated_at: None,
                    },
                    &subscription,
                    counts,
                )?;
                if stored.changed {
                    let variation = subscription
                        .plan_variation_id
                        .as_deref()
                        .and_then(|id| plan_variations.get(id))
                        .and_then(Option::as_ref);
                    let subscription_id =
                        store_subscription(&conn, stored.id, &subscription, variation, counts)?;
                    attribution::attribute_subscription_by_id(&conn, subscription_id)?;
                }
            }
        }
        if page.cursor.is_none() {
            break;
        }
        cursor = page.cursor;
    }
    Ok(())
}

/// Retrieves the catalog plan variations referenced by `subscriptions` that
/// aren't in `plan_variations` yet.
///
/// Cadence and price are only on the catalog object, so every subscription
/// needs its variation. Variations the catalog doesn't return are recorded as
/// absent, so a deleted variation — or a token without `ITEMS_READ` — costs one
/// request rather than one per subscription. A failed retrieval leaves the plan
/// unknown rather than failing the sync, since payments and refunds don't
/// depend on it.
async fn load_plan_variations(
    client: &SquareClient,
    subscriptions: &[SquareSubscription],
    plan_variations: &mut FxHashMap<String, Option<SquarePlanVariationData>>,
) {
    let mut object_ids: Vec<String> = Vec::new();
    let mut requested = FxHashSet::default();
    for subscription in subscriptions {
        let Some(id) = subscription.plan_variation_id.as_deref() else {
            continue;
        };
        if plan_variations.contains_key(id) || !requested.insert(id) {
            continue;
        }
        object_ids.push(id.to_string());
    }
    if object_ids.is_empty() {
        return;
    }

    let request = BatchRetrieveCatalogObjectsRequest {
        object_ids: &object_ids,
    };
    let response: BatchRetrieveCatalogObjectsResponse =
        match client.post("catalog/batch-retrieve", &request).await {
            Ok(response) => response,
            Err(error) => {
                warn!("failed to retrieve Square subscription plan variations: {error}");
                BatchRetrieveCatalogObjectsResponse::default()
            }
        };
    for object in response.objects {
        if let Some(data) = object.subscription_plan_variation_data {
            plan_variations.insert(object.id, Some(data));
        }
    }
    for id in object_ids {
        plan_variations.entry(id).or_insert(None);
    }
}

/// Lists all invoices without a date lower bound.
///
/// Square's ListInvoices / SearchInvoices APIs only filter by location (and
/// optionally customer), not by created/updated time. We intentionally fetch
/// the full list on full syncs so we don't miss updates.
async fn sync_invoices_for_all_locations(
    server_context: &ServerContext,
    project: &Arc<ProjectCache>,
    client: &SquareClient,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    let response: ListLocationsResponse = client.get("locations", &[]).await?;
    for location in response.locations {
        sync_invoices(
            server_context,
            project,
            client,
            &location.id,
            &location.id,
            counts,
        )
        .await?;
    }
    Ok(())
}

async fn sync_invoices(
    server_context: &ServerContext,
    project: &Arc<ProjectCache>,
    client: &SquareClient,
    external_account_id: &str,
    location_id: &str,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    let mut cursor: Option<String> = None;
    loop {
        let mut query = vec![
            ("location_id", location_id.to_string()),
            ("limit", SQUARE_INVOICES_PAGE_LIMIT.to_string()),
        ];
        if let Some(cursor) = cursor.as_deref() {
            query.push(("cursor", cursor.to_string()));
        }

        let page: ListInvoicesResponse = client.get("invoices", &query).await?;
        {
            let conn = crate::db::open_project_database_write_for(server_context, project)?;
            for invoice in page.invoices {
                store_provider_object(
                    &conn,
                    PROVIDER,
                    external_account_id,
                    ProviderObjectRef {
                        object_type: "invoice",
                        object_id: &invoice.id,
                        created_at: Some(parse_square_timestamp(invoice.created_at.as_deref())?),
                        updated_at: parse_optional_square_timestamp(invoice.updated_at.as_deref())?,
                    },
                    &invoice,
                    counts,
                )?;
            }
        }
        if page.cursor.is_none() {
            break;
        }
        cursor = page.cursor;
    }
    Ok(())
}

fn store_subscription(
    conn: &ProjectConnection,
    provider_object_id: i64,
    subscription: &SquareSubscription,
    variation: Option<&SquarePlanVariationData>,
    counts: &mut SyncCounts,
) -> SyncResult<i64> {
    let current_period_end = subscription
        .charged_through_date
        .as_deref()
        .or(subscription.paid_until_date.as_deref())
        .map(parse_square_date)
        .transpose()?;
    let started_at = subscription
        .start_date
        .as_deref()
        .map(parse_square_date)
        .transpose()?
        .or(parse_optional_square_timestamp(
            subscription.created_at.as_deref(),
        )?);
    // Cancelling a Square subscription schedules the cancellation for the end
    // of the active billing period and sets `canceled_date` to that day, so it
    // is the effective end rather than the time of the request, which Square
    // doesn't report.
    let cancel_at = subscription
        .canceled_date
        .as_deref()
        .map(parse_square_date)
        .transpose()?;
    let plan = subscription_plan(
        subscription,
        variation,
        started_at,
        cancel_at,
        OffsetDateTime::now_utc().date(),
    );

    let mut upsert = SubscriptionUpsert::new(PROVIDER, &subscription.id);
    upsert.provider_object_id = Some(provider_object_id);
    upsert.provider_customer_id = subscription.customer_id.as_deref();
    upsert.status = subscription.status.as_deref();
    upsert.plan = SubscriptionPlan {
        interval: plan
            .interval
            .map(|(unit, count)| PlanInterval::new(unit, count)),
        provider_price_id: subscription.plan_variation_id.as_deref(),
        plan_name: plan.name,
        quantity: None,
        currency: plan.currency.as_deref(),
        amount: plan.amount,
    };
    upsert.current_period_end = current_period_end;
    upsert.started_at = started_at;
    upsert.ended_at = Reported::or_cleared(subscription_end_at(
        subscription,
        cancel_at,
        current_period_end,
    ));
    upsert.cancel_at = Reported::or_cleared(cancel_at);
    upsert_subscription(conn, &upsert, counts)
}

fn store_payment(
    conn: &ProjectConnection,
    provider_object_id: i64,
    payment: &SquarePayment,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    let Some(money) = payment
        .total_money
        .as_ref()
        .or(payment.amount_money.as_ref())
    else {
        return Ok(());
    };
    let Some(currency) = money.currency_code() else {
        return Ok(());
    };
    let amount = square_amount_to_thousandths(money.amount.unwrap_or(0), &currency);
    let fee_amount = payment.processing_fee_amount(&currency);
    let paid_at = parse_square_timestamp(payment.created_at.as_deref())?;
    // Square doesn't link payments to subscriptions directly; the subscription's
    // invoice carries the order the payment settled.
    let invoice_link = match payment.order_id.as_deref() {
        Some(order_id) => invoice_subscription_link(conn, order_id)?,
        None => None,
    };
    let payment_type = if invoice_link.is_some() {
        PaymentType::Subscription
    } else {
        PaymentType::Single
    };

    let mut upsert = PaymentUpsert::new(
        PROVIDER,
        &payment.id,
        payment_type,
        payment_status(&payment.status),
        amount,
        &currency,
        paid_at,
    );
    upsert.provider_object_id = Some(provider_object_id);
    upsert.provider_object_type = Some("payment");
    upsert.subscription_id = invoice_link.as_ref().map(|link| link.subscription_id);
    upsert.interval = invoice_link
        .as_ref()
        .map(|link| subscription_interval(conn, link.subscription_id))
        .transpose()?
        .flatten();
    upsert.provider_status = payment.status.as_deref();
    upsert.provider_order_id = payment.order_id.as_deref();
    upsert.provider_invoice_id = invoice_link
        .as_ref()
        .map(|link| link.provider_invoice_id.as_str());
    upsert.provider_customer_id = payment.customer_id.as_deref();
    upsert.customer_email = payment.buyer_email_address.as_deref();
    upsert.fee_amount = fee_amount;
    upsert.net_amount = fee_amount.map(|fee| amount.saturating_sub(fee.abs()));
    upsert.settlement_currency = Some(&currency);
    upsert.settlement_amount = Some(amount);
    upsert_payment(conn, &upsert, counts)
}

fn store_refund(
    conn: &ProjectConnection,
    provider_object_id: i64,
    refund: &SquareRefund,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    let Some(money) = refund.amount_money.as_ref() else {
        return Ok(());
    };
    let Some(currency) = money.currency_code() else {
        return Ok(());
    };
    let amount = square_amount_to_thousandths(money.amount.unwrap_or(0), &currency);
    let fee_amount = refund
        .processing_fee_amount(&currency)
        .map(|fee| -fee.abs());
    let paid_at = parse_square_timestamp(refund.created_at.as_deref())?;

    let mut upsert = PaymentUpsert::new(
        PROVIDER,
        &refund.id,
        PaymentType::Refund,
        refund_status(&refund.status),
        amount,
        &currency,
        paid_at,
    );
    upsert.provider_object_id = Some(provider_object_id);
    upsert.provider_object_type = Some("refund");
    upsert.provider_status = refund.status.as_deref();
    upsert.provider_order_id = refund.order_id.as_deref();
    upsert.provider_refund_id = Some(&refund.id);
    upsert.parent_provider_payment_id = refund.payment_id.as_deref();
    upsert.fee_amount = fee_amount;
    upsert.net_amount = fee_amount.map(|fee| amount.abs().saturating_sub(fee.abs()));
    upsert_payment(conn, &upsert, counts)
}

/// The recurring price and cadence from the plan phase a subscription is in.
struct SquarePlan<'a> {
    interval: Option<(&'static str, i64)>,
    name: Option<&'a str>,
    currency: Option<String>,
    amount: Option<i64>,
}

impl SquarePlan<'_> {
    /// The plan for a subscription whose phase we couldn't pin down. Leaving the
    /// cadence and price absent lets the upsert keep whatever an earlier sync
    /// recorded instead of replacing it with a guess.
    fn unknown<'a>(name: Option<&'a str>) -> SquarePlan<'a> {
        SquarePlan {
            interval: None,
            name,
            currency: None,
            amount: None,
        }
    }
}

/// Resolves what a subscription is billing as of `today`.
///
/// The subscription itself carries only the order of its phases and a pointer
/// into the catalog for each; cadence and price come from the plan variation.
fn subscription_plan<'a>(
    subscription: &SquareSubscription,
    variation: Option<&'a SquarePlanVariationData>,
    started_at: Option<i64>,
    cancel_at: Option<i64>,
    today: Date,
) -> SquarePlan<'a> {
    let Some(variation) = variation else {
        return SquarePlan::unknown(None);
    };
    let name = variation.name.as_deref();
    let start = started_at.and_then(timestamp_date).unwrap_or(today);
    let as_of = plan_as_of(cancel_at.and_then(timestamp_date), start, today);
    let Some(phase) = active_phase(subscription, variation, start, as_of) else {
        return SquarePlan::unknown(name);
    };

    let money = phase_price(subscription, phase);
    let currency = money.and_then(SquareMoney::currency_code);
    SquarePlan {
        interval: Some(cadence_interval(phase.cadence.as_deref())),
        name,
        amount: match (money.and_then(|money| money.amount), currency.as_deref()) {
            (Some(amount), Some(currency)) => Some(square_amount_to_thousandths(amount, currency)),
            _ => None,
        },
        currency,
    }
}

/// The date whose phase the stored plan should describe.
///
/// A subscription that has already stopped is described by the phase it was in
/// when it stopped, so one canceled during an introductory phase isn't repriced
/// by phases it never reached. A `cancel_at` still in the future hasn't taken
/// effect, and neither has a subscription that hasn't started.
fn plan_as_of(cancel_at: Option<Date>, start: Date, today: Date) -> Date {
    cancel_at.unwrap_or(today).min(today).max(start)
}

fn timestamp_date(at: i64) -> Option<Date> {
    OffsetDateTime::from_unix_timestamp(at)
        .ok()
        .map(OffsetDateTime::date)
}

/// The plan phase in effect on `as_of`.
///
/// Phases run back to back from the subscription's start date, each lasting
/// `periods` cadences. The last phase is normally open-ended, and is also what
/// we settle on when a plan's phases don't cover `as_of`.
fn active_phase<'a>(
    subscription: &SquareSubscription,
    variation: &'a SquarePlanVariationData,
    start: Date,
    as_of: Date,
) -> Option<&'a SquarePlanPhase> {
    let phases = ordered_phases(subscription, variation);
    let last = phases.len().checked_sub(1)?;
    let mut phase_start = start;
    for (index, phase) in phases.iter().enumerate() {
        if index == last {
            return Some(phase);
        }
        let (unit, count) = cadence_interval(phase.cadence.as_deref());
        let Some(periods) = phase.periods.filter(|periods| *periods > 0) else {
            return Some(phase);
        };
        let Some(phase_end) = add_interval(phase_start, unit, count.saturating_mul(periods)) else {
            return Some(phase);
        };
        if as_of < phase_end {
            return Some(phase);
        }
        phase_start = phase_end;
    }
    None
}

/// The catalog phases a subscription runs through, in billing order.
///
/// The subscription's own phase list is authoritative, since it can reorder or
/// omit the plan's phases, but it is only usable when every entry resolves to a
/// catalog phase. Otherwise the plan's own ordering stands in.
fn ordered_phases<'a>(
    subscription: &SquareSubscription,
    variation: &'a SquarePlanVariationData,
) -> Vec<&'a SquarePlanPhase> {
    let mut subscription_phases: Vec<&SquareSubscriptionPhase> =
        subscription.phases.iter().collect();
    subscription_phases.sort_by_key(|phase| phase.ordinal.unwrap_or(i64::MAX));
    let resolved: Vec<&SquarePlanPhase> = subscription_phases
        .iter()
        .filter_map(|phase| {
            let uid = phase.plan_phase_uid.as_deref()?;
            variation
                .phases
                .iter()
                .find(|plan_phase| plan_phase.uid.as_deref() == Some(uid))
        })
        .collect();
    if !resolved.is_empty() && resolved.len() == subscription_phases.len() {
        return resolved;
    }

    let mut phases: Vec<&SquarePlanPhase> = variation.phases.iter().collect();
    phases.sort_by_key(|phase| phase.ordinal.unwrap_or(i64::MAX));
    phases
}

/// What a phase bills per cadence.
///
/// `price_override_money` replaces the catalog price, but only for a statically
/// priced phase; a relatively priced one is billed off the subscription's order
/// template, whose total we don't have.
fn phase_price<'a>(
    subscription: &'a SquareSubscription,
    phase: &'a SquarePlanPhase,
) -> Option<&'a SquareMoney> {
    let pricing = phase.pricing.as_ref();
    if pricing.is_some_and(|pricing| pricing.pricing_type.as_deref() == Some("RELATIVE")) {
        return None;
    }
    subscription
        .price_override_money
        .as_ref()
        .or(pricing.and_then(|pricing| pricing.price_money.as_ref()))
        .or(phase.recurring_price_money.as_ref())
}

/// Maps a Square billing cadence onto an interval unit and count.
fn cadence_interval(cadence: Option<&str>) -> (&'static str, i64) {
    match cadence.unwrap_or_default() {
        "DAILY" => ("day", 1),
        "WEEKLY" => ("week", 1),
        "EVERY_TWO_WEEKS" => ("week", 2),
        "THIRTY_DAYS" => ("day", 30),
        "SIXTY_DAYS" => ("day", 60),
        "NINETY_DAYS" => ("day", 90),
        "MONTHLY" => ("month", 1),
        "EVERY_TWO_MONTHS" => ("month", 2),
        "QUARTERLY" => ("month", 3),
        "EVERY_FOUR_MONTHS" => ("month", 4),
        "EVERY_SIX_MONTHS" => ("month", 6),
        "ANNUAL" => ("year", 1),
        "EVERY_TWO_YEARS" => ("year", 2),
        _ => ("month", 1),
    }
}

/// Advances a date by `count` whole intervals, clamping to the last day of the
/// month the way Square's own billing anchor does.
fn add_interval(date: Date, unit: &str, count: i64) -> Option<Date> {
    match unit {
        "day" => date.checked_add(time::Duration::days(count)),
        "week" => date.checked_add(time::Duration::weeks(count)),
        "month" => add_months(date, count),
        "year" => add_months(date, count.checked_mul(12)?),
        _ => None,
    }
}

fn add_months(date: Date, months: i64) -> Option<Date> {
    let month_index = i64::from(date.year()) * 12 + i64::from(u8::from(date.month())) - 1;
    let shifted = month_index.checked_add(months)?;
    let year = i32::try_from(shifted.div_euclid(12)).ok()?;
    let month = Month::try_from(u8::try_from(shifted.rem_euclid(12) + 1).ok()?).ok()?;
    let day = date.day().min(time::util::days_in_month(month, year));
    Date::from_calendar_date(year, month, day).ok()
}

/// Links a Square order to the subscription whose invoice created it.
struct InvoiceSubscriptionLink {
    subscription_id: i64,
    provider_invoice_id: String,
}

fn invoice_subscription_link(
    conn: &ProjectConnection,
    provider_order_id: &str,
) -> SyncResult<Option<InvoiceSubscriptionLink>> {
    Ok(conn
        .query_row(
            "SELECT s.id, po.provider_object_id
             FROM provider_objects po
             JOIN subscriptions s
                ON s.provider = 'square'
                AND s.provider_subscription_id = json_extract(po.payload, '$.subscription_id')
             WHERE po.provider = 'square'
                AND po.object_type = 'invoice'
                AND json_extract(po.payload, '$.order_id') = ?1
             LIMIT 1",
            params![provider_order_id],
            |row| {
                Ok(InvoiceSubscriptionLink {
                    subscription_id: row.get(0)?,
                    provider_invoice_id: row.get(1)?,
                })
            },
        )
        .optional()?)
}

fn subscription_end_at(
    subscription: &SquareSubscription,
    cancel_at: Option<i64>,
    current_period_end: Option<i64>,
) -> Option<i64> {
    match subscription.status.as_deref() {
        Some("CANCELED") | Some("DEACTIVATED") => cancel_at.or(current_period_end),
        _ => None,
    }
}

fn payment_status(status: &Option<String>) -> PaymentStatus {
    match status.as_deref() {
        Some("COMPLETED") => PaymentStatus::Ok,
        Some("CANCELED") => PaymentStatus::Canceled,
        Some("FAILED") => PaymentStatus::Failed,
        _ => PaymentStatus::Pending,
    }
}

fn refund_status(status: &Option<String>) -> PaymentStatus {
    match status.as_deref() {
        Some("COMPLETED") => PaymentStatus::Ok,
        Some("REJECTED") | Some("FAILED") => PaymentStatus::Failed,
        Some("CANCELED") => PaymentStatus::Canceled,
        _ => PaymentStatus::Pending,
    }
}

fn parse_optional_square_timestamp(value: Option<&str>) -> SyncResult<Option<i64>> {
    value
        .map(|value| parse_square_timestamp(Some(value)))
        .transpose()
}

fn parse_square_timestamp(value: Option<&str>) -> SyncResult<i64> {
    let value =
        value.ok_or_else(|| SyncError::Square("Square object is missing timestamp".to_string()))?;
    rfc3339_to_unix(value)
        .map_err(|error| SyncError::Square(format!("invalid Square timestamp {value:?}: {error}")))
}

fn parse_square_date(value: &str) -> SyncResult<i64> {
    let date = parse_iso_date(value)
        .ok_or_else(|| SyncError::Square(format!("invalid Square date {value:?}")))?;
    Ok(date_to_unix_utc_midnight(date))
}

fn format_square_time(seconds: i64) -> SyncResult<String> {
    unix_to_rfc3339(seconds)
        .ok_or_else(|| SyncError::Square(format!("invalid sync timestamp {seconds}")))
}

fn square_amount_to_thousandths(amount: i64, currency: &str) -> i64 {
    minor_units_to_thousandths(amount, currency)
}

#[derive(Deserialize)]
struct ListPaymentsResponse {
    #[serde(default)]
    payments: Vec<SquarePayment>,
    cursor: Option<String>,
}

#[derive(Deserialize)]
struct ListRefundsResponse {
    #[serde(default)]
    refunds: Vec<SquareRefund>,
    cursor: Option<String>,
}

#[derive(Deserialize)]
struct SearchSubscriptionsResponse {
    #[serde(default)]
    subscriptions: Vec<SquareSubscription>,
    cursor: Option<String>,
}

#[derive(Deserialize)]
struct ListLocationsResponse {
    #[serde(default)]
    locations: Vec<SquareLocation>,
}

#[derive(Deserialize)]
struct SquareLocation {
    id: String,
}

#[derive(Deserialize)]
struct ListInvoicesResponse {
    #[serde(default)]
    invoices: Vec<SquareInvoice>,
    cursor: Option<String>,
}

#[derive(Serialize)]
struct SearchSubscriptionsRequest {
    limit: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    query: Option<SearchSubscriptionsQuery>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    include: Vec<&'static str>,
}

impl SearchSubscriptionsRequest {
    fn new(cursor: Option<&str>, location_id: Option<&str>) -> Self {
        Self {
            limit: SQUARE_SUBSCRIPTIONS_PAGE_LIMIT,
            cursor: cursor.map(ToString::to_string),
            query: location_id.map(|location_id| SearchSubscriptionsQuery {
                filter: SearchSubscriptionsFilter {
                    location_ids: vec![location_id.to_string()],
                },
            }),
            include: vec!["actions"],
        }
    }
}

#[derive(Serialize)]
struct SearchSubscriptionsQuery {
    filter: SearchSubscriptionsFilter,
}

#[derive(Serialize)]
struct SearchSubscriptionsFilter {
    location_ids: Vec<String>,
}

#[derive(Deserialize, Serialize)]
struct SquarePayment {
    id: String,
    status: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
    order_id: Option<String>,
    location_id: Option<String>,
    customer_id: Option<String>,
    buyer_email_address: Option<String>,
    amount_money: Option<SquareMoney>,
    total_money: Option<SquareMoney>,
    #[serde(default)]
    processing_fee: Vec<SquareProcessingFee>,
}

impl SquarePayment {
    fn processing_fee_amount(&self, currency: &str) -> Option<i64> {
        processing_fee_amount(&self.processing_fee, currency)
    }
}

#[derive(Deserialize, Serialize)]
struct SquareRefund {
    id: String,
    status: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
    order_id: Option<String>,
    payment_id: Option<String>,
    location_id: Option<String>,
    amount_money: Option<SquareMoney>,
    #[serde(default)]
    processing_fee: Vec<SquareProcessingFee>,
}

impl SquareRefund {
    fn processing_fee_amount(&self, currency: &str) -> Option<i64> {
        processing_fee_amount(&self.processing_fee, currency)
    }
}

#[derive(Deserialize, Serialize)]
struct SquareSubscription {
    id: String,
    location_id: Option<String>,
    customer_id: Option<String>,
    status: Option<String>,
    created_at: Option<String>,
    start_date: Option<String>,
    canceled_date: Option<String>,
    charged_through_date: Option<String>,
    paid_until_date: Option<String>,
    plan_variation_id: Option<String>,
    price_override_money: Option<SquareMoney>,
    #[serde(default)]
    invoice_ids: Vec<String>,
    #[serde(default)]
    phases: Vec<SquareSubscriptionPhase>,
}

/// One entry in a subscription's phase sequence. It carries no cadence or price
/// of its own — `plan_phase_uid` points at the catalog phase holding those.
#[derive(Deserialize, Serialize)]
struct SquareSubscriptionPhase {
    uid: Option<String>,
    ordinal: Option<i64>,
    order_template_id: Option<String>,
    plan_phase_uid: Option<String>,
}

#[derive(Default, Deserialize)]
struct BatchRetrieveCatalogObjectsResponse {
    #[serde(default)]
    objects: Vec<SquareCatalogObject>,
}

#[derive(Serialize)]
struct BatchRetrieveCatalogObjectsRequest<'a> {
    object_ids: &'a [String],
}

#[derive(Deserialize)]
struct SquareCatalogObject {
    id: String,
    subscription_plan_variation_data: Option<SquarePlanVariationData>,
}

#[derive(Deserialize)]
struct SquarePlanVariationData {
    name: Option<String>,
    #[serde(default)]
    phases: Vec<SquarePlanPhase>,
}

/// A phase of a catalog plan variation: what the subscriber is billed, how
/// often, and for how many billing periods before the next phase takes over.
#[derive(Deserialize)]
struct SquarePlanPhase {
    uid: Option<String>,
    ordinal: Option<i64>,
    cadence: Option<String>,
    /// The number of cadences the phase lasts. Absent means it never ends,
    /// which only the last phase may be.
    periods: Option<i64>,
    recurring_price_money: Option<SquareMoney>,
    pricing: Option<SquarePlanPricing>,
}

#[derive(Deserialize)]
struct SquarePlanPricing {
    #[serde(rename = "type")]
    pricing_type: Option<String>,
    price_money: Option<SquareMoney>,
}

#[derive(Deserialize, Serialize)]
struct SquareInvoice {
    id: String,
    location_id: Option<String>,
    order_id: Option<String>,
    subscription_id: Option<String>,
    status: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
}

#[derive(Deserialize, Serialize)]
struct SquareProcessingFee {
    amount_money: Option<SquareMoney>,
    effective_at: Option<String>,
}

#[derive(Deserialize, Serialize)]
struct SquareMoney {
    amount: Option<i64>,
    #[serde(alias = "currency_code")]
    currency: Option<String>,
}

impl SquareMoney {
    fn currency_code(&self) -> Option<String> {
        self.currency
            .as_ref()
            .map(|currency| currency.to_ascii_uppercase())
    }
}

fn processing_fee_amount(fees: &[SquareProcessingFee], currency: &str) -> Option<i64> {
    let mut amount = 0_i64;
    let mut found = false;
    for fee in fees {
        let Some(money) = fee.amount_money.as_ref() else {
            continue;
        };
        let Some(fee_currency) = money.currency_code() else {
            continue;
        };
        if fee_currency != currency {
            continue;
        }
        amount = amount.saturating_add(square_amount_to_thousandths(
            money.amount.unwrap_or(0),
            &fee_currency,
        ));
        found = true;
    }
    found.then_some(amount)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    /// Thousandths of a dollar, which is how plan amounts are stored.
    const DOLLARS_10: i64 = 10_000;
    const DOLLARS_50: i64 = 50_000;
    const DOLLARS_75: i64 = 75_000;

    fn day(value: &str) -> Date {
        parse_iso_date(value).expect("date")
    }

    fn subscription(value: Value) -> SquareSubscription {
        serde_json::from_value(value).expect("subscription")
    }

    fn plan_variation(value: Value) -> SquarePlanVariationData {
        serde_json::from_value(value).expect("plan variation")
    }

    /// A subscription that started mid-January and runs both plan phases.
    fn active_subscription() -> SquareSubscription {
        subscription(json!({
            "id": "sub_1",
            "status": "ACTIVE",
            "start_date": "2026-01-15",
            "plan_variation_id": "variation_1",
            "phases": [
                {"uid": "sub_phase_1", "ordinal": 0, "plan_phase_uid": "phase_intro"},
                {"uid": "sub_phase_2", "ordinal": 1, "plan_phase_uid": "phase_standard"},
            ],
        }))
    }

    /// A $10 introductory price for three months, then $50 a month indefinitely.
    fn intro_then_standard() -> SquarePlanVariationData {
        plan_variation(json!({
            "name": "Pro",
            "phases": [
                {
                    "uid": "phase_intro",
                    "ordinal": 0,
                    "cadence": "MONTHLY",
                    "periods": 3,
                    "recurring_price_money": {"amount": 1_000, "currency": "USD"},
                },
                {
                    "uid": "phase_standard",
                    "ordinal": 1,
                    "cadence": "MONTHLY",
                    "recurring_price_money": {"amount": 5_000, "currency": "USD"},
                },
            ],
        }))
    }

    /// A `YYYY-MM-DD` date as the unix timestamp `store_subscription` parses it
    /// into before handing it to the plan resolver.
    fn at(value: &str) -> Option<i64> {
        Some(parse_square_date(value).expect("date"))
    }

    #[test]
    fn timestamps_are_read_at_their_offset() {
        let utc = parse_square_timestamp(Some("2026-01-15T12:00:00.123Z")).expect("utc");
        let pacific = parse_square_timestamp(Some("2026-01-15T04:00:00-08:00")).expect("offset");
        assert_eq!(utc, pacific);
        assert_eq!(at("2026-01-15"), Some(utc - 12 * 3600));
        assert!(parse_square_timestamp(Some("2026-01-15")).is_err());
        assert!(parse_square_timestamp(None).is_err());
        assert_eq!(
            format_square_time(utc - 12 * 3600).expect("in range"),
            "2026-01-15T00:00:00Z"
        );
    }

    #[test]
    fn the_introductory_phase_gives_way_to_the_standard_one() {
        let subscription = active_subscription();
        let variation = intro_then_standard();
        let started_at = at("2026-01-15");

        let intro = subscription_plan(
            &subscription,
            Some(&variation),
            started_at,
            None,
            day("2026-03-01"),
        );
        assert_eq!(intro.amount, Some(DOLLARS_10));
        assert_eq!(intro.currency.as_deref(), Some("USD"));
        assert_eq!(intro.name, Some("Pro"));
        assert_eq!(intro.interval, Some(("month", 1)));

        // The three introductory periods run out on April 15th.
        for today in ["2026-04-15", "2026-09-01", "2031-01-01"] {
            let standard = subscription_plan(
                &subscription,
                Some(&variation),
                started_at,
                None,
                day(today),
            );
            assert_eq!(standard.amount, Some(DOLLARS_50), "on {today}");
        }
    }

    #[test]
    fn a_canceled_subscription_keeps_the_phase_it_ended_in() {
        let mut subscription = active_subscription();
        subscription.status = Some("CANCELED".to_string());
        let variation = intro_then_standard();

        let plan = subscription_plan(
            &subscription,
            Some(&variation),
            at("2026-01-15"),
            at("2026-02-01"),
            day("2026-09-01"),
        );

        assert_eq!(plan.amount, Some(DOLLARS_10));
    }

    #[test]
    fn a_cancellation_scheduled_for_later_does_not_rewind_the_phase() {
        let variation = intro_then_standard();

        let plan = subscription_plan(
            &active_subscription(),
            Some(&variation),
            at("2026-01-15"),
            at("2026-12-01"),
            day("2026-06-01"),
        );

        assert_eq!(plan.amount, Some(DOLLARS_50));
    }

    #[test]
    fn a_subscription_that_has_not_started_is_on_its_first_phase() {
        let variation = intro_then_standard();

        let plan = subscription_plan(
            &active_subscription(),
            Some(&variation),
            at("2026-01-15"),
            None,
            day("2025-12-01"),
        );

        assert_eq!(plan.amount, Some(DOLLARS_10));
    }

    #[test]
    fn a_price_override_replaces_a_static_phase_price() {
        let mut subscription = active_subscription();
        subscription.price_override_money = Some(SquareMoney {
            amount: Some(7_500),
            currency: Some("USD".to_string()),
        });
        let variation = intro_then_standard();

        let plan = subscription_plan(
            &subscription,
            Some(&variation),
            at("2026-01-15"),
            None,
            day("2026-09-01"),
        );

        assert_eq!(plan.amount, Some(DOLLARS_75));
    }

    #[test]
    fn a_relatively_priced_phase_has_no_amount_and_ignores_the_override() {
        let mut subscription = subscription(json!({
            "id": "sub_1",
            "start_date": "2026-01-15",
            "plan_variation_id": "variation_1",
            "phases": [{"uid": "sub_phase_1", "ordinal": 0, "plan_phase_uid": "phase_only"}],
        }));
        subscription.price_override_money = Some(SquareMoney {
            amount: Some(7_500),
            currency: Some("USD".to_string()),
        });
        let variation = plan_variation(json!({
            "name": "Itemized",
            "phases": [{
                "uid": "phase_only",
                "ordinal": 0,
                "cadence": "MONTHLY",
                "pricing": {"type": "RELATIVE", "discount_ids": ["discount_1"]},
            }],
        }));

        let plan = subscription_plan(
            &subscription,
            Some(&variation),
            at("2026-01-15"),
            None,
            day("2026-09-01"),
        );

        assert_eq!(plan.amount, None);
        assert_eq!(plan.currency, None);
        assert_eq!(plan.name, Some("Itemized"));
    }

    #[test]
    fn a_static_pricing_price_is_used_when_the_legacy_field_is_absent() {
        let variation = plan_variation(json!({
            "name": "Annual",
            "phases": [{
                "uid": "phase_only",
                "ordinal": 0,
                "cadence": "ANNUAL",
                "pricing": {"type": "STATIC", "price_money": {"amount": 5_000, "currency": "USD"}},
            }],
        }));

        let plan = subscription_plan(
            &subscription(json!({"id": "sub_1", "plan_variation_id": "variation_1"})),
            Some(&variation),
            at("2026-01-15"),
            None,
            day("2026-09-01"),
        );

        assert_eq!(plan.amount, Some(DOLLARS_50));
        assert_eq!(plan.interval, Some(("year", 1)));
    }

    #[test]
    fn phases_the_subscription_does_not_map_fall_back_to_the_catalog_ordering() {
        // Phases listed newest first, and a subscription whose phases point at
        // uids the plan variation doesn't have.
        let variation = plan_variation(json!({
            "name": "Pro",
            "phases": [
                {
                    "uid": "phase_standard",
                    "ordinal": 1,
                    "cadence": "MONTHLY",
                    "recurring_price_money": {"amount": 5_000, "currency": "USD"},
                },
                {
                    "uid": "phase_intro",
                    "ordinal": 0,
                    "cadence": "MONTHLY",
                    "periods": 3,
                    "recurring_price_money": {"amount": 1_000, "currency": "USD"},
                },
            ],
        }));
        let subscription = subscription(json!({
            "id": "sub_1",
            "start_date": "2026-01-15",
            "plan_variation_id": "variation_1",
            "phases": [{"uid": "sub_phase_1", "ordinal": 0, "plan_phase_uid": "phase_removed"}],
        }));
        let started_at = at("2026-01-15");

        let intro = subscription_plan(
            &subscription,
            Some(&variation),
            started_at,
            None,
            day("2026-03-01"),
        );
        assert_eq!(intro.amount, Some(DOLLARS_10));

        let standard = subscription_plan(
            &subscription,
            Some(&variation),
            started_at,
            None,
            day("2026-06-01"),
        );
        assert_eq!(standard.amount, Some(DOLLARS_50));
    }

    #[test]
    fn an_unresolved_plan_variation_leaves_the_plan_unknown() {
        let plan = subscription_plan(
            &active_subscription(),
            None,
            at("2026-01-15"),
            None,
            day("2026-06-01"),
        );

        assert_eq!(plan.interval, None);
        assert_eq!(plan.amount, None);
        assert_eq!(plan.currency, None);
        assert_eq!(plan.name, None);
    }

    #[test]
    fn advancing_a_month_end_date_clamps_to_the_shorter_month() {
        assert_eq!(
            add_interval(day("2026-01-31"), "month", 1),
            Some(day("2026-02-28"))
        );
        assert_eq!(
            add_interval(day("2028-01-31"), "month", 1),
            Some(day("2028-02-29"))
        );
        assert_eq!(
            add_interval(day("2026-01-31"), "month", 12),
            Some(day("2027-01-31"))
        );
        assert_eq!(
            add_interval(day("2026-12-15"), "month", 1),
            Some(day("2027-01-15"))
        );
        assert_eq!(
            add_interval(day("2026-01-01"), "year", 2),
            Some(day("2028-01-01"))
        );
        assert_eq!(
            add_interval(day("2026-01-01"), "week", 2),
            Some(day("2026-01-15"))
        );
        assert_eq!(
            add_interval(day("2026-01-01"), "day", 90),
            Some(day("2026-04-01"))
        );
    }
}
