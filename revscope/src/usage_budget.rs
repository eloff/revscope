//! The monthly allowance an account's plan gives it for features that cost us
//! money each time they're used, and what the account has spent of it this
//! calendar month. Every such feature draws on the same allowance, so each one
//! records its usage in its own table and adds that table's cost to the month's
//! spend in [`allowance`].
//!
//! Free and Founder & Friends accounts get no allowance. Starter gets a flat
//! $1.20. Other plans get 40% of `actual_price_cents` or $10, whichever is
//! larger, except that an account that came through a trial gets $5 until its
//! first real payment. A self-hosted instance pays its providers itself, so it
//! has no allowance to run out of.

use tokio_postgres::types::ToSql;

use crate::auth::AuthError;
use crate::db::main::Connection;

/// Spend is counted in millionths of a cent, because one AI token costs far
/// less than a cent.
const MICROCENTS_PER_CENT: i64 = 1_000_000;
/// Assumed AI gateway cost of one prompt or completion token: $6 / 1M tokens.
const AI_TOKEN_MICROCENTS: i64 = 600;
/// Starter's whole monthly allowance: 200K AI tokens.
const STARTER_ALLOWANCE_CENTS: i64 = 120;
/// Floor on the monthly allowance for other paid plans.
const MIN_ALLOWANCE_CENTS: i64 = 1_000; // $10
/// Allowance while an account on those plans is still on a trial.
const TRIAL_ALLOWANCE_CENTS: i64 = 500; // $5
/// Share of the subscription's `actual_price_cents` available each month.
const ALLOWANCE_PERCENT: i64 = 40;

/// Where an account stands against its monthly allowance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Allowance {
    /// Some of this month's allowance is left.
    Available,
    /// The plan has no allowance for paid features.
    NotIncluded,
    /// This month's allowance is spent; it resets at the start of next month.
    Exhausted,
}

/// Reads the account's plan and this month's spend, summed over every metered
/// feature, in one query.
pub(crate) async fn allowance(conn: &Connection, account_id: &str) -> Result<Allowance, AuthError> {
    let sql = format!(
        "SELECT
             sub.plan_type::text,
             sub.actual_price_cents,
             -- The trial date can't say this alone: a first charge can fail after it
             -- passes, and it can stay set once paid, since webhooks copy the
             -- provider's. A status can't either: `active` can precede the first
             -- charge, and `past_due` can follow months of payments. A trial's
             -- first invoice can be a paid $0.
             a.trial_ends_at IS NOT NULL
                AND NOT EXISTS (
                    SELECT 1 FROM account_orders o
                    WHERE o.account_id = a.id
                      AND o.paid
                      AND o.total_amount_cents > 0
                      AND o.status <> 'refunded'
                ),
             (
                 SELECT COALESCE(SUM(prompt_tokens + completion_tokens), 0)::bigint
                 FROM chat_usage
                 WHERE account_id = a.id
                   AND created_at >= date_trunc('month', NOW())
             ) * {AI_TOKEN_MICROCENTS}
         FROM accounts a
         INNER JOIN account_subscriptions sub
            ON sub.account_id = a.id AND sub.ended_at IS NULL
         WHERE a.id = $1::text::uuid"
    );
    let row = conn
        .query_one(&sql, &[&account_id as &(dyn ToSql + Sync)])
        .await?;
    let plan_type: String = row.get(0);
    let actual_price_cents: i32 = row.get(1);
    let trial_unpaid: bool = row.get(2);
    let spent_microcents: i64 = row.get(3);
    let Some(allowance_cents) =
        monthly_allowance_cents(&plan_type, actual_price_cents, trial_unpaid)
    else {
        return Ok(Allowance::NotIncluded);
    };
    if !crate::self_hosted::ENABLED && spent_microcents >= allowance_cents * MICROCENTS_PER_CENT {
        return Ok(Allowance::Exhausted);
    }
    Ok(Allowance::Available)
}

/// The plan's monthly allowance in cents, or `None` when it has none.
/// Complimentary accounts on paid plans (price 0) still get the $10 floor.
fn monthly_allowance_cents(
    plan_type: &str,
    actual_price_cents: i32,
    trial_unpaid: bool,
) -> Option<i64> {
    // Founder & Friends is Growth without what costs us money to serve.
    match plan_type {
        "free" | "founder_friends" => None,
        "starter" => Some(STARTER_ALLOWANCE_CENTS),
        _ if trial_unpaid => Some(TRIAL_ALLOWANCE_CENTS),
        _ => {
            let from_subscription = i64::from(actual_price_cents) * ALLOWANCE_PERCENT / 100;
            Some(from_subscription.max(MIN_ALLOWANCE_CENTS))
        }
    }
}

/// Tokens an [`insert_metered_account`] account has used this month: more than the $5
/// trial allowance covers but fewer than the $10 paid floor.
#[cfg(all(test, not(feature = "self-hosted")))]
const TEST_ACCOUNT_TOKENS: i32 = 1_000_000;

/// Inserts an account on `plan_type` at $9/month that has used
/// [`TEST_ACCOUNT_TOKENS`] of Ask this month, and returns its id and its one
/// user's. `trial_ends_in` is an offset from now, and `order` is its only order,
/// as the amount in cents, whether it's paid, and its status.
#[cfg(all(test, not(feature = "self-hosted")))]
pub(crate) async fn insert_metered_account(
    conn: &Connection,
    tag: &str,
    plan_type: &str,
    trial_ends_in: Option<&str>,
    order: Option<(i32, bool, &str)>,
) -> (String, String) {
    let account_id = crate::test_utils::insert_test_account(conn, tag).await;
    conn.execute(
        "UPDATE accounts SET trial_ends_at = NOW() + $2::text::interval
         WHERE id = $1::text::uuid",
        &[&account_id, &trial_ends_in],
    )
    .await
    .expect("set trial end");
    conn.execute(
        "INSERT INTO account_subscriptions (
            account_id, plan_type, billing_period, regular_price_cents, actual_price_cents
         ) VALUES ($1::text::uuid, $2::text::plan_type, 'monthly', 900, 900)",
        &[&account_id, &plan_type],
    )
    .await
    .expect("open subscription period");
    if let Some((cents, paid, status)) = order {
        conn.execute(
            "INSERT INTO account_orders (
                polar_order_id, account_id, status, paid, total_amount_cents, currency, created_at
             ) VALUES ($1, $1::text::uuid, $4, $2, $3, 'usd', NOW())",
            &[&account_id, &paid, &cents, &status],
        )
        .await
        .expect("record order");
    }
    let project_id: i32 = conn
        .query_one(
            "INSERT INTO projects (account_id, name) VALUES ($1::text::uuid, $2) RETURNING id",
            &[&account_id, &format!("p{}", account_id.replace('-', ""))],
        )
        .await
        .expect("insert project")
        .get(0);
    let user_id: String = conn
        .query_one(
            "INSERT INTO users (account_id, name, email)
             VALUES ($1::text::uuid, 'Trial', $2) RETURNING id::text",
            &[&account_id, &format!("{account_id}@example.com")],
        )
        .await
        .expect("insert user")
        .get(0);
    conn.execute(
        "INSERT INTO chat_usage (account_id, user_id, project_id, model, prompt_tokens)
         VALUES ($1::text::uuid, $2::text::uuid, $3, 'test', $4)",
        &[&account_id, &user_id, &project_id, &TEST_ACCOUNT_TOKENS],
    )
    .await
    .expect("record usage");

    (account_id, user_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starter_is_capped_at_200k_ai_tokens() {
        for (price, trial_unpaid) in [(0, false), (12_900, false), (12_900, true)] {
            let cents = monthly_allowance_cents("starter", price, trial_unpaid).unwrap();
            assert_eq!(cents * MICROCENTS_PER_CENT / AI_TOKEN_MICROCENTS, 200_000);
        }
    }

    #[test]
    fn free_plans_have_no_allowance() {
        for plan in ["free", "founder_friends"] {
            assert_eq!(monthly_allowance_cents(plan, 12_900, false), None, "{plan}");
        }
        for plan in ["starter", "growth", "founder", "enterprise"] {
            assert!(
                monthly_allowance_cents(plan, 12_900, false).is_some(),
                "{plan}"
            );
        }
    }

    #[test]
    fn the_allowance_has_a_ten_dollar_floor() {
        assert_eq!(monthly_allowance_cents("growth", 0, false), Some(1_000));
        // 40% of $9 = $3.60, still below the floor.
        assert_eq!(monthly_allowance_cents("growth", 900, false), Some(1_000));
        // 40% of $25 = $10, exactly the floor.
        assert_eq!(
            monthly_allowance_cents("founder", 2_500, false),
            Some(1_000)
        );
    }

    #[test]
    fn the_allowance_scales_with_subscription_price() {
        // 40% of $129 = $51.60.
        assert_eq!(
            monthly_allowance_cents("growth", 12_900, false),
            Some(5_160)
        );
        assert_eq!(
            monthly_allowance_cents("enterprise", 12_900, false),
            Some(5_160)
        );
    }

    #[test]
    fn unpaid_trials_are_capped_at_five_dollars() {
        assert_eq!(monthly_allowance_cents("growth", 12_900, true), Some(500));
        assert_eq!(monthly_allowance_cents("founder", 0, true), Some(500));
    }

    /// Whether a growth account from [`insert_metered_account`] has any allowance left.
    #[cfg(not(feature = "self-hosted"))]
    async fn allowance_left(trial_ends_in: Option<&str>, order: Option<(i32, bool, &str)>) -> bool {
        let tag = "allowance_trial";
        let (_server_context, conn) = crate::test_utils::main_test_db(tag).await;
        let (account_id, _) =
            insert_metered_account(&conn, tag, "growth", trial_ends_in, order).await;
        let result = allowance(&conn, &account_id).await.expect("read allowance");
        conn.execute(
            "DELETE FROM accounts WHERE id = $1::text::uuid",
            &[&account_id],
        )
        .await
        .expect("delete account");
        result == Allowance::Available
    }

    #[cfg(not(feature = "self-hosted"))]
    #[tokio::test(flavor = "multi_thread")]
    async fn the_trial_budget_lasts_until_the_first_real_payment() {
        // Paid, with the provider's trial date still set.
        assert!(allowance_left(Some("-7 days"), Some((900, true, "paid"))).await);
        // Partly refunded, as a plan change does, is still a real payment.
        assert!(allowance_left(Some("-7 days"), Some((900, true, "partially_refunded"))).await);
        // Never had a trial: a complimentary account has no orders at all.
        assert!(allowance_left(None, None).await);

        assert!(!allowance_left(Some("7 days"), None).await);
        // The date passed but nothing has been paid.
        assert!(!allowance_left(Some("-7 days"), None).await);
        assert!(!allowance_left(Some("-7 days"), Some((900, false, "pending"))).await);
        assert!(!allowance_left(Some("-7 days"), Some((900, true, "refunded"))).await);
        // The invoice that starts a trial.
        assert!(!allowance_left(Some("-7 days"), Some((0, true, "paid"))).await);
    }
}
