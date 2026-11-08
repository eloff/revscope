//! Copying attribution metadata from Stripe Checkout sessions onto the
//! payments and subscriptions they created.

use rusqlite::{OptionalExtension, params};
use serde_json::{Map, Value};

use crate::db::ProjectConnection;
use crate::integration::attribution;
use crate::integration::sync::{SyncCounts, SyncResult};
use crate::untrusted_keys::UntrustedKeyMap;

use super::records::{expandable_id, expandable_invoice_id};

pub(super) struct CheckoutPaymentRow {
    pub(super) id: i64,
    pub(super) subscription_id: Option<i64>,
    pub(super) provider_payment_id: String,
    pub(super) provider_checkout_id: Option<String>,
    pub(super) metadata: Option<String>,
}

pub(super) struct CheckoutSubscriptionRow {
    pub(super) id: i64,
    pub(super) provider_checkout_id: Option<String>,
    pub(super) metadata: Option<String>,
}

pub(super) fn enrich_from_checkout_session(
    conn: &ProjectConnection,
    session: &stripe_shared::CheckoutSession,
    counts: &mut SyncCounts,
) -> SyncResult<()> {
    let checkout_id = session.id.to_string();
    let checkout_metadata = checkout_enrichment_metadata(
        session.metadata.as_ref(),
        session.client_reference_id.as_deref(),
    );
    let checkout_metadata = checkout_metadata.as_ref();
    match &session.mode {
        stripe_shared::CheckoutSessionMode::Payment => {
            if let Some(payment_intent_id) = session.payment_intent.as_ref().map(expandable_id)
                && update_payment_checkout_by_provider_payment_id(
                    conn,
                    &payment_intent_id,
                    &checkout_id,
                    checkout_metadata,
                    counts,
                )?
            {
                attribution::attribute_payment_by_provider_id(conn, "stripe", &payment_intent_id)?;
            }
        }
        stripe_shared::CheckoutSessionMode::Subscription => {
            if let Some(subscription_id) = session.subscription.as_ref().map(expandable_id)
                && let Some(row_id) = update_subscription_checkout_by_provider_subscription_id(
                    conn,
                    &subscription_id,
                    &checkout_id,
                    checkout_metadata,
                    counts,
                )?
            {
                attribution::propagate_linked_subscription_payment_fields(conn, row_id)?;
                attribution::attribute_subscription_by_id(conn, row_id)?;
            }
        }
        _ => {}
    }

    if let Some(invoice_id) = session.invoice.as_ref().and_then(expandable_invoice_id) {
        let updated_payments = update_payment_checkouts_by_invoice_id(
            conn,
            &invoice_id,
            &checkout_id,
            checkout_metadata,
            counts,
        )?;
        for payment in updated_payments {
            if let Some(subscription_id) = payment.subscription_id {
                attribution::propagate_linked_subscription_payment_fields(conn, subscription_id)?;
                attribution::attribute_subscription_by_id(conn, subscription_id)?;
            }
            attribution::attribute_payment_by_provider_id(
                conn,
                "stripe",
                &payment.provider_payment_id,
            )?;
        }
    }

    Ok(())
}

/// Builds the metadata map written onto payments/subscriptions from a checkout
/// session. Includes Stripe session metadata plus `client_reference_id` when
/// set (e.g. from a Payment Link), so attribution can match it like other
/// metadata values.
pub(super) fn checkout_enrichment_metadata(
    session_metadata: Option<&UntrustedKeyMap<String, String>>,
    client_reference_id: Option<&str>,
) -> Option<UntrustedKeyMap<String, String>> {
    let mut metadata = session_metadata.cloned().unwrap_or_default();
    if let Some(client_reference_id) = client_reference_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        metadata.insert(
            "client_reference_id".to_string(),
            client_reference_id.to_string(),
        );
    }
    if metadata.is_empty() {
        None
    } else {
        Some(metadata)
    }
}

pub(super) fn update_payment_checkout_by_provider_payment_id(
    conn: &ProjectConnection,
    provider_payment_id: &str,
    checkout_id: &str,
    checkout_metadata: Option<&UntrustedKeyMap<String, String>>,
    counts: &mut SyncCounts,
) -> SyncResult<bool> {
    let Some(row) = conn
        .query_row(
            "SELECT id, subscription_id, provider_payment_id, provider_checkout_id, metadata
             FROM payments
             WHERE provider = 'stripe' AND provider_payment_id = ?1",
            params![provider_payment_id],
            checkout_payment_row,
        )
        .optional()?
    else {
        return Ok(false);
    };
    update_payment_checkout(conn, &row, checkout_id, checkout_metadata, counts)
}

pub(super) fn update_payment_checkouts_by_invoice_id(
    conn: &ProjectConnection,
    invoice_id: &str,
    checkout_id: &str,
    checkout_metadata: Option<&UntrustedKeyMap<String, String>>,
    counts: &mut SyncCounts,
) -> SyncResult<Vec<CheckoutPaymentRow>> {
    let rows = conn.with_query(
        "SELECT id, subscription_id, provider_payment_id, provider_checkout_id, metadata
         FROM payments
         WHERE provider = 'stripe' AND provider_invoice_id = ?1",
        params![invoice_id],
        |mut rows| {
            let mut payments = Vec::new();
            while let Some(row) = rows.next()? {
                payments.push(checkout_payment_row(row)?);
            }
            Ok(payments)
        },
    )?;

    let mut updated = Vec::new();
    for row in rows {
        if update_payment_checkout(conn, &row, checkout_id, checkout_metadata, counts)? {
            updated.push(row);
        }
    }
    Ok(updated)
}

pub(super) fn update_payment_checkout(
    conn: &ProjectConnection,
    row: &CheckoutPaymentRow,
    checkout_id: &str,
    checkout_metadata: Option<&UntrustedKeyMap<String, String>>,
    counts: &mut SyncCounts,
) -> SyncResult<bool> {
    let metadata = merge_checkout_metadata(row.metadata.as_deref(), checkout_metadata)?;
    if row.provider_checkout_id.as_deref() == Some(checkout_id) && row.metadata == metadata {
        return Ok(false);
    }
    conn.execute(
        "UPDATE payments
         SET provider_checkout_id = ?2,
             metadata = ?3
         WHERE id = ?1",
        params![row.id, checkout_id, metadata],
    )?;
    counts.records_updated += 1;
    Ok(true)
}

pub(super) fn update_subscription_checkout_by_provider_subscription_id(
    conn: &ProjectConnection,
    provider_subscription_id: &str,
    checkout_id: &str,
    checkout_metadata: Option<&UntrustedKeyMap<String, String>>,
    counts: &mut SyncCounts,
) -> SyncResult<Option<i64>> {
    let Some(row) = conn
        .query_row(
            "SELECT id, provider_checkout_id, metadata
             FROM subscriptions
             WHERE provider = 'stripe' AND provider_subscription_id = ?1",
            params![provider_subscription_id],
            |row| {
                Ok(CheckoutSubscriptionRow {
                    id: row.get(0)?,
                    provider_checkout_id: row.get(1)?,
                    metadata: row.get(2)?,
                })
            },
        )
        .optional()?
    else {
        return Ok(None);
    };
    let metadata = merge_checkout_metadata(row.metadata.as_deref(), checkout_metadata)?;
    if row.provider_checkout_id.as_deref() == Some(checkout_id) && row.metadata == metadata {
        return Ok(None);
    }
    conn.execute(
        "UPDATE subscriptions
         SET provider_checkout_id = ?2,
             metadata = ?3
         WHERE id = ?1",
        params![row.id, checkout_id, metadata],
    )?;
    counts.records_updated += 1;
    Ok(Some(row.id))
}

pub(super) fn checkout_payment_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<CheckoutPaymentRow> {
    Ok(CheckoutPaymentRow {
        id: row.get(0)?,
        subscription_id: row.get(1)?,
        provider_payment_id: row.get(2)?,
        provider_checkout_id: row.get(3)?,
        metadata: row.get(4)?,
    })
}

pub(super) fn merge_checkout_metadata(
    existing: Option<&str>,
    checkout_metadata: Option<&UntrustedKeyMap<String, String>>,
) -> SyncResult<Option<String>> {
    let Some(checkout_metadata) = checkout_metadata.filter(|metadata| !metadata.is_empty()) else {
        return Ok(existing.map(ToOwned::to_owned));
    };
    let mut merged = existing
        .map(serde_json::from_str::<Value>)
        .transpose()?
        .and_then(|value| match value {
            Value::Object(object) => Some(object),
            _ => None,
        })
        .unwrap_or_else(Map::new);
    for (key, value) in checkout_metadata {
        merged.insert(key.clone(), Value::String(value.clone()));
    }
    if merged.is_empty() {
        Ok(None)
    } else {
        Ok(Some(serde_json::to_string(&Value::Object(merged))?))
    }
}
