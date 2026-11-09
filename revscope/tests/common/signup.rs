//! Signing up on the hosted build, shared by the public signup tests and the
//! private billing repository's plan tests.

use std::io;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use super::TestServer;

pub const SIGNUP_PATH: &str = "/v2/public/auth/signup";
pub const PASSWORD: &str = "a-long-enough-password";

/// A valid self-serve signup request. Websites and emails are unique per test
/// so that runs against the shared main database never collide.
#[derive(Clone, Debug)]
pub struct NewSignup {
    pub slug: String,
    pub user_name: String,
    pub account_name: String,
    /// What the backend derives from `account_name` for tracking/ingest.
    pub project_name: String,
    /// The project name the form sends after the user edited it; `None` sends
    /// none, so the backend derives `project_name`.
    pub requested_project_name: Option<String>,
    pub email: String,
    pub password: String,
    pub plan: String,
    pub events: String,
    pub billing: String,
    pub timezone: Option<String>,
}

impl NewSignup {
    pub fn new(tag: &str) -> Self {
        let slug = unique_slug(tag);
        Self {
            user_name: "Ada Lovelace".to_string(),
            account_name: format!("{slug}.example"),
            project_name: format!("{slug}example"),
            requested_project_name: None,
            email: format!("{slug}@example.com"),
            password: PASSWORD.to_string(),
            plan: "starter".to_string(),
            events: "100k".to_string(),
            billing: "monthly".to_string(),
            timezone: None,
            slug,
        }
    }

    pub fn body(&self) -> String {
        let mut body = json!({
            "name": self.user_name,
            "account_name": self.account_name,
            "email": self.email,
            "password": self.password,
            "plan": self.plan,
            "events": self.events,
            "billing": self.billing,
        });
        if let Some(project_name) = &self.requested_project_name {
            body["project_name"] = json!(project_name);
        }
        if let Some(timezone) = &self.timezone {
            body["timezone"] = json!(timezone);
        }
        body.to_string()
    }
}

/// Marks a signup's subscription as paid, for tests of what a paying account can
/// do. A build with billing leaves a paid plan unpaid until checkout completes.
pub async fn mark_subscription_paid(server: &TestServer, account_name: &str) {
    let conn = revscope::db::open_main_database_write(server.context.clone())
        .await
        .expect("open main database");
    let updated = conn
        .execute(
            "UPDATE account_subscriptions SET subscription_status = 'active'
             WHERE ended_at IS NULL
               AND account_id = (SELECT id FROM accounts WHERE name = $1)",
            &[&account_name],
        )
        .await
        .expect("mark subscription paid");
    assert_eq!(updated, 1, "{account_name} has one open period");
}

pub fn unique_slug(tag: &str) -> String {
    static RUN_ID: OnceLock<u128> = OnceLock::new();
    static COUNTER: AtomicU32 = AtomicU32::new(0);

    let run_id = RUN_ID.get_or_init(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after the unix epoch")
            .as_micros()
    });
    let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("e2e{tag}{run_id:x}{sequence}")
}

pub async fn assert_rejected(
    server: &TestServer,
    signup: &NewSignup,
    expected_error: &str,
) -> io::Result<()> {
    let response = server.post_json(SIGNUP_PATH, &signup.body(), &[]).await?;
    assert_eq!(response.status_code(), 400, "{}", response.body_str());
    assert_eq!(response.json::<Value>()["error"], json!(expected_error));
    assert!(
        fetch_account(server, &signup.account_name).await.is_none(),
        "a rejected signup must not create an account"
    );
    Ok(())
}

#[derive(Debug)]
pub struct AccountRow {
    pub id: String,
    pub plan: String,
    pub events_monthly_limit: Option<i32>,
    pub trial_ends_at_is_null: bool,
    pub billing_period: String,
    pub regular_price_cents: i32,
    pub actual_price_cents: i32,
    pub discount_ends_at_is_null: bool,
}

pub async fn fetch_account(server: &TestServer, account_name: &str) -> Option<AccountRow> {
    let conn = main_db(server).await;
    let row = conn
        .query_opt(
            "SELECT a.id::text, sub.plan_type::text, sub.events_monthly_limit,
                    a.trial_ends_at IS NULL,
                    sub.billing_period::text, sub.regular_price_cents, sub.actual_price_cents,
                    sub.discount_ends_at IS NULL
             FROM accounts a
             INNER JOIN account_subscriptions sub
                ON sub.account_id = a.id AND sub.ended_at IS NULL
             WHERE a.name = $1",
            &[&account_name],
        )
        .await
        .expect("query account")?;
    Some(AccountRow {
        id: row.get(0),
        plan: row.get(1),
        events_monthly_limit: row.get(2),
        trial_ends_at_is_null: row.get(3),
        billing_period: row.get(4),
        regular_price_cents: row.get(5),
        actual_price_cents: row.get(6),
        discount_ends_at_is_null: row.get(7),
    })
}

/// Remove what a test wrote to the shared main database. Projects, users,
/// and sessions cascade from the account.
pub async fn delete_accounts(server: &TestServer, account_names: &[&str]) {
    let conn = revscope::db::open_main_database_write(server.context.clone())
        .await
        .expect("open main database");
    for account_name in account_names {
        conn.execute("DELETE FROM accounts WHERE name = $1", &[account_name])
            .await
            .expect("delete test account");
    }
}

pub async fn main_db(server: &TestServer) -> revscope::db::main::Connection {
    revscope::db::open_main_database_read(server.context.clone())
        .await
        .expect("open main database")
}
