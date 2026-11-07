//! Stripe sync: reads subscriptions, charges, invoices, checkout sessions,
//! refunds and disputes from a connected Stripe account into the project database.

mod checkout;
mod plan;
mod records;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use log::info;

use crate::db::ProjectCache;
use crate::server_context::ServerContext;

use super::attribution;
use super::schedule::{
    self, PaymentIntegration, SyncOutcome, decrypt_secret, load_due_payment_integrations,
};
use super::store::{ProviderObjectRef, store_provider_object};
use super::sync::{SyncCounts, SyncError, SyncKind, SyncResult};

use checkout::enrich_from_checkout_session;
use plan::{
    CouponCache, PaymentIntentMetadataCache, cache_invoice_payment_intents,
    cache_missing_payment_intents, cache_page_coupons,
};
use records::{
    charge_payment_id, invoice_payment_id, store_charge_payment, store_dispute_payment,
    store_invoice_payment, store_refund_payment, store_subscription,
};

const PROVIDER: &str = "stripe";
// Stripe’s API supports a maximum page limit of 100 objects per API request (default page size is 10)
const STRIPE_PAGE_LIMIT: i64 = 100;

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
    let secret_key = decrypt_secret(
        integration.encrypted_secret.as_deref(),
        SyncError::MissingStripeSecret,
        || SyncError::InvalidStoredSecret,
    )?;
    let client = build_client(&secret_key)?;
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
    sync_charges(
        &server_context,
        &project,
        &client,
        &external_account_id,
        since,
        &mut counts,
    )
    .await?;
    sync_invoices(
        &server_context,
        &project,
        &client,
        &external_account_id,
        since,
        &mut counts,
    )
    .await?;
    sync_checkout_sessions(&server_context, &project, &client, since, &mut counts).await?;
    sync_refunds(
        &server_context,
        &project,
        &client,
        &external_account_id,
        since,
        &mut counts,
    )
    .await?;
    sync_disputes(
        &server_context,
        &project,
        &client,
        &external_account_id,
        since,
        &mut counts,
    )
    .await?;
    if integration.sync_kind == SyncKind::Full {
        attribution::attribute_full_sync(server_context.clone(), &project, "stripe").await?;
    }

    info!(
        "synced stripe integration {} ({}): inserted={}, updated={}, deleted={}",
        integration.id,
        integration.sync_kind.as_str(),
        counts.records_inserted,
        counts.records_updated,
        counts.records_deleted
    );
    Ok(counts)
}

fn build_client(secret_key: &str) -> SyncResult<stripe::Client> {
    // async-stripe brings its own HTTP client, which has no timeout by default.
    Ok(stripe::ClientBuilder::new(secret_key.to_string())
        .timeout(crate::server_context::OUTBOUND_HTTP_TIMEOUT)
        .build()?)
}

async fn sync_subscriptions(
    server_context: &ServerContext,
    project: &Arc<ProjectCache>,
    client: &stripe::Client,
    external_account_id: &str,
    since: Option<i64>,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    let mut starting_after = None;
    let mut coupons = CouponCache::default();
    loop {
        let mut request = stripe_billing::subscription::ListSubscription::new()
            .status(stripe_billing::subscription::ListSubscriptionStatus::All)
            // Discounts are needed to price a subscription at what the customer
            // actually pays. Four levels is Stripe's expansion limit, so the
            // coupon behind an item discount has to be fetched separately.
            .expand(vec![
                "data.discounts.source.coupon".to_string(),
                "data.items.data.discounts".to_string(),
            ])
            .limit(STRIPE_PAGE_LIMIT);
        if let Some(since) = since {
            request = request.created(stripe_types::RangeQueryTs::gte(since));
        }
        if let Some(cursor) = starting_after.take() {
            request = request.starting_after(cursor);
        }
        let page = request.send(client).await?;
        let has_more = page.has_more;
        cache_page_coupons(client, &page.data, &mut coupons).await;
        {
            let conn = crate::db::open_project_database_write_for(server_context, project)?;
            for subscription in page.data {
                starting_after = Some(subscription.id.to_string());
                let stored = store_provider_object(
                    &conn,
                    PROVIDER,
                    external_account_id,
                    ProviderObjectRef {
                        object_type: "subscription",
                        object_id: subscription.id.as_ref(),
                        created_at: Some(subscription.created),
                        updated_at: None,
                    },
                    &subscription,
                    counts,
                )?;
                if stored.changed {
                    let subscription_id =
                        store_subscription(&conn, stored.id, &subscription, &coupons, counts)?;
                    attribution::attribute_subscription_by_id(&conn, subscription_id)?;
                }
            }
        }
        if !has_more {
            break;
        }
    }
    Ok(())
}

async fn sync_charges(
    server_context: &ServerContext,
    project: &Arc<ProjectCache>,
    client: &stripe::Client,
    external_account_id: &str,
    since: Option<i64>,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    let mut starting_after = None;
    loop {
        let mut request = stripe_core::charge::ListCharge::new()
            .expand(vec![
                "data.balance_transaction".to_string(),
                "data.payment_intent".to_string(),
            ])
            .limit(STRIPE_PAGE_LIMIT);
        if let Some(since) = since {
            request = request.created(stripe_types::RangeQueryTs::gte(since));
        }
        if let Some(cursor) = starting_after.take() {
            request = request.starting_after(cursor);
        }
        let page = request.send(client).await?;
        let has_more = page.has_more;
        {
            let conn = crate::db::open_project_database_write_for(server_context, project)?;
            for charge in page.data {
                starting_after = Some(charge.id.to_string());
                let stored = store_provider_object(
                    &conn,
                    PROVIDER,
                    external_account_id,
                    ProviderObjectRef {
                        object_type: "charge",
                        object_id: charge.id.as_ref(),
                        created_at: Some(charge.created),
                        updated_at: None,
                    },
                    &charge,
                    counts,
                )?;
                if stored.changed {
                    let provider_payment_id = charge_payment_id(&charge);
                    store_charge_payment(&conn, stored.id, &charge, counts)?;
                    attribution::attribute_payment_by_provider_id(
                        &conn,
                        PROVIDER,
                        &provider_payment_id,
                    )?;
                }
            }
        }
        if !has_more {
            break;
        }
    }
    Ok(())
}

async fn sync_invoices(
    server_context: &ServerContext,
    project: &Arc<ProjectCache>,
    client: &stripe::Client,
    external_account_id: &str,
    since: Option<i64>,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    let mut payment_intents = PaymentIntentMetadataCache::default();
    cache_invoice_payment_intents(client, since, &mut payment_intents).await?;

    let mut starting_after = None;
    loop {
        let mut request = stripe_billing::invoice::ListInvoice::new()
            // `payments` has to be expanded or Stripe omits the list, and the
            // charge and payment intent ids on it are what tie an invoice to
            // the charge it paid. Expanding those objects is five levels
            // (`data.payments.data.payment.payment_intent`), past Stripe's
            // limit of four, so their metadata is loaded separately.
            .expand(vec![
                "data.payments".to_string(),
                "data.subscription".to_string(),
            ])
            .limit(STRIPE_PAGE_LIMIT);
        if let Some(since) = since {
            request = request.created(stripe_types::RangeQueryTs::gte(since));
        }
        if let Some(cursor) = starting_after.take() {
            request = request.starting_after(cursor);
        }
        let page = request.send(client).await?;
        let has_more = page.has_more;
        cache_missing_payment_intents(client, &page.data, &mut payment_intents).await;
        {
            let conn = crate::db::open_project_database_write_for(server_context, project)?;
            for invoice in page.data {
                let Some(invoice_id) = invoice.id.as_ref().map(ToString::to_string) else {
                    continue;
                };
                starting_after = Some(invoice_id.clone());
                let stored = store_provider_object(
                    &conn,
                    PROVIDER,
                    external_account_id,
                    ProviderObjectRef {
                        object_type: "invoice",
                        object_id: &invoice_id,
                        created_at: Some(invoice.created),
                        updated_at: invoice.status_transitions.paid_at,
                    },
                    &invoice,
                    counts,
                )?;
                if stored.changed {
                    let provider_payment_id = invoice_payment_id(&invoice_id, &invoice);
                    store_invoice_payment(
                        &conn,
                        stored.id,
                        &invoice_id,
                        &invoice,
                        &payment_intents,
                        counts,
                    )?;
                    attribution::attribute_payment_by_provider_id(
                        &conn,
                        PROVIDER,
                        &provider_payment_id,
                    )?;
                }
            }
        }
        if !has_more {
            break;
        }
    }
    Ok(())
}

async fn sync_checkout_sessions(
    server_context: &ServerContext,
    project: &Arc<ProjectCache>,
    client: &stripe::Client,
    since: Option<i64>,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    let mut starting_after = None;
    loop {
        let mut request = stripe_checkout::checkout_session::ListCheckoutSession::new()
            .status(stripe_shared::CheckoutSessionStatus::Complete)
            .expand(vec![
                "data.invoice".to_string(),
                "data.payment_intent".to_string(),
                "data.subscription".to_string(),
            ])
            .limit(STRIPE_PAGE_LIMIT);
        if let Some(since) = since {
            request = request.created(stripe_types::RangeQueryTs::gte(since));
        }
        if let Some(cursor) = starting_after.take() {
            request = request.starting_after(cursor);
        }
        let page = request.send(client).await?;
        let has_more = page.has_more;
        {
            let conn = crate::db::open_project_database_write_for(server_context, project)?;
            for session in page.data {
                starting_after = Some(session.id.to_string());
                enrich_from_checkout_session(&conn, &session, counts)?;
            }
        }
        if !has_more {
            break;
        }
    }
    Ok(())
}

async fn sync_refunds(
    server_context: &ServerContext,
    project: &Arc<ProjectCache>,
    client: &stripe::Client,
    external_account_id: &str,
    since: Option<i64>,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    let mut starting_after = None;
    loop {
        let mut request = stripe_core::refund::ListRefund::new()
            .expand(vec![
                "data.balance_transaction".to_string(),
                "data.payment_intent".to_string(),
            ])
            .limit(STRIPE_PAGE_LIMIT);
        if let Some(since) = since {
            request = request.created(stripe_types::RangeQueryTs::gte(since));
        }
        if let Some(cursor) = starting_after.take() {
            request = request.starting_after(cursor);
        }
        let page = request.send(client).await?;
        let has_more = page.has_more;
        {
            let conn = crate::db::open_project_database_write_for(server_context, project)?;
            for refund in page.data {
                starting_after = Some(refund.id.to_string());
                let stored = store_provider_object(
                    &conn,
                    PROVIDER,
                    external_account_id,
                    ProviderObjectRef {
                        object_type: "refund",
                        object_id: refund.id.as_ref(),
                        created_at: Some(refund.created),
                        updated_at: None,
                    },
                    &refund,
                    counts,
                )?;
                if stored.changed {
                    let provider_payment_id = refund.id.to_string();
                    store_refund_payment(&conn, stored.id, &refund, counts)?;
                    attribution::attribute_payment_by_provider_id(
                        &conn,
                        PROVIDER,
                        &provider_payment_id,
                    )?;
                }
            }
        }
        if !has_more {
            break;
        }
    }
    Ok(())
}

async fn sync_disputes(
    server_context: &ServerContext,
    project: &Arc<ProjectCache>,
    client: &stripe::Client,
    external_account_id: &str,
    since: Option<i64>,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    let mut starting_after = None;
    loop {
        let mut request = stripe_core::dispute::ListDispute::new()
            .expand(vec!["data.payment_intent".to_string()])
            .limit(STRIPE_PAGE_LIMIT);
        if let Some(since) = since {
            request = request.created(stripe_types::RangeQueryTs::gte(since));
        }
        if let Some(cursor) = starting_after.take() {
            request = request.starting_after(cursor);
        }
        let page = request.send(client).await?;
        let has_more = page.has_more;
        {
            let conn = crate::db::open_project_database_write_for(server_context, project)?;
            for dispute in page.data {
                starting_after = Some(dispute.id.to_string());
                let stored = store_provider_object(
                    &conn,
                    PROVIDER,
                    external_account_id,
                    ProviderObjectRef {
                        object_type: "dispute",
                        object_id: dispute.id.as_ref(),
                        created_at: Some(dispute.created),
                        updated_at: None,
                    },
                    &dispute,
                    counts,
                )?;
                if stored.changed {
                    let provider_payment_id = dispute.id.to_string();
                    store_dispute_payment(&conn, stored.id, &dispute, counts)?;
                    attribution::attribute_payment_by_provider_id(
                        &conn,
                        PROVIDER,
                        &provider_payment_id,
                    )?;
                }
            }
        }
        if !has_more {
            break;
        }
    }
    Ok(())
}
