// Hosted signup is open, and hosted plans have a retention.
#![cfg(not(feature = "self-hosted"))]

//! The dashboard shows only what the account's plan keeps, whatever range it asks for.

mod common;

use std::io;
use std::time::{SystemTime, UNIX_EPOCH};

use common::TestServer;
use serde_json::{Value, json};

const SIGNUP_PATH: &str = "/v2/public/auth/signup";
const PASSWORD: &str = "a-long-enough-password";
const DAY: i64 = 86_400;

struct Account {
    project_id: String,
    cookie: String,
}

async fn free_account(server: &TestServer) -> io::Result<Account> {
    let slug = format!(
        "retention{:x}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_micros()
    );
    let email = format!("{slug}@example.com");
    let signup = server
        .post_json(
            SIGNUP_PATH,
            &json!({
                "name": "Ada Lovelace",
                "account_name": format!("{slug}.example"),
                "email": email,
                "password": PASSWORD,
                "plan": "free",
                "events": "20k",
                "billing": "monthly",
            })
            .to_string(),
            &[],
        )
        .await?;
    assert_eq!(signup.status_code(), 200, "{}", signup.body_str());
    let body: Value = signup.json();
    let project_id = body["projects"]
        .as_object()
        .and_then(|projects| projects.values().next())
        .and_then(|project| project["id"].as_str())
        .expect("a project")
        .to_string();

    // A build without the billing module signs everyone up unlimited.
    let conn = revscope::db::open_main_database_write(server.context.clone())
        .await
        .expect("open main database");
    conn.execute(
        "UPDATE account_subscriptions SET plan_type = 'free'
         WHERE account_id = $1::text::uuid AND ended_at IS NULL",
        &[&body["account_id"].as_str().expect("account id")],
    )
    .await
    .expect("move to the free plan");

    // Logging in again reads the plan fresh rather than from the session cache.
    let login = server
        .post_json(
            "/v2/public/auth/login",
            &json!({ "username": email, "password": PASSWORD }).to_string(),
            &[],
        )
        .await?;
    assert_eq!(login.status_code(), 200, "{}", login.body_str());
    assert_eq!(login.json::<Value>()["data_retention_months"], json!(3));
    let cookie = login
        .header("set-cookie")
        .and_then(|value| value.split(';').next())
        .expect("session cookie")
        .to_string();
    Ok(Account { project_id, cookie })
}

fn three_months_ago(now: i64) -> i64 {
    let today = jiff::Timestamp::from_second(now)
        .expect("timestamp")
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date();
    today
        .checked_sub(jiff::ToSpan::months(3))
        .expect("earlier")
        .to_zoned(jiff::tz::TimeZone::UTC)
        .expect("zoned")
        .timestamp()
        .as_second()
}

#[tokio::test]
async fn reports_start_no_earlier_than_the_plan_keeps() -> io::Result<()> {
    let server = TestServer::spawn("plan-retention").await?;
    let account = free_account(&server).await?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_secs() as i64;
    let today = now - now % DAY;
    let overview = |start: i64| {
        format!(
            "/v2/read/projects/{}/overview?start={start}&end={}&bucket=daily",
            account.project_id,
            today + DAY
        )
    };

    let response = server
        .get_query(
            &overview(today - 300 * DAY),
            &[("Cookie", account.cookie.as_str())],
        )
        .await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    let body: Value = response.json();
    assert_eq!(
        body["buckets"][0]["start"],
        json!(three_months_ago(now)),
        "the range starts where retention does"
    );

    let inside = server
        .get_query(
            &overview(today - 7 * DAY),
            &[("Cookie", account.cookie.as_str())],
        )
        .await?;
    assert_eq!(inside.status_code(), 200, "{}", inside.body_str());
    assert_eq!(
        inside.json::<Value>()["buckets"][0]["start"],
        json!(today - 7 * DAY)
    );

    let too_old = format!(
        "/v2/read/projects/{}/pages?start={}&end={}",
        account.project_id,
        today - 300 * DAY,
        today - 200 * DAY
    );
    let refused = server
        .get_query(&too_old, &[("Cookie", account.cookie.as_str())])
        .await?;
    assert_eq!(refused.status_code(), 400, "{}", refused.body_str());
    assert!(
        refused.body_str().contains("retention"),
        "{}",
        refused.body_str()
    );

    // The public demo belongs to nobody reading it, so it has no limit.
    let demo = server
        .get_query(
            &format!(
                "/v2/read/projects/{}/pages?start={}&end={}",
                common::DEMO_PROJECT_ID,
                today - 300 * DAY,
                today - 200 * DAY
            ),
            &[("Cookie", account.cookie.as_str())],
        )
        .await?;
    assert_ne!(demo.status_code(), 400, "{}", demo.body_str());
    Ok(())
}
