// Self-hosted builds have no plans to gate on.
#![cfg(not(feature = "self-hosted"))]

//! Invites email text the inviter wrote, so they need a paid plan in good
//! standing and are limited per account.

mod common;

use std::io;

use common::signup::{NewSignup, SIGNUP_PATH};
use common::{HttpResponse, TestServer};
use serde_json::{Value, json};

const INVITE_PATH: &str = "/v2/session/users/invite";
const USERS_PATH: &str = "/v2/session/users";

struct Account {
    account_name: String,
    email: String,
    cookie: String,
}

async fn signup(server: &TestServer, tag: &str) -> io::Result<Account> {
    let new = NewSignup::new(tag);
    let response = server.post_json(SIGNUP_PATH, &new.body(), &[]).await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    let cookie = response
        .header("set-cookie")
        .expect("session cookie")
        .split(';')
        .next()
        .expect("cookie value")
        .to_string();
    Ok(Account {
        account_name: new.account_name,
        email: new.email,
        cookie,
    })
}

async fn set_plan(server: &TestServer, account: &Account, plan: &str, status: Option<&str>) {
    let conn = revscope::db::open_main_database_write(server.context.clone())
        .await
        .expect("open main database");
    let updated = conn
        .execute(
            "UPDATE account_subscriptions
             SET plan_type = $2::text::plan_type, subscription_status = $3
             WHERE ended_at IS NULL
               AND account_id = (SELECT id FROM accounts WHERE name = $1)",
            &[&account.account_name, &plan, &status],
        )
        .await
        .expect("set plan");
    assert_eq!(updated, 1);
}

async fn delete_account(server: &TestServer, account: &Account) {
    let conn = revscope::db::open_main_database_write(server.context.clone())
        .await
        .expect("open main database");
    conn.execute(
        "DELETE FROM accounts WHERE name = $1",
        &[&account.account_name],
    )
    .await
    .expect("delete test account");
}

async fn invite(server: &TestServer, account: &Account, email: &str) -> io::Result<HttpResponse> {
    server
        .post_json(
            INVITE_PATH,
            &json!({
                "name": "Grace Hopper",
                "email": email,
                "is_admin": false,
                "project_ids": null,
            })
            .to_string(),
            &[("Cookie", account.cookie.as_str())],
        )
        .await
}

async fn users(server: &TestServer, account: &Account) -> io::Result<Value> {
    let response = server
        .get_query(USERS_PATH, &[("Cookie", account.cookie.as_str())])
        .await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    Ok(response.json::<Value>())
}

#[tokio::test]
async fn only_a_paid_plan_in_good_standing_can_invite() -> io::Result<()> {
    let server = TestServer::spawn("invite-plans").await?;
    let account = signup(&server, "inviteplans").await?;
    let invitee = format!("invitee-{}", account.email);

    for (plan, status) in [
        ("free", None),
        // What choosing a plan on /pricing leaves behind before any payment.
        ("growth", None),
        ("starter", Some("canceled")),
    ] {
        set_plan(&server, &account, plan, status).await;
        let refused = invite(&server, &account, &invitee).await?;
        assert_eq!(
            refused.status_code(),
            403,
            "{plan} {status:?}: {}",
            refused.body_str()
        );
        let listed = users(&server, &account).await?;
        assert_eq!(
            listed["invites_included"],
            json!(false),
            "{plan} {status:?}"
        );
        assert_eq!(listed["users"].as_array().expect("users").len(), 1);
    }

    set_plan(&server, &account, "starter", Some("active")).await;
    let accepted = invite(&server, &account, &invitee).await?;
    assert_eq!(accepted.status_code(), 200, "{}", accepted.body_str());
    let listed = users(&server, &account).await?;
    assert_eq!(listed["invites_included"], json!(true));
    assert_eq!(listed["users"].as_array().expect("users").len(), 2);

    delete_account(&server, &account).await;
    Ok(())
}

#[tokio::test]
async fn invites_are_limited_per_account_and_refused_ones_cost_nothing() -> io::Result<()> {
    let server = TestServer::spawn("invite-rate").await?;
    let limited = signup(&server, "inviterate").await?;
    let other = signup(&server, "inviteother").await?;
    set_plan(&server, &limited, "starter", Some("active")).await;
    set_plan(&server, &other, "starter", Some("active")).await;

    // Inviting someone who already has an account is refused and sends nothing.
    for _ in 0..30 {
        let refused = invite(&server, &limited, &limited.email).await?;
        assert_eq!(refused.status_code(), 409, "{}", refused.body_str());
    }

    let mut accepted = 0;
    let rate_limited = loop {
        assert!(accepted < 100, "no limit reached after {accepted} invites");
        let email = format!("invitee{accepted}-{}", limited.email);
        let response = invite(&server, &limited, &email).await?;
        if response.status_code() != 200 {
            break response;
        }
        accepted += 1;
    };
    assert_eq!(accepted, 20);
    assert_eq!(
        rate_limited.status_code(),
        429,
        "{}",
        rate_limited.body_str()
    );
    let retry_after: u64 = rate_limited
        .header("retry-after")
        .expect("Retry-After")
        .parse()
        .expect("whole seconds");
    assert!(retry_after > 0);

    let email = format!("invitee-{}", other.email);
    let unaffected = invite(&server, &other, &email).await?;
    assert_eq!(unaffected.status_code(), 200, "{}", unaffected.body_str());

    delete_account(&server, &limited).await;
    delete_account(&server, &other).await;
    Ok(())
}
