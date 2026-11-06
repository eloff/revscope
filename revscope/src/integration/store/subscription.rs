//! Upserting subscriptions, and the plan and lifecycle state they carry.

use log::warn;
use rusqlite::{OptionalExtension, params};

use crate::db::ProjectConnection;
use crate::integration::attribution;
use crate::integration::sync::{SyncCounts, SyncResult};
use crate::metrics::monthly_amount;
use crate::subscription_status;
use crate::util::{non_empty, unix_seconds};

use super::history::append_subscription_history;
use super::{Reported, count_upsert, owned};

/// A billing period, `count` `unit`s long, charged for once per period.
#[derive(Clone, Copy, Debug)]
pub(in crate::integration) struct PlanInterval<'a> {
    unit: &'a str,
    count: i64,
}

impl<'a> PlanInterval<'a> {
    pub(in crate::integration) fn new(unit: &'a str, count: i64) -> Self {
        Self {
            unit,
            count: count.max(1),
        }
    }
}

/// The recurring plan a subscription is on, as one payload reports it.
/// `amount` is charged once per billing period.
#[derive(Clone, Debug)]
pub(in crate::integration) struct SubscriptionPlan<'a> {
    /// The billing period, or `None` when the payload doesn't pin it down.
    /// Unknown is not the same as monthly: assuming monthly for a plan an
    /// earlier sync recorded as annual would multiply its MRR twelvefold.
    pub(in crate::integration) interval: Option<PlanInterval<'a>>,
    pub(in crate::integration) provider_price_id: Option<&'a str>,
    pub(in crate::integration) plan_name: Option<&'a str>,
    pub(in crate::integration) quantity: Option<i64>,
    pub(in crate::integration) currency: Option<&'a str>,
    /// Integer thousandths of the currency unit, per billing period.
    pub(in crate::integration) amount: Option<i64>,
}

impl SubscriptionPlan<'_> {
    /// A plan nothing is known about, which leaves whatever an earlier sync
    /// recorded in place.
    fn unknown() -> Self {
        Self {
            interval: None,
            provider_price_id: None,
            plan_name: None,
            quantity: None,
            currency: None,
            amount: None,
        }
    }
}

/// A subscription as one provider payload reports it.
///
/// The fields describe that payload rather than the subscription: a payload is
/// merged onto the stored state (see [`SubscriptionState::merged`]) before
/// anything is written, so one covering only part of a subscription can't erase
/// what a fuller sync recorded. [`Option`] fields are unknown when absent; the
/// lifecycle timestamps are [`Reported`] because a payload that covers them has
/// to be able to clear them.
#[derive(Clone, Debug)]
pub(in crate::integration) struct SubscriptionUpsert<'a> {
    pub(in crate::integration) provider: &'a str,
    pub(in crate::integration) provider_subscription_id: &'a str,
    pub(in crate::integration) provider_object_id: Option<i64>,
    pub(in crate::integration) provider_order_id: Option<&'a str>,
    pub(in crate::integration) provider_checkout_id: Option<&'a str>,
    pub(in crate::integration) provider_customer_id: Option<&'a str>,
    pub(in crate::integration) customer_email: Option<&'a str>,
    pub(in crate::integration) order_email: Option<&'a str>,
    pub(in crate::integration) status: Option<&'a str>,
    pub(in crate::integration) plan: SubscriptionPlan<'a>,
    pub(in crate::integration) current_period_start: Option<i64>,
    pub(in crate::integration) current_period_end: Option<i64>,
    pub(in crate::integration) next_billing_at: Option<i64>,
    pub(in crate::integration) started_at: Option<i64>,
    pub(in crate::integration) ended_at: Reported<i64>,
    /// When the cancellation was *requested*, which for a cancellation
    /// scheduled at period end is not when the subscription stops.
    pub(in crate::integration) canceled_at: Reported<i64>,
    /// When a requested cancellation takes effect, for a subscription still
    /// running out the period it has paid for.
    pub(in crate::integration) cancel_at: Reported<i64>,
    pub(in crate::integration) metadata: Option<&'a str>,
    /// When this sync observed the state, which can lag the change itself.
    pub(in crate::integration) observed_at: i64,
}

impl<'a> SubscriptionUpsert<'a> {
    pub(in crate::integration) fn new(
        provider: &'a str,
        provider_subscription_id: &'a str,
    ) -> Self {
        Self {
            provider,
            provider_subscription_id,
            provider_object_id: None,
            provider_order_id: None,
            provider_checkout_id: None,
            provider_customer_id: None,
            customer_email: None,
            order_email: None,
            status: None,
            plan: SubscriptionPlan::unknown(),
            current_period_start: None,
            current_period_end: None,
            next_billing_at: None,
            started_at: None,
            ended_at: Reported::Unknown,
            canceled_at: Reported::Unknown,
            cancel_at: Reported::Unknown,
            metadata: None,
            observed_at: unix_seconds(),
        }
    }

    /// `next_billing_at` defaults to the end of the current period, which is
    /// what most providers mean by it.
    fn next_billing_at(&self) -> Option<i64> {
        self.next_billing_at.or(self.current_period_end)
    }

    /// The provider's own status mapped onto the canonical set. Normalizing
    /// here rather than per provider is what lets the metrics queries reason
    /// about lifecycle state without a provider-specific case for each one.
    fn normalized_status(&self) -> Option<&'static str> {
        non_empty(self.status).map(subscription_status::normalize)
    }
}

/// A subscription's state as the anchor row and its history versions record it:
/// one payload merged onto what was already stored.
///
/// Every write goes through this. Resolving absent fields in the anchor's SQL
/// instead is what once let a sparse payload keep $100 a year on the anchor
/// while appending a history version that recorded no price at all.
#[derive(Clone, Debug)]
pub(super) struct SubscriptionState {
    pub(super) provider_object_id: Option<i64>,
    pub(super) provider_order_id: Option<String>,
    pub(super) provider_checkout_id: Option<String>,
    pub(super) provider_customer_id: Option<String>,
    pub(super) customer_email: Option<String>,
    pub(super) order_email: Option<String>,
    pub(super) status: Option<String>,
    pub(super) currency: Option<String>,
    pub(super) provider_price_id: Option<String>,
    pub(super) plan_name: Option<String>,
    pub(super) quantity: Option<i64>,
    pub(super) plan_amount: Option<i64>,
    pub(super) interval_unit: String,
    pub(super) interval_count: i64,
    pub(super) current_period_start: Option<i64>,
    pub(super) current_period_end: Option<i64>,
    pub(super) next_billing_at: Option<i64>,
    pub(super) started_at: Option<i64>,
    pub(super) ended_at: Option<i64>,
    pub(super) canceled_at: Option<i64>,
    pub(super) cancel_at: Option<i64>,
    pub(super) metadata: Option<String>,
}

impl SubscriptionState {
    /// Where a subscription no sync has stored yet starts from.
    ///
    /// The interval falls back to monthly because the anchor requires one, so a
    /// first payload that can't pin the cadence down leaves the subscription
    /// monthly. Every other field stays unknown until a payload reports it.
    fn unrecorded() -> Self {
        Self {
            provider_object_id: None,
            provider_order_id: None,
            provider_checkout_id: None,
            provider_customer_id: None,
            customer_email: None,
            order_email: None,
            status: None,
            currency: None,
            provider_price_id: None,
            plan_name: None,
            quantity: None,
            plan_amount: None,
            interval_unit: "month".to_string(),
            interval_count: 1,
            current_period_start: None,
            current_period_end: None,
            next_billing_at: None,
            started_at: None,
            ended_at: None,
            canceled_at: None,
            cancel_at: None,
            metadata: None,
        }
    }

    /// Lays an incoming payload over the stored state, field by field.
    fn merged(subscription: &SubscriptionUpsert<'_>, stored: Option<Self>) -> Self {
        let stored = stored.unwrap_or_else(Self::unrecorded);
        let plan = &subscription.plan;
        // Unit and count only mean anything together, so an interval is carried
        // over whole rather than one field at a time.
        let (interval_unit, interval_count) = match plan.interval {
            Some(interval) => (interval.unit.to_string(), interval.count),
            None => (stored.interval_unit, stored.interval_count),
        };
        Self {
            provider_object_id: subscription
                .provider_object_id
                .or(stored.provider_object_id),
            provider_order_id: owned(subscription.provider_order_id).or(stored.provider_order_id),
            provider_checkout_id: owned(subscription.provider_checkout_id)
                .or(stored.provider_checkout_id),
            provider_customer_id: owned(subscription.provider_customer_id)
                .or(stored.provider_customer_id),
            customer_email: owned(subscription.customer_email).or(stored.customer_email),
            order_email: owned(subscription.order_email).or(stored.order_email),
            status: subscription
                .normalized_status()
                .map(str::to_string)
                .or(stored.status),
            currency: owned(plan.currency).or(stored.currency),
            provider_price_id: owned(plan.provider_price_id).or(stored.provider_price_id),
            plan_name: owned(plan.plan_name).or(stored.plan_name),
            quantity: plan.quantity.or(stored.quantity),
            plan_amount: plan.amount.or(stored.plan_amount),
            interval_unit,
            interval_count,
            current_period_start: subscription
                .current_period_start
                .or(stored.current_period_start),
            current_period_end: subscription
                .current_period_end
                .or(stored.current_period_end),
            next_billing_at: subscription.next_billing_at().or(stored.next_billing_at),
            started_at: subscription.started_at.or(stored.started_at),
            ended_at: subscription.ended_at.applied_to(stored.ended_at),
            canceled_at: subscription.canceled_at.applied_to(stored.canceled_at),
            cancel_at: subscription.cancel_at.applied_to(stored.cancel_at),
            metadata: owned(subscription.metadata).or(stored.metadata),
        }
    }

    /// The plan amount normalized to a month, for an interval that can be.
    pub(super) fn mrr_amount(&self) -> Option<i64> {
        self.plan_amount
            .and_then(|amount| monthly_amount(amount, &self.interval_unit, self.interval_count))
    }

    /// Whether a cancellation is in force, whether or not it has taken effect.
    /// A subscription scheduled to stop at period end is already canceled even
    /// though it keeps billing until then.
    pub(super) fn is_canceled(&self) -> bool {
        self.canceled_at.is_some() || self.cancel_at.is_some() || self.ended_at.is_some()
    }
}

/// Upserts the subscription anchor row and appends a history version when its
/// tracked state changed, returning the anchor's row id.
///
/// The anchor write, closing the open history version, and inserting its
/// successor share one write transaction so a failure cannot leave the anchor
/// ahead of history or history with no open version.
pub(in crate::integration) fn upsert_subscription(
    conn: &ProjectConnection,
    subscription: &SubscriptionUpsert<'_>,
    counts: &mut SyncCounts,
) -> SyncResult<i64> {
    conn.execute("BEGIN IMMEDIATE", [])?;
    match upsert_subscription_in_transaction(conn, subscription) {
        Ok((subscription_id, existed)) => {
            conn.execute("COMMIT", [])?;
            count_upsert(existed, counts);
            Ok(subscription_id)
        }
        Err(error) => {
            if let Err(rollback_error) = conn.execute("ROLLBACK", []) {
                warn!(
                    "failed to roll back subscription upsert for {}/{}: {rollback_error}",
                    subscription.provider, subscription.provider_subscription_id
                );
            }
            Err(error)
        }
    }
}

pub(super) fn upsert_subscription_in_transaction(
    conn: &ProjectConnection,
    subscription: &SubscriptionUpsert<'_>,
) -> SyncResult<(i64, bool)> {
    let stored = stored_subscription(
        conn,
        subscription.provider,
        subscription.provider_subscription_id,
    )?;
    let existed = stored.is_some();
    let state = SubscriptionState::merged(subscription, stored);

    // Every column is written as-is: `state` already resolved each field against
    // what this row holds, and doing that here rather than in the update is what
    // keeps the anchor and the history version below describing the same thing.
    let subscription_id = conn.query_row(
        "INSERT INTO subscriptions (
            provider, provider_subscription_id, provider_object_id,
            provider_order_id, provider_checkout_id, provider_customer_id,
            customer_email, order_email, status, currency,
            provider_price_id, plan_name, quantity, plan_amount, mrr_amount,
            interval_unit, interval_count,
            current_period_start, current_period_end, next_billing_at,
            started_at, ended_at, canceled_at, cancel_at, metadata
         ) VALUES (
            ?1, ?2, ?3,
            ?4, ?5, ?6,
            ?7, ?8, ?9, ?10,
            ?11, ?12, ?13, ?14, ?15,
            ?16, ?17,
            ?18, ?19, ?20,
            ?21, ?22, ?23, ?24, ?25
         )
         ON CONFLICT(provider, provider_subscription_id) DO UPDATE SET
            provider_object_id = excluded.provider_object_id,
            provider_order_id = excluded.provider_order_id,
            provider_checkout_id = excluded.provider_checkout_id,
            provider_customer_id = excluded.provider_customer_id,
            customer_email = excluded.customer_email,
            order_email = excluded.order_email,
            status = excluded.status,
            currency = excluded.currency,
            provider_price_id = excluded.provider_price_id,
            plan_name = excluded.plan_name,
            quantity = excluded.quantity,
            plan_amount = excluded.plan_amount,
            mrr_amount = excluded.mrr_amount,
            interval_unit = excluded.interval_unit,
            interval_count = excluded.interval_count,
            current_period_start = excluded.current_period_start,
            current_period_end = excluded.current_period_end,
            next_billing_at = excluded.next_billing_at,
            started_at = excluded.started_at,
            ended_at = excluded.ended_at,
            canceled_at = excluded.canceled_at,
            cancel_at = excluded.cancel_at,
            metadata = excluded.metadata
         RETURNING id",
        params![
            subscription.provider,
            subscription.provider_subscription_id,
            state.provider_object_id,
            state.provider_order_id,
            state.provider_checkout_id,
            state.provider_customer_id,
            state.customer_email,
            state.order_email,
            state.status,
            state.currency,
            state.provider_price_id,
            state.plan_name,
            state.quantity,
            state.plan_amount,
            state.mrr_amount(),
            state.interval_unit,
            state.interval_count,
            state.current_period_start,
            state.current_period_end,
            state.next_billing_at,
            state.started_at,
            state.ended_at,
            state.canceled_at,
            state.cancel_at,
            state.metadata,
        ],
        |row| row.get::<_, i64>(0),
    )?;

    append_subscription_history(conn, subscription_id, &state, subscription.observed_at)?;
    attribution::propagate_linked_subscription_payment_fields(conn, subscription_id)?;
    Ok((subscription_id, existed))
}

/// The state a previous sync left in the anchor row, which an incoming payload
/// is merged onto.
pub(super) fn stored_subscription(
    conn: &ProjectConnection,
    provider: &str,
    provider_subscription_id: &str,
) -> SyncResult<Option<SubscriptionState>> {
    Ok(conn
        .query_row(
            "SELECT provider_object_id, provider_order_id, provider_checkout_id,
                    provider_customer_id, customer_email, order_email, status, currency,
                    provider_price_id, plan_name, quantity, plan_amount,
                    interval_unit, interval_count,
                    current_period_start, current_period_end, next_billing_at,
                    started_at, ended_at, canceled_at, cancel_at, metadata
             FROM subscriptions
             WHERE provider = ?1 AND provider_subscription_id = ?2",
            params![provider, provider_subscription_id],
            |row| {
                Ok(SubscriptionState {
                    provider_object_id: row.get(0)?,
                    provider_order_id: row.get(1)?,
                    provider_checkout_id: row.get(2)?,
                    provider_customer_id: row.get(3)?,
                    customer_email: row.get(4)?,
                    order_email: row.get(5)?,
                    status: row.get(6)?,
                    currency: row.get(7)?,
                    provider_price_id: row.get(8)?,
                    plan_name: row.get(9)?,
                    quantity: row.get(10)?,
                    plan_amount: row.get(11)?,
                    interval_unit: row.get(12)?,
                    interval_count: row.get(13)?,
                    current_period_start: row.get(14)?,
                    current_period_end: row.get(15)?,
                    next_billing_at: row.get(16)?,
                    started_at: row.get(17)?,
                    ended_at: row.get(18)?,
                    canceled_at: row.get(19)?,
                    cancel_at: row.get(20)?,
                    metadata: row.get(21)?,
                })
            },
        )
        .optional()?)
}

pub(in crate::integration) fn subscription_row_id(
    conn: &ProjectConnection,
    provider: &str,
    provider_subscription_id: &str,
) -> SyncResult<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT id
             FROM subscriptions
             WHERE provider = ?1 AND provider_subscription_id = ?2",
            params![provider, provider_subscription_id],
            |row| row.get(0),
        )
        .optional()?)
}
