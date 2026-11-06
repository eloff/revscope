//! Shared persistence for provider objects, subscriptions, and payments.
//!
//! Every payment integration writes the same three project-database tables, so
//! the SQL and the derived-value rules (MRR normalization, minor-unit scaling,
//! refund sign conventions, subscription history) live here rather than being
//! copied per provider. Provider modules are left with the job of mapping their
//! API types onto [`SubscriptionUpsert`] and [`PaymentUpsert`].

mod history;
mod payment;
mod subscription;
#[cfg(test)]
mod tests;

use rusqlite::{OptionalExtension, params};
use serde::Serialize;

use crate::db::ProjectConnection;
use crate::util::{non_empty, unix_seconds};

use super::sync::{SyncCounts, SyncError, SyncResult};

pub(super) use payment::{
    PaymentInterval, PaymentStatus, PaymentType, PaymentUpsert, upsert_payment,
};
pub(super) use subscription::{
    PlanInterval, SubscriptionPlan, SubscriptionUpsert, subscription_row_id, upsert_subscription,
};

/// A `provider_objects` row, along with whether this sync changed its payload.
/// When `changed` is false the caller can skip the downstream subscription or
/// payment upsert, since the source data is identical to the last sync.
#[derive(Clone, Copy, Debug)]
pub(super) struct ProviderObject {
    pub(super) id: i64,
    pub(super) changed: bool,
}

/// Identifies a provider payload: what kind of object it is, the provider's id
/// for it, and the provider's own timestamps.
#[derive(Clone, Copy, Debug)]
pub(super) struct ProviderObjectRef<'a> {
    pub(super) object_type: &'a str,
    pub(super) object_id: &'a str,
    pub(super) created_at: Option<i64>,
    pub(super) updated_at: Option<i64>,
}

/// Stores the raw provider payload, returning its row id and whether it changed.
pub(super) fn store_provider_object<T: Serialize>(
    conn: &ProjectConnection,
    provider: &str,
    external_account_id: &str,
    object_ref: ProviderObjectRef<'_>,
    object: &T,
    counts: &mut SyncCounts,
) -> SyncResult<ProviderObject> {
    let ProviderObjectRef {
        object_type,
        object_id,
        created_at,
        updated_at,
    } = object_ref;
    let existing_id = provider_object_id(conn, provider, object_type, object_id)?;
    let payload = serde_json::to_string(object)?;
    // The upsert's WHERE clause suppresses the write when the payload is
    // unchanged, so no row comes back from RETURNING in that case.
    let stored_id = conn
        .query_row(
            "INSERT INTO provider_objects (
                provider, external_account_id, object_type, provider_object_id,
                object_created_at, object_updated_at, received_at, payload
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8
             )
             ON CONFLICT(provider, object_type, provider_object_id) DO UPDATE SET
                external_account_id = excluded.external_account_id,
                object_created_at = excluded.object_created_at,
                object_updated_at = coalesce(excluded.object_updated_at, object_updated_at),
                received_at = excluded.received_at,
                payload = excluded.payload
             WHERE payload IS NOT excluded.payload
             RETURNING id",
            params![
                provider,
                external_account_id,
                object_type,
                object_id,
                created_at,
                updated_at,
                unix_seconds(),
                payload,
            ],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;

    match (existing_id, stored_id) {
        (Some(id), Some(_)) => {
            counts.records_updated += 1;
            Ok(ProviderObject { id, changed: true })
        }
        (Some(id), None) => Ok(ProviderObject { id, changed: false }),
        (None, Some(id)) => {
            counts.records_inserted += 1;
            Ok(ProviderObject { id, changed: true })
        }
        (None, None) => Err(SyncError::MissingUpsertedRow("provider_objects")),
    }
}

fn provider_object_id(
    conn: &ProjectConnection,
    provider: &str,
    object_type: &str,
    object_id: &str,
) -> SyncResult<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT id
             FROM provider_objects
             WHERE provider = ?1 AND object_type = ?2 AND provider_object_id = ?3",
            params![provider, object_type, object_id],
            |row| row.get(0),
        )
        .optional()?)
}

/// What one payload says about a field, separating a field the payload doesn't
/// cover from one it covers and reports as empty.
///
/// `Option` can't hold both meanings, and which of them applies decides whether
/// a cancellation survives a sync. A payload describing a whole subscription
/// reports no `canceled_at` once the customer resumes, so the stored value has
/// to go; a payload carrying only part of one, such as a subscription embedded
/// in an order, says nothing about cancellation and must leave it alone.
#[derive(Clone, Copy, Debug)]
pub(super) enum Reported<T> {
    /// Outside what the payload covers, so the stored value stands.
    Unknown,
    /// Covered by the payload, which reports no value, so a stored value is
    /// cleared.
    Cleared,
    Value(T),
}

impl<T> Reported<T> {
    /// A field read off a payload that describes the whole subscription, where
    /// an absent value means the subscription no longer has one.
    pub(super) fn or_cleared(value: Option<T>) -> Self {
        match value {
            Some(value) => Self::Value(value),
            None => Self::Cleared,
        }
    }

    /// What the field holds once this payload is applied to the stored value.
    fn applied_to(self, stored: Option<T>) -> Option<T> {
        match self {
            Self::Unknown => stored,
            Self::Cleared => None,
            Self::Value(value) => Some(value),
        }
    }
}

/// The billing interval to denormalize onto a payment of `subscription_id`.
pub(super) fn subscription_interval(
    conn: &ProjectConnection,
    subscription_id: i64,
) -> SyncResult<Option<PaymentInterval>> {
    Ok(conn
        .query_row(
            "SELECT interval_unit, interval_count
             FROM subscriptions
             WHERE id = ?1",
            params![subscription_id],
            |row| {
                Ok(PaymentInterval {
                    unit: row.get(0)?,
                    count: row.get(1)?,
                })
            },
        )
        .optional()?)
}

/// Converts a provider amount in a currency's minor unit to thousandths of the
/// currency unit, which is how `payments.amount` and `subscriptions.plan_amount`
/// are stored.
pub(super) fn minor_units_to_thousandths(amount: i64, currency: &str) -> i64 {
    let multiplier = match minor_unit_decimals(currency) {
        0 => 1000,
        3 => 1,
        _ => 10,
    };
    amount.saturating_mul(multiplier)
}

fn minor_unit_decimals(currency: &str) -> u8 {
    match currency.to_ascii_uppercase().as_str() {
        "BIF" | "CLP" | "DJF" | "GNF" | "JPY" | "KMF" | "KRW" | "MGA" | "PYG" | "RWF" | "UGX"
        | "VND" | "VUV" | "XAF" | "XOF" | "XPF" => 0,
        "BHD" | "IQD" | "JOD" | "KWD" | "LYD" | "OMR" | "TND" => 3,
        _ => 2,
    }
}

/// Normalizes a currency code to the uppercase form the schema requires,
/// falling back to the ISO "no currency" code when it isn't three letters.
pub(super) fn currency_code(currency: &str) -> String {
    let code = currency.trim().to_ascii_uppercase();
    if code.len() == 3 && code.chars().all(|ch| ch.is_ascii_alphabetic()) {
        code
    } else {
        "XXX".to_string()
    }
}

pub(super) fn count_upsert(existing: bool, counts: &mut SyncCounts) {
    if existing {
        counts.records_updated += 1;
    } else {
        counts.records_inserted += 1;
    }
}

/// A provider string as [`SubscriptionState`](subscription::SubscriptionState) holds it, absent when empty.
fn owned(value: Option<&str>) -> Option<String> {
    non_empty(value).map(str::to_string)
}
