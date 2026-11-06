//! `subscription_history`: a new version each time a subscription's billed
//! state changes, so MRR can be read as of any instant.

use std::fmt::Write as _;

use rusqlite::{OptionalExtension, params};
use xxhash_rust::xxh3::xxh3_64;

use crate::db::ProjectConnection;
use crate::integration::sync::SyncResult;

use super::subscription::SubscriptionState;

/// The currently open (`effective_to IS NULL`) history version.
#[derive(Debug)]
pub(super) struct OpenVersion {
    pub(super) id: i64,
    pub(super) version: i64,
    pub(super) effective_from: i64,
    pub(super) fingerprint: String,
    pub(super) mrr_amount: Option<i64>,
    pub(super) quantity: Option<i64>,
    pub(super) interval_unit: String,
    pub(super) interval_count: i64,
    pub(super) canceled_at: Option<i64>,
    pub(super) cancel_at: Option<i64>,
    pub(super) ended_at: Option<i64>,
}

impl OpenVersion {
    /// Whether a cancellation was in force, whether or not it had taken effect.
    fn is_canceled(&self) -> bool {
        self.canceled_at.is_some() || self.cancel_at.is_some() || self.ended_at.is_some()
    }
}

/// Appends a history version when the subscription's tracked state changed,
/// closing the previous version's range at the new one's start.
///
/// Period boundaries and `next_billing_at` are deliberately not tracked: they
/// advance on every renewal, which `payments` already records, and including
/// them would add a version per billing cycle for no added information.
pub(super) fn append_subscription_history(
    conn: &ProjectConnection,
    subscription_id: i64,
    state: &SubscriptionState,
    observed_at: i64,
) -> SyncResult<bool> {
    let fingerprint = version_fingerprint(state);
    let open = open_version(conn, subscription_id)?;
    if open
        .as_ref()
        .is_some_and(|open| open.fingerprint == fingerprint)
    {
        return Ok(false);
    }

    let effective_from = version_effective_from(open.as_ref(), state, observed_at);
    let change_kind = version_change_kind(open.as_ref(), state);
    let version = open.as_ref().map_or(1, |open| open.version + 1);
    let prev_mrr_amount = open.as_ref().and_then(|open| open.mrr_amount);

    if let Some(open) = &open {
        conn.execute(
            "UPDATE subscription_history SET effective_to = ?2 WHERE id = ?1",
            params![open.id, effective_from],
        )?;
    }

    conn.execute(
        "INSERT INTO subscription_history (
            subscription_id, version, effective_from, effective_to, observed_at,
            change_kind, status, provider_price_id, plan_name, quantity,
            currency, plan_amount, mrr_amount, prev_mrr_amount,
            interval_unit, interval_count, canceled_at, cancel_at, ended_at,
            provider_object_id, fingerprint
         ) VALUES (
            ?1, ?2, ?3, NULL, ?4,
            ?5, ?6, ?7, ?8, ?9,
            ?10, ?11, ?12, ?13,
            ?14, ?15, ?16, ?17, ?18,
            ?19, ?20
         )",
        params![
            subscription_id,
            version,
            effective_from,
            observed_at,
            change_kind,
            state.status,
            state.provider_price_id,
            state.plan_name,
            state.quantity,
            state.currency,
            state.plan_amount,
            state.mrr_amount(),
            prev_mrr_amount,
            state.interval_unit,
            state.interval_count,
            state.canceled_at,
            state.cancel_at,
            state.ended_at,
            state.provider_object_id,
            fingerprint,
        ],
    )?;
    Ok(true)
}

pub(super) fn open_version(
    conn: &ProjectConnection,
    subscription_id: i64,
) -> SyncResult<Option<OpenVersion>> {
    Ok(conn
        .query_row(
            "SELECT id, version, effective_from, fingerprint, mrr_amount, quantity,
                    interval_unit, interval_count, canceled_at, cancel_at, ended_at
             FROM subscription_history
             WHERE subscription_id = ?1 AND effective_to IS NULL
             ORDER BY version DESC
             LIMIT 1",
            params![subscription_id],
            |row| {
                Ok(OpenVersion {
                    id: row.get(0)?,
                    version: row.get(1)?,
                    effective_from: row.get(2)?,
                    fingerprint: row.get(3)?,
                    mrr_amount: row.get(4)?,
                    quantity: row.get(5)?,
                    interval_unit: row.get(6)?,
                    interval_count: row.get(7)?,
                    canceled_at: row.get(8)?,
                    cancel_at: row.get(9)?,
                    ended_at: row.get(10)?,
                })
            },
        )
        .optional()?)
}

/// Hashes the fields whose change warrants a new version.
pub(super) fn version_fingerprint(state: &SubscriptionState) -> String {
    let mut canonical = String::new();
    let _ = write!(
        canonical,
        "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        state.status.as_deref().unwrap_or_default(),
        state.provider_price_id.as_deref().unwrap_or_default(),
        state.plan_name.as_deref().unwrap_or_default(),
        OptionalField(state.quantity),
        state.currency.as_deref().unwrap_or_default(),
        OptionalField(state.plan_amount),
        OptionalField(state.mrr_amount()),
        state.interval_unit,
        state.interval_count,
        OptionalField(state.canceled_at),
        OptionalField(state.cancel_at),
        OptionalField(state.ended_at),
    );
    format!("{:016x}", xxh3_64(canonical.as_bytes()))
}

pub(super) struct OptionalField(Option<i64>);

impl std::fmt::Display for OptionalField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            Some(value) => write!(f, "{value}"),
            None => Ok(()),
        }
    }
}

/// Dates a new version, preferring a provider timestamp that explains the change
/// over the time we happened to observe it.
///
/// Candidates at or before the open version's start were already accounted for,
/// and candidates in the future have not happened yet. `cancel_at` is not among
/// them: a cancellation scheduled for period end changes the subscription when
/// it is requested, not when it takes effect. Ranges are kept strictly
/// increasing so that an as-of lookup matches exactly one version.
pub(super) fn version_effective_from(
    open: Option<&OpenVersion>,
    state: &SubscriptionState,
    observed_at: i64,
) -> i64 {
    let Some(open) = open else {
        return state.started_at.unwrap_or(observed_at);
    };
    [
        state.canceled_at,
        state.ended_at,
        state.current_period_start,
    ]
    .into_iter()
    .flatten()
    .filter(|at| *at > open.effective_from && *at <= observed_at)
    .max()
    .unwrap_or(observed_at)
    .max(open.effective_from + 1)
}

pub(super) fn version_change_kind(
    open: Option<&OpenVersion>,
    state: &SubscriptionState,
) -> &'static str {
    let Some(open) = open else {
        return "created";
    };
    if state.ended_at.is_some() && open.ended_at.is_none() {
        return "ended";
    }
    if state.is_canceled() && !open.is_canceled() {
        return "canceled";
    }
    if !state.is_canceled() && open.is_canceled() {
        return "resumed";
    }
    if let (Some(previous), Some(current)) = (open.mrr_amount, state.mrr_amount()) {
        if current > previous {
            return "upgrade";
        }
        if current < previous {
            return "downgrade";
        }
    }
    if open.interval_unit != state.interval_unit || open.interval_count != state.interval_count {
        return "interval_change";
    }
    if open.quantity != state.quantity {
        return "quantity_change";
    }
    "status_change"
}
