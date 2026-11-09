// Self-hosted builds have no plans to gate on.
#![cfg(not(feature = "self-hosted"))]

//! Custom domains are for Starter and higher plans, refused on the free plan.

mod common;

use std::io;

use common::signup::{NewSignup, SIGNUP_PATH};
use common::{HttpResponse, TestServer};
use serde_json::{Value, json};

struct Account {
    account_name: String,
    project_id: String,
    cookie: String,
}

async fn signup(server: &TestServer, tag: &str) -> io::Result<Account> {
    let new = NewSignup::new(tag);
    let response = server.post_json(SIGNUP_PATH, &new.body(), &[]).await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    let project_id = response.json::<Value>()["projects"]
        .as_object()
        .expect("projects")
        .values()
        .next()
        .expect("a project")["id"]
        .as_str()
        .expect("project id")
        .to_string();
    let cookie = response
        .header("set-cookie")
        .expect("session cookie")
        .split(';')
        .next()
        .expect("cookie value")
        .to_string();
    Ok(Account {
        account_name: new.account_name,
        project_id,
        cookie,
    })
}

async fn set_plan(server: &TestServer, account_name: &str, plan: &str) {
    let conn = revscope::db::open_main_database_write(server.context.clone())
        .await
        .expect("open main database");
    let updated = conn
        .execute(
            "UPDATE account_subscriptions
             SET plan_type = $2::text::plan_type, subscription_status = 'active'
             WHERE ended_at IS NULL
               AND account_id = (SELECT id FROM accounts WHERE name = $1)",
            &[&account_name, &plan],
        )
        .await
        .expect("set plan");
    assert_eq!(updated, 1);
}

async fn delete_account(server: &TestServer, account_name: &str) {
    let conn = revscope::db::open_main_database_write(server.context.clone())
        .await
        .expect("open main database");
    conn.execute("DELETE FROM accounts WHERE name = $1", &[&account_name])
        .await
        .expect("delete test account");
}

async fn save(
    server: &TestServer,
    account: &Account,
    timezone: &str,
    custom_domain: Option<&str>,
) -> io::Result<HttpResponse> {
    server
        .post_json(
            &format!("/v2/session/projects/{}/settings", account.project_id),
            &json!({
                "timezone": timezone,
                "custom_domain": custom_domain,
                "integrations": [],
            })
            .to_string(),
            &[("Cookie", account.cookie.as_str())],
        )
        .await
}

async fn install_server_url(server: &TestServer, account: &Account) -> io::Result<Value> {
    let response = server
        .get_query(
            &format!("/v2/session/projects/{}/install", account.project_id),
            &[("Cookie", account.cookie.as_str())],
        )
        .await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    Ok(response.json::<Value>()["server_url"].clone())
}

fn domain_for(account: &Account, label: &str) -> String {
    format!("{label}-{}.example.com", account.project_id)
}

#[tokio::test]
async fn the_free_plan_cannot_set_a_custom_domain_and_nothing_else_is_saved() -> io::Result<()> {
    let server = TestServer::spawn_on_disk("custom-domain-free").await?;
    let account = signup(&server, "cdfree").await?;
    set_plan(&server, &account.account_name, "free").await;

    let refused = save(
        &server,
        &account,
        "Europe/Paris",
        Some(&domain_for(&account, "assets")),
    )
    .await?;

    assert_eq!(refused.status_code(), 403, "{}", refused.body_str());
    let settings = server
        .get_query(
            &format!("/v2/session/projects/{}/settings", account.project_id),
            &[("Cookie", account.cookie.as_str())],
        )
        .await?
        .json::<Value>();
    assert_eq!(settings["custom_domain"], Value::Null);
    assert_ne!(settings["timezone"], "Europe/Paris");
    delete_account(&server, &account.account_name).await;
    Ok(())
}

#[tokio::test]
async fn the_starter_plan_can_set_a_custom_domain() -> io::Result<()> {
    let server = TestServer::spawn_on_disk("custom-domain-starter").await?;
    let account = signup(&server, "cdstarter").await?;
    set_plan(&server, &account.account_name, "starter").await;
    let domain = domain_for(&account, "assets");

    let saved = save(&server, &account, "UTC", Some(&domain)).await?;

    assert_eq!(saved.status_code(), 200, "{}", saved.body_str());
    assert_eq!(saved.json::<Value>()["custom_domain"], domain.as_str());
    assert_eq!(
        install_server_url(&server, &account).await?,
        format!("https://{domain}/")
    );
    delete_account(&server, &account.account_name).await;
    Ok(())
}

#[tokio::test]
async fn a_domain_kept_through_a_downgrade_goes_unused_but_blocks_nothing() -> io::Result<()> {
    let server = TestServer::spawn_on_disk("custom-domain-downgrade").await?;
    let account = signup(&server, "cddown").await?;
    set_plan(&server, &account.account_name, "starter").await;
    let domain = domain_for(&account, "assets");
    let saved = save(&server, &account, "UTC", Some(&domain)).await?;
    assert_eq!(saved.status_code(), 200, "{}", saved.body_str());
    set_plan(&server, &account.account_name, "free").await;

    assert_eq!(install_server_url(&server, &account).await?, Value::Null);

    let unchanged = save(&server, &account, "Europe/Paris", Some(&domain)).await?;
    assert_eq!(unchanged.status_code(), 200, "{}", unchanged.body_str());
    assert_eq!(unchanged.json::<Value>()["timezone"], "Europe/Paris");

    let changed = save(
        &server,
        &account,
        "Europe/Paris",
        Some(&domain_for(&account, "other")),
    )
    .await?;
    assert_eq!(changed.status_code(), 403, "{}", changed.body_str());

    let cleared = save(&server, &account, "Europe/Paris", None).await?;
    assert_eq!(cleared.status_code(), 200, "{}", cleared.body_str());
    assert_eq!(cleared.json::<Value>()["custom_domain"], Value::Null);
    delete_account(&server, &account.account_name).await;
    Ok(())
}

#[tokio::test]
async fn a_starter_plan_that_was_never_paid_for_is_refused() -> io::Result<()> {
    let server = TestServer::spawn_on_disk("custom-domain-unpaid").await?;
    let account = signup(&server, "cdunpaid").await?;
    set_plan(&server, &account.account_name, "starter").await;
    let conn = revscope::db::open_main_database_write(server.context.clone())
        .await
        .expect("open main database");
    conn.execute(
        "UPDATE account_subscriptions SET subscription_status = NULL
         WHERE ended_at IS NULL
           AND account_id = (SELECT id FROM accounts WHERE name = $1)",
        &[&account.account_name],
    )
    .await
    .expect("clear status");

    let refused = save(
        &server,
        &account,
        "UTC",
        Some(&domain_for(&account, "assets")),
    )
    .await?;

    assert_eq!(refused.status_code(), 403, "{}", refused.body_str());
    delete_account(&server, &account.account_name).await;
    Ok(())
}
