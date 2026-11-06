//! Writes against `account_subscriptions` period history.
//!
//! Periods are half-open `[started_at, ended_at)`; the open period has
//! `ended_at` NULL. Overlap is not constrained in Postgres (no GiST exclusion);
//! every insert and every close that opens a successor goes through a helper
//! that keeps periods contiguous and then calls [`ensure_no_overlapping_periods`].
//! The signup insert is here; the billing providers' changes are in the
//! private billing module.

use crate::db::PostgresError;
use crate::db::main::Connection;

#[derive(Debug)]
pub(crate) enum AccountSubscriptionError {
    /// Two periods for the same account have overlapping time ranges.
    OverlappingPeriods,
    Db(String),
}

impl From<tokio_postgres::Error> for AccountSubscriptionError {
    fn from(error: tokio_postgres::Error) -> Self {
        Self::Db(PostgresError::from(error).to_string())
    }
}

/// The plan and locked-in prices an account's first open period starts with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PlanTerms {
    pub(crate) plan_type: &'static str,
    pub(crate) events_monthly_limit: Option<i32>,
    pub(crate) billing_period: &'static str,
    pub(crate) regular_price_cents: i32,
    pub(crate) actual_price_cents: i32,
    /// When set, `discount_ends_at = NOW() + N years`.
    pub(crate) discount_years: Option<i32>,
}

impl PlanTerms {
    /// Unlimited and never billed: every self-hosted account, and every hosted
    /// signup in a build without the private `billing` module.
    pub(crate) const COMPLIMENTARY: Self = Self {
        plan_type: "enterprise",
        events_monthly_limit: None,
        billing_period: "yearly",
        regular_price_cents: 0,
        actual_price_cents: 0,
        discount_years: None,
    };
}

/// Opens a new subscription period for an account that has none yet (signup).
///
/// Verifies afterwards that the account's periods do not overlap.
pub(crate) async fn insert_open_period(
    conn: &Connection,
    account_id: &str,
    terms: &PlanTerms,
) -> Result<(), AccountSubscriptionError> {
    conn.execute(
        "INSERT INTO account_subscriptions (
            account_id, plan_type, events_monthly_limit, billing_period,
            regular_price_cents, actual_price_cents, discount_ends_at
         )
         VALUES (
            $1::text::uuid, $2::text::plan_type, $3, $4::text::billing_period, $5, $6,
            CASE
                WHEN $7::int IS NULL THEN NULL
                ELSE NOW() + ($7::int * INTERVAL '1 year')
            END
         )",
        &[
            &account_id,
            &terms.plan_type,
            &terms.events_monthly_limit,
            &terms.billing_period,
            &terms.regular_price_cents,
            &terms.actual_price_cents,
            &terms.discount_years,
        ],
    )
    .await?;
    ensure_no_overlapping_periods(conn, account_id).await
}

/// Fails if any two periods for `account_id` have overlapping half-open ranges.
///
/// Contiguous periods that only touch at an endpoint (`ended_at` of one equals
/// `started_at` of the next) are allowed.
pub(crate) async fn ensure_no_overlapping_periods(
    conn: &Connection,
    account_id: &str,
) -> Result<(), AccountSubscriptionError> {
    let overlaps: bool = conn
        .query_one(
            "SELECT EXISTS (
                SELECT 1
                FROM account_subscriptions a
                INNER JOIN account_subscriptions b
                    ON a.account_id = b.account_id
                   AND a.id < b.id
                   AND tstzrange(a.started_at, a.ended_at, '[)')
                       && tstzrange(b.started_at, b.ended_at, '[)')
                WHERE a.account_id = $1::text::uuid
             )",
            &[&account_id],
        )
        .await?
        .get(0);
    if overlaps {
        return Err(AccountSubscriptionError::OverlappingPeriods);
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::test_utils::{insert_test_account, main_test_db};

    /// One `account_subscriptions` row, with times in microseconds so a
    /// successor starting where its predecessor ended compares equal.
    #[derive(Debug)]
    #[cfg_attr(not(feature = "billing"), allow(dead_code))]
    pub(crate) struct Period {
        pub(crate) id: String,
        pub(crate) plan_type: String,
        pub(crate) events_monthly_limit: Option<i32>,
        pub(crate) billing_period: String,
        pub(crate) regular_price_cents: i32,
        pub(crate) actual_price_cents: i32,
        pub(crate) discount_ends_at: Option<i64>,
        pub(crate) stripe_customer_id: Option<String>,
        pub(crate) stripe_subscription_id: Option<String>,
        pub(crate) polar_customer_id: Option<String>,
        pub(crate) polar_subscription_id: Option<String>,
        pub(crate) subscription_status: Option<String>,
        pub(crate) current_period_end: Option<i64>,
        pub(crate) started_at: i64,
        pub(crate) ended_at: Option<i64>,
    }

    pub(crate) async fn periods(conn: &Connection, account_id: &str) -> Vec<Period> {
        conn.query(
            "SELECT id::text, plan_type::text, events_monthly_limit, billing_period::text,
                    regular_price_cents, actual_price_cents,
                    EXTRACT(EPOCH FROM discount_ends_at)::bigint,
                    stripe_customer_id, stripe_subscription_id,
                    polar_customer_id, polar_subscription_id,
                    subscription_status,
                    EXTRACT(EPOCH FROM current_period_end)::bigint,
                    (EXTRACT(EPOCH FROM started_at) * 1000000)::bigint,
                    (EXTRACT(EPOCH FROM ended_at) * 1000000)::bigint
             FROM account_subscriptions
             WHERE account_id = $1::text::uuid
             ORDER BY started_at",
            &[&account_id],
        )
        .await
        .expect("load periods")
        .iter()
        .map(|row| Period {
            id: row.get(0),
            plan_type: row.get(1),
            events_monthly_limit: row.get(2),
            billing_period: row.get(3),
            regular_price_cents: row.get(4),
            actual_price_cents: row.get(5),
            discount_ends_at: row.get(6),
            stripe_customer_id: row.get(7),
            stripe_subscription_id: row.get(8),
            polar_customer_id: row.get(9),
            polar_subscription_id: row.get(10),
            subscription_status: row.get(11),
            current_period_end: row.get(12),
            started_at: row.get(13),
            ended_at: row.get(14),
        })
        .collect()
    }

    pub(crate) const STARTER: PlanTerms = PlanTerms {
        plan_type: "starter",
        events_monthly_limit: Some(10_000),
        billing_period: "monthly",
        regular_price_cents: 3_900,
        actual_price_cents: 2_900,
        discount_years: None,
    };

    pub(crate) async fn delete_account(conn: &Connection, account_id: &str) {
        conn.execute(
            "DELETE FROM accounts WHERE id = $1::text::uuid",
            &[&account_id],
        )
        .await
        .expect("delete account");
    }

    #[tokio::test]
    async fn a_signup_opens_one_period_and_a_second_open_period_is_refused() {
        let (_context, conn) = main_test_db("account-subscription-signup").await;
        let account_id = insert_test_account(&conn, "signup").await;
        let founder = PlanTerms {
            plan_type: "founder",
            actual_price_cents: 1_900,
            discount_years: Some(2),
            ..STARTER
        };
        insert_open_period(&conn, &account_id, &founder)
            .await
            .expect("open period");

        let opened = periods(&conn, &account_id).await;
        assert_eq!(opened.len(), 1);
        assert_eq!(opened[0].plan_type, "founder");
        assert_eq!(opened[0].events_monthly_limit, Some(10_000));
        assert_eq!(opened[0].regular_price_cents, 3_900);
        assert_eq!(opened[0].actual_price_cents, 1_900);
        assert_eq!(opened[0].ended_at, None);
        let discount_ends_at = opened[0].discount_ends_at.expect("promo end");
        let two_years = 2 * 365 * 24 * 60 * 60;
        assert!(
            (discount_ends_at - crate::util::unix_seconds() - two_years).abs() < 2 * 24 * 60 * 60
        );

        let second = insert_open_period(&conn, &account_id, &STARTER).await;
        assert!(matches!(second, Err(AccountSubscriptionError::Db(_))));
        assert_eq!(periods(&conn, &account_id).await.len(), 1);
        delete_account(&conn, &account_id).await;
    }

    #[tokio::test]
    async fn periods_may_touch_but_not_overlap() {
        let (_context, conn) = main_test_db("account-subscription-overlap").await;
        let account_id = insert_test_account(&conn, "overlap").await;
        let insert = |started: &'static str, ended: Option<&'static str>| {
            let conn = &conn;
            let account_id = account_id.clone();
            async move {
                conn.execute(
                    "INSERT INTO account_subscriptions (
                        account_id, plan_type, billing_period,
                        regular_price_cents, actual_price_cents, started_at, ended_at
                     ) VALUES (
                        $1::text::uuid, 'starter', 'monthly', 2900, 2900,
                        $2::text::timestamptz, $3::text::timestamptz
                     )",
                    &[&account_id, &started, &ended],
                )
                .await
                .expect("insert period");
            }
        };
        insert("2026-01-01T00:00:00Z", Some("2026-02-01T00:00:00Z")).await;
        insert("2026-02-01T00:00:00Z", None).await;
        ensure_no_overlapping_periods(&conn, &account_id)
            .await
            .expect("contiguous periods are allowed");

        insert("2026-01-15T00:00:00Z", Some("2026-01-20T00:00:00Z")).await;
        let overlap = ensure_no_overlapping_periods(&conn, &account_id).await;
        assert!(matches!(
            overlap,
            Err(AccountSubscriptionError::OverlappingPeriods)
        ));
        delete_account(&conn, &account_id).await;
    }
}
