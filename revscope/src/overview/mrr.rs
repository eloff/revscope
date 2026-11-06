use std::collections::BTreeMap;

use rusqlite::params;

use crate::auth::{AuthError, map_row_error};
use crate::comparison::DAY_SECONDS;
use crate::db::ProjectConnection;
use crate::subscription_status;
use crate::timezone::Buckets;

use super::buckets::OverviewBucket;
pub(super) const CHURN_RESUBSCRIBE_WINDOW_SECONDS: i64 = 7 * DAY_SECONDS;

/// SQL for the last moment a churned subscription still counted toward MRR.
///
/// Cancelling appends a history version dated when it was requested, so reading
/// the state at the churn timestamp itself reads the cancellation rather than
/// what was lost.
fn churn_instant() -> String {
    format!("{} - 1", effective_end("s"))
}

/// SQL for when a subscription's revenue actually stops.
///
/// `canceled_at` is when the cancellation was *requested*. For one scheduled at
/// period end that is not when billing stops: Stripe documents that its
/// `canceled_at` "will reflect the time of the most recent update request, not
/// the end of the subscription period", and Polar, Square, Lemon Squeezy and
/// Dodo all draw the same distinction. The customer has paid through
/// `cancel_at` and keeps the subscription until then, so keying off the request
/// would drop MRR and book churn up to a full billing period early. `ended_at`
/// wins over both, being the end the provider confirmed rather than projected.
fn effective_end(alias: &str) -> String {
    format!("COALESCE({alias}.ended_at, {alias}.cancel_at, {alias}.canceled_at)")
}

/// Churned subscriptions and the MRR they took with them, per bucket.
///
/// Churn is keyed off the anchor row's [`effective_end`], so a subscription
/// counts once, on the day it stopped being worth anything, even when it was
/// canceled well before that and only later ended.
///
/// The lost MRR is whatever the subscription was contributing an instant before
/// it churned, under the same rules [`fetch_mrr`] applies, so what churn
/// removes always matches what MRR was counting. A canceled trial and a
/// subscription that was already paused therefore churn zero MRR rather than
/// the price they were never charged.
pub(super) fn populate_churn_metrics(
    conn: &ProjectConnection,
    buckets: &mut BTreeMap<i64, OverviewBucket>,
    bucket_bounds: &Buckets,
    start: i64,
    end: i64,
) -> Result<(), AuthError> {
    conn.with_query(
        // The resubscribe window is a constant, so we format it into the query
        // directly. Rows come back unbucketed and are grouped in code, because
        // timezone bucket boundaries are irregular and cannot be expressed as
        // integer division in SQL.
        &format!(
            "SELECT churn_at,
                    churn_mrr
             FROM (
                 SELECT s.id,
                        {effective_end} AS churn_at,
                        COALESCE(s.provider_customer_id, s.customer_email, s.order_email) AS customer_key,
                        CASE WHEN {realized} THEN COALESCE({mrr_before_churn}, 0)
                             ELSE 0 END AS churn_mrr
                 FROM subscriptions s
                 WHERE {effective_end} IS NOT NULL
                   AND {effective_end} >= ?1
                   AND {effective_end} < ?2
             ) churned
             WHERE NOT EXISTS (
                 SELECT 1
                 FROM subscriptions renewed
                 WHERE COALESCE(renewed.provider_customer_id, renewed.customer_email, renewed.order_email) = churned.customer_key
                   AND renewed.id != churned.id
                   AND renewed.started_at IS NOT NULL
                   AND renewed.started_at > churned.churn_at
                   AND renewed.started_at <= churned.churn_at + {resubscribe_window_seconds}
             )",
            resubscribe_window_seconds = CHURN_RESUBSCRIBE_WINDOW_SECONDS,
            effective_end = effective_end("s"),
            realized = realized_payment_exists("s.id", &churn_instant()),
            mrr_before_churn = mrr_at_instant("s.id", &churn_instant()),
        ),
        params![start, end],
        |mut rows| {
            while let Some(row) = rows.next()? {
                let churn_at: i64 = row.get(0)?;
                let churn_mrr: i64 = row.get(1)?;
                let Some(bucket) = bucket_bounds.bucket_for_second(churn_at) else {
                    continue;
                };
                if let Some(entry) = buckets.get_mut(&bucket) {
                    entry.churn = entry.churn.saturating_add(1);
                    entry.churn_mrr += churn_mrr;
                }
            }
            Ok(())
        },
    )
    .map_err(map_row_error)
}

/// SQL predicate that holds when a subscription has been paid for at least once
/// strictly before `before`.
///
/// A subscription carries its recurring price from the moment it is created,
/// but that price is only revenue once the customer has been charged for it, so
/// a trial or an unpaid signup contributes nothing until its first invoice
/// settles. The amount has to be non-zero because providers record the opening
/// invoice of a free trial as a settled subscription payment of zero.
///
/// `subscription_id` and `before` are SQL expressions written by the callers in
/// this module, not user input.
fn realized_payment_exists(subscription_id: &str, before: &str) -> String {
    format!(
        "EXISTS (
             SELECT 1
             FROM payments realized
             WHERE realized.subscription_id = {subscription_id}
               AND realized.status = 'ok'
               AND realized.payment_type = 'subscription'
               AND realized.amount > 0
               AND realized.paid_at < {before}
         )"
    )
}

/// SQL scalar subquery for the monthly-normalized amount of the last payment a
/// subscription actually made before `before`, used as the MRR of
/// subscriptions whose recurring price we never saw.
fn last_realized_payment_mrr(subscription_id: &str, before: &str) -> String {
    format!(
        "(SELECT realized.mrr_amount
          FROM payments realized
          WHERE realized.subscription_id = {subscription_id}
            AND realized.status = 'ok'
            AND realized.payment_type = 'subscription'
            AND realized.amount > 0
            AND realized.paid_at < {before}
          ORDER BY realized.paid_at DESC
          LIMIT 1)"
    )
}

/// SQL predicate for a lifecycle state that is actually being billed, applied
/// to the status column of a `subscription_history` version.
///
/// This covers only the live states in which nobody is being charged, such as a
/// trial or a pause. When a subscription stops for good is [`effective_end`]'s
/// job, so a status is never what removes a subscription from MRR.
///
/// A missing or unrecognized status counts as billing. Sync normalizes what the
/// providers report onto [`crate::subscription_status`], but a provider adding
/// a state we haven't mapped should leave revenue overstated rather than make
/// it disappear.
fn billing_status(status_column: &str) -> String {
    format!(
        "({status_column} IS NULL OR {status_column} NOT IN ({non_billing}))",
        non_billing = subscription_status::non_billing_sql_list(),
    )
}

/// SQL for what one `subscription_history` version is worth per month: the
/// recurring price it carries, the price the version before it carried when a
/// partial sync left this one without one, or the last payment the subscription
/// actually made.
fn version_mrr(version: &str, before: &str) -> String {
    format!(
        "COALESCE({version}.mrr_amount, {version}.prev_mrr_amount, {last_payment})",
        last_payment = last_realized_payment_mrr(&format!("{version}.subscription_id"), before),
    )
}

/// SQL scalar subquery for the MRR a single subscription was contributing at
/// `instant`, which is zero (NULL) when it was not being billed then.
///
/// Seeks the covering history version via `(subscription_id, effective_from)`
/// rather than scanning every version with `effective_from <= instant`.
fn mrr_at_instant(subscription_id: &str, instant: &str) -> String {
    format!(
        "(SELECT {mrr}
          FROM subscription_history covering
          WHERE covering.subscription_id = {subscription_id}
            AND covering.effective_from <= {instant}
            AND (covering.effective_to IS NULL OR covering.effective_to > {instant})
            AND ({effective_end} IS NULL OR {effective_end} > {instant})
            AND {billing}
          ORDER BY covering.effective_from DESC
          LIMIT 1)",
        mrr = version_mrr("covering", instant),
        effective_end = effective_end("covering"),
        billing = billing_status("covering.status"),
    )
}

/// MRR in effect at `as_of`, summed over the subscriptions that were active then.
///
/// Starts from `subscriptions` that could still be alive at `as_of`, then each
/// contributes the one covering `subscription_history` version via
/// [`mrr_at_instant`]. Driving from history with `effective_from <= as_of` is
/// not selective for a current as_of — nearly every version ever written
/// matches — so the covering-version lookup is done per candidate subscription
/// instead. Versions written before we knew a subscription's recurring price
/// carry no `mrr_amount`, and fall back to normalizing its last payment.
///
/// A subscription counts only while it is both being billed and has realized
/// revenue, so a trial's future price stays out of MRR until the customer is
/// charged, and a paused or written-off subscription drops out until it
/// resumes.
pub(super) fn fetch_mrr(conn: &ProjectConnection, as_of: i64) -> Result<i64, AuthError> {
    conn.query_row(
        &format!(
            "SELECT COALESCE(SUM({mrr}), 0)
             FROM subscriptions s
             WHERE (s.started_at IS NULL OR s.started_at <= ?1)
               AND ({effective_end} IS NULL OR {effective_end} > ?1)
               AND {realized}",
            effective_end = effective_end("s"),
            mrr = mrr_at_instant("s.id", "?1"),
            realized = realized_payment_exists("s.id", "?1"),
        ),
        params![as_of],
        |row| row.get::<_, i64>(0),
    )
    .map_err(map_row_error)
}
