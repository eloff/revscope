//! Canonical subscription lifecycle states.
//!
//! Every provider names these differently, and some report a state no other
//! provider has. Mapping them onto one small set as they are synced lets the
//! metrics queries ask whether a subscription is being billed without knowing
//! which provider it came from. The provider's own wording is still recoverable
//! from the stored `provider_objects` payload.

/// In a trial that has not converted to a paid period.
pub(crate) const TRIALING: &str = "trialing";
/// Billing normally.
pub(crate) const ACTIVE: &str = "active";
/// A charge failed and the provider is still retrying it. Still billing, so
/// dunning does not look like churn.
pub(crate) const PAST_DUE: &str = "past_due";
/// Collection deliberately suspended, expected to resume.
pub(crate) const PAUSED: &str = "paused";
/// The provider stopped trying to collect but has not terminated the
/// subscription.
pub(crate) const UNPAID: &str = "unpaid";
/// Not billing yet: scheduled to start later, or a signup whose first payment
/// never went through.
pub(crate) const PENDING: &str = "pending";
/// Terminated, whether canceled by the customer or expired on its own.
pub(crate) const CANCELED: &str = "canceled";
/// A status we have no mapping for, which counts as billing so that a provider
/// adding a state we don't know about can't silently erase revenue.
pub(crate) const UNKNOWN: &str = "unknown";

/// Maps a provider's own subscription status onto the canonical set.
///
/// The provider strings are matched as one union rather than per provider: they
/// barely overlap, and where they do (`paused`, `active`) they mean the same
/// thing.
pub(crate) fn normalize(status: &str) -> &'static str {
    match status.trim().to_ascii_lowercase().as_str() {
        "active" => ACTIVE,
        // Lemon Squeezy calls a trial `on_trial`.
        "trialing" | "on_trial" => TRIALING,
        "past_due" => PAST_DUE,
        // Dodo puts a suspended subscription `on_hold`.
        "on_hold" | "paused" => PAUSED,
        // Shopify marks a contract `failed` once its billing attempts are
        // exhausted, and `stale` when it has gone unbilled for a long time.
        "unpaid" | "failed" | "stale" => UNPAID,
        // Stripe and Polar report a subscription whose first payment has not
        // settled as `incomplete`; Square and Dodo call a not-yet-started one
        // `pending`.
        "pending" | "incomplete" => PENDING,
        // Square reports a terminated subscription as `deactivated`, and
        // Stripe an abandoned signup as `incomplete_expired`.
        "canceled" | "cancelled" | "expired" | "incomplete_expired" | "deactivated" => CANCELED,
        _ => UNKNOWN,
    }
}

/// The live states in which a subscription's recurring price is not being
/// collected, so it is not MRR.
///
/// A subscription in one of these states may still be worth money later, but
/// counting it now would book revenue nobody is being charged.
///
/// [`CANCELED`] is deliberately absent. When a subscription stops is a question
/// of dates, not of status: a cancellation requested today can leave the
/// customer paying through the end of a period they have already bought, and
/// providers disagree about when they flip the status. The metrics queries end
/// a subscription on its `ended_at`/`cancel_at`/`canceled_at` instead, and
/// letting a status overrule that would drop MRR up to a billing period early.
pub(crate) const NON_BILLING: [&str; 4] = [TRIALING, PAUSED, UNPAID, PENDING];

/// [`NON_BILLING`] as a quoted SQL list, for an `IN (...)` filter.
pub(crate) fn non_billing_sql_list() -> String {
    NON_BILLING.map(|status| format!("'{status}'")).join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_wording_maps_onto_the_canonical_states() {
        // Stripe and Polar
        assert_eq!(normalize("trialing"), TRIALING);
        assert_eq!(normalize("past_due"), PAST_DUE);
        assert_eq!(normalize("incomplete"), PENDING);
        assert_eq!(normalize("incomplete_expired"), CANCELED);
        // Lemon Squeezy
        assert_eq!(normalize("on_trial"), TRIALING);
        assert_eq!(normalize("cancelled"), CANCELED);
        assert_eq!(normalize("expired"), CANCELED);
        // Dodo
        assert_eq!(normalize("on_hold"), PAUSED);
        assert_eq!(normalize("failed"), UNPAID);
        // Square
        assert_eq!(normalize("DEACTIVATED"), CANCELED);
        assert_eq!(normalize("PENDING"), PENDING);
        // Shopify
        assert_eq!(normalize("ACTIVE"), ACTIVE);
        assert_eq!(normalize("PAUSED"), PAUSED);
        assert_eq!(normalize("STALE"), UNPAID);
    }

    #[test]
    fn an_unrecognized_status_still_counts_as_billing() {
        assert_eq!(normalize("half_off_tuesdays"), UNKNOWN);
        assert!(!NON_BILLING.contains(&UNKNOWN));
        assert!(!NON_BILLING.contains(&ACTIVE));
        assert!(
            !NON_BILLING.contains(&PAST_DUE),
            "a subscription in dunning is still being billed"
        );
    }

    #[test]
    fn the_sql_list_quotes_every_non_billing_status() {
        assert_eq!(
            non_billing_sql_list(),
            "'trialing', 'paused', 'unpaid', 'pending'"
        );
    }

    #[test]
    fn cancellation_is_left_to_the_dates_rather_than_the_status() {
        assert!(
            !NON_BILLING.contains(&CANCELED),
            "a cancellation requested today can still be paid through the end of the period"
        );
    }
}
