// Project limits come from the account's plan, which these tests set directly
// so they run without the billing module.
#![cfg(not(feature = "self-hosted"))]

//! Adding and deleting projects through `/v2/session/projects`.

mod common;

use std::io;
use std::time::{SystemTime, UNIX_EPOCH};

use common::{DEMO_PROJECT_ID, HttpResponse, TestServer, TrackHit, page_id};
use serde_json::{Value, json};

const SIGNUP_PATH: &str = "/v2/public/auth/signup";
const CREATE_PATH: &str = "/v2/session/projects";
const ME_PATH: &str = "/v2/session/me";

fn overview_path(project_id: &str) -> String {
    let end = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after the unix epoch")
        .as_secs();
    let start = end - 30 * 24 * 60 * 60;
    format!("/v2/read/projects/{project_id}/overview?start={start}&end={end}")
}

fn project_path(project_id: &str) -> String {
    format!("/v2/session/projects/{project_id}")
}

struct Account {
    account_name: String,
    project_name: String,
    project_id: String,
    cookie: String,
}

#[tokio::test]
async fn a_growth_account_adds_projects_and_deletes_all_but_the_last() -> io::Result<()> {
    let server = TestServer::spawn("projects-growth").await?;
    let account = signup(&server, "projgrowth", "growth").await?;
    let cookie = [("Cookie", account.cookie.as_str())];

    let website = format!("https://Shop.{}/pricing", account.account_name);
    let created = server
        .post_json(
            CREATE_PATH,
            &json!({ "website": website, "timezone": "Europe/Berlin" }).to_string(),
            &cookie,
        )
        .await?;
    assert_eq!(created.status_code(), 200, "{}", created.body_str());
    let created: Value = created.json();
    let new_name = format!("shop{}", account.project_name);
    assert_eq!(created["name"], json!(new_name));
    let new_id = created["id"].as_str().expect("id").to_string();

    let projects = me_projects(&server, &account.cookie).await?;
    assert_eq!(projects.len(), 2, "{projects:?}");
    assert_eq!(projects[&new_name]["id"], json!(new_id));
    assert_eq!(
        projects[&new_name]["settings"]["timezone"],
        json!("Europe/Berlin")
    );

    // The name is the ingest path, so a second project can't share it.
    let duplicate = server
        .post_json(
            CREATE_PATH,
            &json!({ "website": website }).to_string(),
            &cookie,
        )
        .await?;
    assert_eq!(duplicate.status_code(), 409, "{}", duplicate.body_str());

    // A chosen name gets past that, and is checked like signup's.
    let invalid = server
        .post_json(
            CREATE_PATH,
            &json!({ "website": website, "project_name": "Not-Valid" }).to_string(),
            &cookie,
        )
        .await?;
    assert_eq!(invalid.status_code(), 400, "{}", invalid.body_str());
    let chosen_name = format!("{new_name}_two");
    let chosen = server
        .post_json(
            CREATE_PATH,
            &json!({ "website": website, "project_name": format!(" {chosen_name} ") }).to_string(),
            &cookie,
        )
        .await?;
    assert_eq!(chosen.status_code(), 200, "{}", chosen.body_str());
    let chosen: Value = chosen.json();
    assert_eq!(chosen["name"], json!(chosen_name));
    let chosen_id = chosen["id"].as_str().expect("id");
    let removed = delete(&server, chosen_id, &chosen_name, &account.cookie).await?;
    assert_eq!(removed.status_code(), 200, "{}", removed.body_str());

    // The new project ingests and reads like the first.
    let hit = TrackHit::pageview(&new_name, "/", &page_id(1));
    let ingest = server
        .post_ingest_for(&new_name, &hit.to_form(), &[])
        .await?;
    assert_eq!(ingest.status_code(), 200, "{}", ingest.body_str());
    server.flush().await;
    let overview = server.get_query(&overview_path(&new_id), &cookie).await?;
    assert_eq!(overview.status_code(), 200, "{}", overview.body_str());

    let misnamed = delete(&server, &new_id, &account.project_name, &account.cookie).await?;
    assert_eq!(misnamed.status_code(), 400, "{}", misnamed.body_str());
    assert_eq!(me_projects(&server, &account.cookie).await?.len(), 2);

    let deleted = delete(&server, &new_id, &new_name, &account.cookie).await?;
    assert_eq!(deleted.status_code(), 200, "{}", deleted.body_str());
    let projects = me_projects(&server, &account.cookie).await?;
    assert_eq!(projects.len(), 1, "{projects:?}");
    assert!(projects.get(&account.project_name).is_some());

    // Gone for reads and ingest too, not just from the account's list.
    let overview = server.get_query(&overview_path(&new_id), &cookie).await?;
    assert_ne!(overview.status_code(), 200, "{}", overview.body_str());
    let ingest = server
        .post_ingest_for(&new_name, &hit.to_form(), &[])
        .await?;
    assert_ne!(ingest.status_code(), 200, "{}", ingest.body_str());

    let last = delete(
        &server,
        &account.project_id,
        &account.project_name,
        &account.cookie,
    )
    .await?;
    assert_eq!(last.status_code(), 409, "{}", last.body_str());
    assert_eq!(me_projects(&server, &account.cookie).await?.len(), 1);

    // The deleted name is the ingest path a site may still embed, so no other
    // account can take it, by adding a project or by signing up.
    let other = signup(&server, "projsquat", "growth").await?;
    let squat = server
        .post_json(
            CREATE_PATH,
            &json!({ "website": new_name }).to_string(),
            &[("Cookie", other.cookie.as_str())],
        )
        .await?;
    assert_eq!(squat.status_code(), 409, "{}", squat.body_str());
    let squat_signup = server
        .post_json(
            SIGNUP_PATH,
            &json!({
                "name": "Squatter",
                "account_name": format!("shop.{}", account.account_name),
                "email": format!("squat{}", other.account_name.replace('.', "@")),
                "password": "a-long-enough-password",
                "plan": "starter",
                "events": "100k",
                "billing": "monthly",
            })
            .to_string(),
            &[],
        )
        .await?;
    assert_eq!(
        squat_signup.status_code(),
        409,
        "{}",
        squat_signup.body_str()
    );
    // The account that deleted it may start the project afresh.
    let reclaimed = server
        .post_json(
            CREATE_PATH,
            &json!({ "website": new_name }).to_string(),
            &cookie,
        )
        .await?;
    assert_eq!(reclaimed.status_code(), 200, "{}", reclaimed.body_str());

    delete_account(&server, &other.account_name).await;
    delete_account(&server, &account.account_name).await;
    Ok(())
}

#[tokio::test]
async fn a_starter_account_keeps_its_one_project() -> io::Result<()> {
    let server = TestServer::spawn("projects-starter").await?;
    let account = signup(&server, "projstarter", "starter").await?;

    let created = server
        .post_json(
            CREATE_PATH,
            &json!({ "website": format!("blog.{}", account.account_name) }).to_string(),
            &[("Cookie", account.cookie.as_str())],
        )
        .await?;
    assert_eq!(created.status_code(), 403, "{}", created.body_str());
    assert_eq!(me_projects(&server, &account.cookie).await?.len(), 1);

    delete_account(&server, &account.account_name).await;
    Ok(())
}

#[tokio::test]
async fn projects_are_only_changed_by_their_own_signed_in_account() -> io::Result<()> {
    let server = TestServer::spawn("projects-authz").await?;
    let owner = signup(&server, "projowner", "growth").await?;
    let other = signup(&server, "projother", "growth").await?;
    let created = server
        .post_json(
            CREATE_PATH,
            &json!({ "website": format!("extra.{}", owner.account_name) }).to_string(),
            &[("Cookie", owner.cookie.as_str())],
        )
        .await?;
    assert_eq!(created.status_code(), 200, "{}", created.body_str());
    let created: Value = created.json();
    let extra_id = created["id"].as_str().expect("id").to_string();
    let extra_name = created["name"].as_str().expect("name").to_string();

    let anonymous = server
        .post_json(
            CREATE_PATH,
            &json!({ "website": "anon.example" }).to_string(),
            &[],
        )
        .await?;
    assert_eq!(anonymous.status_code(), 401, "{}", anonymous.body_str());

    let foreign = delete(&server, &extra_id, &extra_name, &other.cookie).await?;
    assert!(foreign.status_code() == 404, "{}", foreign.body_str());
    let demo = delete(&server, DEMO_PROJECT_ID, "demo", &other.cookie).await?;
    assert!(demo.status_code() == 404, "{}", demo.body_str());
    assert_eq!(me_projects(&server, &owner.cookie).await?.len(), 2);

    for (status, body) in [
        (400, json!({ "website": "x" })),
        (400, json!({ "website": "demo" })),
    ] {
        let response = server
            .post_json(
                CREATE_PATH,
                &body.to_string(),
                &[("Cookie", owner.cookie.as_str())],
            )
            .await?;
        assert_eq!(
            response.status_code(),
            status,
            "{body}: {}",
            response.body_str()
        );
    }

    delete_account(&server, &owner.account_name).await;
    delete_account(&server, &other.account_name).await;
    Ok(())
}

/// Deleting doesn't revoke a download token, and an admin holder is still live.
/// A member scoped to the project loses their access to it too, so their token
/// answers 401, as when an admin narrows their scope.
#[tokio::test]
async fn a_download_token_for_a_deleted_project_answers_not_found() -> io::Result<()> {
    let server = TestServer::spawn_on_disk("projects-delete-download").await?;
    let account = signup(&server, "projdownload", "growth").await?;
    let cookie = [("Cookie", account.cookie.as_str())];
    let website = format!("https://shop.{}", account.account_name);
    let created = server
        .post_json(
            CREATE_PATH,
            &json!({ "website": website }).to_string(),
            &cookie,
        )
        .await?;
    assert_eq!(created.status_code(), 200, "{}", created.body_str());
    let created: Value = created.json();
    let new_id = created["id"].as_str().expect("id").to_string();
    let new_name = created["name"].as_str().expect("name").to_string();
    let hit = TrackHit::pageview(&new_name, "/", &page_id(1));
    let ingest = server
        .post_ingest_for(&new_name, &hit.to_form(), &[])
        .await?;
    assert_eq!(ingest.status_code(), 200, "{}", ingest.body_str());
    server.flush().await;

    let download = server
        .post_json(
            &format!("{}/download-token", project_path(&new_id)),
            "{}",
            &cookie,
        )
        .await?;
    assert_eq!(download.status_code(), 200, "{}", download.body_str());
    let download_path = format!(
        "/v2/download/projects/{new_id}?token={}",
        download.json::<Value>()["token"].as_str().expect("token")
    );
    let live = server.get_query(&download_path, &[]).await?;
    assert_eq!(live.status_code(), 200);

    let deleted = delete(&server, &new_id, &new_name, &account.cookie).await?;
    assert_eq!(deleted.status_code(), 200, "{}", deleted.body_str());

    let gone = server.get_query(&download_path, &[]).await?;
    assert_eq!(gone.status_code(), 404, "{}", gone.body_str());
    assert_eq!(gone.json::<Value>()["error"], json!("project not found"));

    delete_account(&server, &account.account_name).await;
    Ok(())
}

async fn delete(
    server: &TestServer,
    project_id: &str,
    confirm_name: &str,
    cookie: &str,
) -> io::Result<HttpResponse> {
    server
        .delete_json(
            &project_path(project_id),
            &json!({ "confirm_name": confirm_name }).to_string(),
            &[("Cookie", cookie)],
        )
        .await
}

async fn me_projects(
    server: &TestServer,
    cookie: &str,
) -> io::Result<serde_json::Map<String, Value>> {
    let response = server.get_query(ME_PATH, &[("Cookie", cookie)]).await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    let body: Value = response.json();
    Ok(body["projects"].as_object().expect("projects").clone())
}

async fn signup(server: &TestServer, tag: &str, plan: &str) -> io::Result<Account> {
    let slug = format!(
        "{tag}{:x}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after the unix epoch")
            .as_micros()
    );
    let account_name = format!("{slug}.example");
    let project_name = format!("{slug}example");
    let response = server
        .post_json(
            SIGNUP_PATH,
            &json!({
                "name": "Ada Lovelace",
                "account_name": account_name,
                "email": format!("{slug}@example.com"),
                "password": "a-long-enough-password",
                "plan": plan,
                "events": "100k",
                "billing": "monthly",
            })
            .to_string(),
            &[],
        )
        .await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    let body: Value = response.json();
    set_plan(server, &account_name, plan).await;
    let project_id = body["projects"][&project_name]["id"]
        .as_str()
        .expect("project id")
        .to_string();
    let session = response
        .header("set-cookie")
        .expect("set-cookie header")
        .split(';')
        .next()
        .expect("cookie pair")
        .trim()
        .to_string();
    Ok(Account {
        account_name,
        project_name,
        project_id,
        cookie: session,
    })
}

/// Puts the account on `plan` directly: signup only prices plans in a build
/// with the private billing module, and these tests are about project limits.
async fn set_plan(server: &TestServer, account_name: &str, plan: &str) {
    let conn = revscope::db::open_main_database_write(server.context.clone())
        .await
        .expect("open main database");
    let updated = conn
        .execute(
            "UPDATE account_subscriptions SET plan_type = $2::text::plan_type
             WHERE ended_at IS NULL
               AND account_id = (SELECT id FROM accounts WHERE name = $1)",
            &[&account_name, &plan],
        )
        .await
        .expect("set plan");
    assert_eq!(updated, 1, "{account_name} has one open period");
}

async fn delete_account(server: &TestServer, account_name: &str) {
    let conn = revscope::db::open_main_database_write(server.context.clone())
        .await
        .expect("open main database");
    conn.execute("DELETE FROM accounts WHERE name = $1", &[&account_name])
        .await
        .expect("delete test account");
}
