// A self-hosted server has one account, so there is nothing to list or delete.
#![cfg(not(feature = "self-hosted"))]

//! The site admin's view of every account, through `/v2/session/site-admin/…`.

mod common;

use std::io;

use common::signup::{NewSignup, SIGNUP_PATH, delete_accounts, main_db, mark_email_verified};
use common::{HttpResponse, TestServer, TrackHit, page_id};
use serde_json::{Value, json};

const ACCOUNTS_PATH: &str = "/v2/session/site-admin/accounts";
const ME_PATH: &str = "/v2/session/me";

struct SignedUp {
    signup: NewSignup,
    account_id: String,
    cookie: String,
}

#[tokio::test]
async fn the_site_admin_deletes_an_account_with_its_users_projects_and_databases() -> io::Result<()>
{
    let (server, admin) = site_admin_server("site-admin-delete").await?;
    let customer = sign_up(&server, "sadelcust").await?;
    let customer_cookie = [("Cookie", customer.cookie.as_str())];
    paid_growth(&server, &customer.account_id).await;
    let created = server
        .post_json(
            "/v2/session/projects",
            &json!({ "website": format!("https://shop.{}", customer.signup.account_name) })
                .to_string(),
            &customer_cookie,
        )
        .await?;
    assert_eq!(created.status_code(), 200, "{}", created.body_str());
    let created: Value = created.json();
    let projects = [
        (
            me(&server, &customer.cookie).await?["projects"][&customer.signup.project_name]["id"]
                .as_str()
                .expect("first project id")
                .to_string(),
            customer.signup.project_name.clone(),
        ),
        (
            created["id"].as_str().expect("id").to_string(),
            created["name"].as_str().expect("name").to_string(),
        ),
    ];
    for (_, name) in &projects {
        let hit = TrackHit::pageview(name, "/", &page_id(1));
        let ingest = server.post_ingest_for(name, &hit.to_form(), &[]).await?;
        assert_eq!(ingest.status_code(), 200, "{}", ingest.body_str());
    }
    server.flush().await;
    let database = |id: &str| revscope::db::project_database_path(server.context.as_ref(), id);
    for (id, _) in &projects {
        assert!(database(id).exists(), "project {id} has a database");
    }

    let listed = find_account(&server, &admin.cookie, &customer.account_id).await?;
    assert_eq!(listed["name"], json!(customer.signup.account_name));
    assert_eq!(listed["is_own"], json!(false));
    assert!(listed["signed_up_at"].as_i64().is_some(), "{listed}");
    assert_eq!(listed["subscription"]["plan"], json!("growth"));
    assert_eq!(listed["subscription"]["status"], json!("active"));
    let names: Vec<&str> = listed["projects"]
        .as_array()
        .expect("projects")
        .iter()
        .map(|project| project["name"].as_str().expect("name"))
        .collect();
    let mut expected: Vec<&str> = projects.iter().map(|(_, name)| name.as_str()).collect();
    expected.sort();
    assert_eq!(names, expected);
    for project in listed["projects"].as_array().expect("projects") {
        assert_eq!(
            project["last_30_days"],
            json!({ "visitors": 1, "revenue": 0 }),
            "{project}"
        );
    }
    let user = &listed["users"][0];
    assert_eq!(user["email"], json!(customer.signup.email));
    assert_eq!(user["is_admin"], json!(true));
    assert_eq!(user["invited"], json!(false));
    assert!(user["signed_up_at"].as_i64().is_some(), "{user}");
    assert_eq!(user["email_verified_at"], Value::Null);
    mark_email_verified(&server, &customer.signup.email).await;
    let listed = find_account(&server, &admin.cookie, &customer.account_id).await?;
    assert!(
        listed["users"][0]["email_verified_at"].as_i64().is_some(),
        "{listed}"
    );

    let misnamed = delete(&server, &admin.cookie, &customer.account_id, "wrong").await?;
    assert_eq!(misnamed.status_code(), 400, "{}", misnamed.body_str());
    let own = delete(
        &server,
        &admin.cookie,
        &admin.account_id,
        &admin.signup.account_name,
    )
    .await?;
    assert_eq!(own.status_code(), 409, "{}", own.body_str());

    let deleted = delete(
        &server,
        &admin.cookie,
        &customer.account_id,
        &customer.signup.account_name,
    )
    .await?;
    assert_eq!(deleted.status_code(), 200, "{}", deleted.body_str());
    assert_eq!(
        deleted.json::<Value>(),
        json!({
            "deleted": customer.account_id,
            "users_deleted": 1,
            "projects_deleted": 2,
        })
    );

    let signed_out = server.get_query(ME_PATH, &customer_cookie).await?;
    assert_eq!(signed_out.status_code(), 401, "{}", signed_out.body_str());
    for (id, _) in &projects {
        assert!(!database(id).exists(), "project {id}'s database is gone");
    }
    let conn = main_db(&server).await;
    let left: i64 = conn
        .query_one(
            "SELECT (SELECT COUNT(*) FROM accounts WHERE id = $1::text::uuid)
                  + (SELECT COUNT(*) FROM users WHERE account_id = $1::text::uuid)
                  + (SELECT COUNT(*) FROM projects WHERE account_id = $1::text::uuid)",
            &[&customer.account_id],
        )
        .await
        .expect("count what's left")
        .get(0);
    assert_eq!(left, 0);
    // Sites may still embed the projects' tracker, so no account may take
    // their names, not even one signing up again with the same website.
    let names: Vec<String> = projects.iter().map(|(_, name)| name.clone()).collect();
    let retired: i64 = conn
        .query_one(
            "SELECT COUNT(*) FROM retired_project_names
             WHERE name = ANY($1) AND account_id IS NULL",
            &[&names],
        )
        .await
        .expect("count retired names")
        .get(0);
    assert_eq!(retired, 2);
    let again = server
        .post_json(SIGNUP_PATH, &customer.signup.body(), &[])
        .await?;
    assert_eq!(again.status_code(), 409, "{}", again.body_str());

    let gone = delete(
        &server,
        &admin.cookie,
        &customer.account_id,
        &customer.signup.account_name,
    )
    .await?;
    assert_eq!(gone.status_code(), 404, "{}", gone.body_str());

    conn.execute(
        "DELETE FROM retired_project_names WHERE name = ANY($1)",
        &[&names],
    )
    .await
    .expect("clean up retired names");
    delete_accounts(&server, &[&admin.signup.account_name]).await;
    Ok(())
}

#[tokio::test]
async fn an_account_a_provider_may_still_charge_is_kept() -> io::Result<()> {
    let (server, admin) = site_admin_server("site-admin-subscribed").await?;
    let customer = sign_up(&server, "sasubcust").await?;
    let conn = main_db(&server).await;
    let try_delete = async || {
        delete(
            &server,
            &admin.cookie,
            &customer.account_id,
            &customer.signup.account_name,
        )
        .await
    };
    let set_open_status = async |status: &str| {
        // A Stripe customer that moved to Polar keeps its Stripe customer id.
        conn.execute(
            "UPDATE account_subscriptions
             SET stripe_customer_id = 'cus_test', polar_subscription_id = 'polar_sub_test',
                 subscription_status = $2
             WHERE account_id = $1::text::uuid AND ended_at IS NULL",
            &[&customer.account_id, &status],
        )
        .await
        .expect("set subscription");
    };

    set_open_status("active").await;
    // Listing reports a missing database as unknown rather than creating it.
    let project_id = me(&server, &customer.cookie).await?["projects"]
        [&customer.signup.project_name]["id"]
        .as_str()
        .expect("project id")
        .to_string();
    let database = revscope::db::project_database_path(server.context.as_ref(), &project_id);
    if database.exists() {
        std::fs::remove_file(&database)?;
    }
    let listed = find_account(&server, &admin.cookie, &customer.account_id).await?;
    assert_eq!(
        listed["projects"][0]["last_30_days"],
        Value::Null,
        "{listed}"
    );
    assert!(!database.exists(), "listing created {}", database.display());
    assert_eq!(listed["subscription"]["provider"], json!("polar"));
    assert_eq!(
        listed["subscription"]["subscription_id"],
        json!("polar_sub_test")
    );
    assert_eq!(listed["subscription"]["customer_id"], Value::Null);
    let refused = try_delete().await?;
    assert_eq!(refused.status_code(), 409, "{}", refused.body_str());

    // An earlier period that ended unpaid, whose provider the billing sweep
    // hasn't yet stopped collecting.
    set_open_status("canceled").await;
    conn.execute(
        "INSERT INTO account_subscriptions (
            account_id, plan_type, billing_period, regular_price_cents, actual_price_cents,
            stripe_subscription_id, subscription_status, ended_unpaid, started_at, ended_at
         )
         VALUES (
            $1::text::uuid, 'growth', 'monthly', 2900, 2900,
            'sub_unpaid_test', 'unpaid', TRUE,
            NOW() - INTERVAL '2 months', NOW() - INTERVAL '1 month'
         )",
        &[&customer.account_id],
    )
    .await
    .expect("insert unpaid period");
    let refused = try_delete().await?;
    assert_eq!(refused.status_code(), 409, "{}", refused.body_str());

    conn.execute(
        "UPDATE account_subscriptions SET subscription_status = 'canceled'
         WHERE account_id = $1::text::uuid AND ended_unpaid",
        &[&customer.account_id],
    )
    .await
    .expect("stop collecting");
    let deleted = try_delete().await?;
    assert_eq!(deleted.status_code(), 200, "{}", deleted.body_str());

    conn.execute(
        "DELETE FROM retired_project_names WHERE name = $1",
        &[&customer.signup.project_name],
    )
    .await
    .expect("clean up retired name");
    delete_accounts(&server, &[&admin.signup.account_name]).await;
    Ok(())
}

#[tokio::test]
async fn only_the_site_admin_sees_every_account() -> io::Result<()> {
    let (server, admin) = site_admin_server("site-admin-only").await?;
    let customer = sign_up(&server, "saonlycust").await?;

    assert_eq!(
        me(&server, &admin.cookie).await?["is_site_admin"],
        json!(true)
    );
    assert_eq!(
        me(&server, &customer.cookie).await?.get("is_site_admin"),
        None
    );

    let page = list(&server, &admin.cookie, 0).await?;
    assert_eq!(page["page_size"], json!(100));
    assert!(page["total"].as_i64().expect("total") >= 2, "{page}");
    // Newest first: the customer signed up after the admin. Only accounts made
    // since, by tests running alongside, can come before them, so both are on
    // the first page.
    let ids: Vec<&str> = page["accounts"]
        .as_array()
        .expect("accounts")
        .iter()
        .map(|account| account["account_id"].as_str().expect("id"))
        .collect();
    let position = |id: &str| {
        ids.iter()
            .position(|listed| *listed == id)
            .unwrap_or_else(|| panic!("{id} isn't on the first page: {ids:?}"))
    };
    assert!(
        position(&customer.account_id) < position(&admin.account_id),
        "{ids:?}"
    );
    // Accounts the first migration seeded have no signup time and come after
    // every account that has one.
    let signed_up: Vec<Option<i64>> = page["accounts"]
        .as_array()
        .expect("accounts")
        .iter()
        .map(|account| account["signed_up_at"].as_i64())
        .collect();
    assert!(
        signed_up.windows(2).all(|pair| match (pair[0], pair[1]) {
            (Some(newer), Some(older)) => newer >= older,
            (None, Some(_)) => false,
            _ => true,
        }),
        "{signed_up:?}"
    );
    let bad_offset = server
        .get_query(
            &format!("{ACCOUNTS_PATH}?offset=-1"),
            &[("Cookie", admin.cookie.as_str())],
        )
        .await?;
    assert_eq!(bad_offset.status_code(), 400, "{}", bad_offset.body_str());

    let customer_cookie = [("Cookie", customer.cookie.as_str())];
    let listed = server.get_query(ACCOUNTS_PATH, &customer_cookie).await?;
    assert_eq!(listed.status_code(), 404, "{}", listed.body_str());
    let deleted = delete(
        &server,
        &customer.cookie,
        &admin.account_id,
        &admin.signup.account_name,
    )
    .await?;
    assert_eq!(deleted.status_code(), 404, "{}", deleted.body_str());
    let anonymous = server.get_query(ACCOUNTS_PATH, &[]).await?;
    assert_eq!(anonymous.status_code(), 401, "{}", anonymous.body_str());

    // This server still caches the admin's session as an admin's, as another
    // process would after a demotion, so only the role in Postgres refuses it.
    main_db(&server)
        .await
        .execute(
            "UPDATE users SET is_admin = FALSE WHERE account_id = $1::text::uuid",
            &[&admin.account_id],
        )
        .await
        .expect("demote the admin");
    let demoted = server
        .get_query(ACCOUNTS_PATH, &[("Cookie", admin.cookie.as_str())])
        .await?;
    assert_eq!(demoted.status_code(), 404, "{}", demoted.body_str());
    let demoted = delete(
        &server,
        &admin.cookie,
        &customer.account_id,
        &customer.signup.account_name,
    )
    .await?;
    assert_eq!(demoted.status_code(), 404, "{}", demoted.body_str());

    delete_accounts(
        &server,
        &[&admin.signup.account_name, &customer.signup.account_name],
    )
    .await;
    Ok(())
}

/// A server whose site admin account is a fresh signup. The account has to
/// exist before the server that names it starts, so a first server signs it up.
async fn site_admin_server(label: &str) -> io::Result<(TestServer, SignedUp)> {
    let setup = TestServer::spawn(&format!("{label}-setup")).await?;
    let admin = sign_up(&setup, &label.replace('-', "")).await?;
    let server = TestServer::spawn_with_site_admin(label, &admin.account_id).await?;
    Ok((server, admin))
}

async fn sign_up(server: &TestServer, tag: &str) -> io::Result<SignedUp> {
    let signup = NewSignup::new(tag);
    let response = server.post_json(SIGNUP_PATH, &signup.body(), &[]).await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    let account_id = response.json::<Value>()["account_id"]
        .as_str()
        .expect("account id")
        .to_string();
    let cookie = response
        .header("set-cookie")
        .expect("set-cookie header")
        .split(';')
        .next()
        .expect("cookie pair")
        .trim()
        .to_string();
    Ok(SignedUp {
        signup,
        account_id,
        cookie,
    })
}

/// A paid Growth plan, which may hold more than one project. Set directly,
/// since signup only prices plans in a build with the private billing module.
async fn paid_growth(server: &TestServer, account_id: &str) {
    let conn = revscope::db::open_main_database_write(server.context.clone())
        .await
        .expect("open main database");
    conn.execute(
        "UPDATE account_subscriptions
         SET plan_type = 'growth', subscription_status = 'active'
         WHERE ended_at IS NULL AND account_id = $1::text::uuid",
        &[&account_id],
    )
    .await
    .expect("set plan");
}

async fn me(server: &TestServer, cookie: &str) -> io::Result<Value> {
    let response = server.get_query(ME_PATH, &[("Cookie", cookie)]).await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    Ok(response.json())
}

async fn list(server: &TestServer, cookie: &str, offset: i64) -> io::Result<Value> {
    let response = server
        .get_query(
            &format!("{ACCOUNTS_PATH}?offset={offset}"),
            &[("Cookie", cookie)],
        )
        .await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    Ok(response.json())
}

/// The account's row, from whichever page lists it. Other tests sign up
/// accounts at the same time, so it may not be on the first.
async fn find_account(server: &TestServer, cookie: &str, account_id: &str) -> io::Result<Value> {
    let mut offset = 0;
    loop {
        let page = list(server, cookie, offset).await?;
        let accounts = page["accounts"].as_array().expect("accounts");
        if let Some(account) = accounts
            .iter()
            .find(|account| account["account_id"] == json!(account_id))
        {
            return Ok(account.clone());
        }
        assert!(!accounts.is_empty(), "{account_id} is on no page");
        offset += accounts.len() as i64;
    }
}

async fn delete(
    server: &TestServer,
    cookie: &str,
    account_id: &str,
    confirm_name: &str,
) -> io::Result<HttpResponse> {
    server
        .delete_json(
            ACCOUNTS_PATH,
            &json!({ "account_id": account_id, "confirm_name": confirm_name }).to_string(),
            &[("Cookie", cookie)],
        )
        .await
}
