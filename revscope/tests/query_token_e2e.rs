// Hosted signup creates a per-website account with a plan; self-hosted signup does not.
#![cfg(not(feature = "self-hosted"))]

//! The path the MCP server takes: trade a session for a project-scoped query
//! token, then read the schema and run queries with nothing but that token.

mod common;

use std::io;
use std::time::{SystemTime, UNIX_EPOCH};

use common::{DEMO_PROJECT_ID, HttpResponse, TestServer, TrackHit, page_id};
use serde_json::{Value, json};

const SIGNUP_PATH: &str = "/v2/public/auth/signup";
const PASSWORD: &str = "a-long-enough-password";
const THIRTY_DAYS_SECONDS: i64 = 30 * 24 * 60 * 60;

fn query_token_path(project_id: impl std::fmt::Display) -> String {
    format!("/v2/session/projects/{project_id}/query-token")
}

fn query_path(project_id: impl std::fmt::Display) -> String {
    format!("/v2/query/projects/{project_id}/sql")
}

fn schema_path(project_id: impl std::fmt::Display) -> String {
    format!("/v2/query/projects/{project_id}/schema")
}

#[tokio::test]
async fn a_query_token_reads_the_schema_and_paginates_queries_for_its_project() -> io::Result<()> {
    let mut server = TestServer::spawn("query-token").await?;
    let account = signup(&server, "querytoken").await?;

    let minted = server
        .post_json(
            &query_token_path(account.project_id),
            "",
            &[("Cookie", &format!("rr_session={}", account.session_id))],
        )
        .await?;
    assert_eq!(minted.status_code(), 200, "{}", minted.body_str());
    let minted: Value = minted.json();
    let token = minted["token"].as_str().expect("token").to_string();
    assert!(
        token.starts_with("rsq_"),
        "a query token is recognizable in an Authorization header: {token}"
    );
    assert_eq!(minted["project_id"], json!(account.project_id.to_string()));
    // Allow a little slack either side of 30 days: minting takes wall time,
    // and the test clock can tick between the response and this check.
    let lifetime = minted["expires_at"].as_i64().expect("expires_at") - unix_seconds();
    assert!(
        (THIRTY_DAYS_SECONDS - 60..=THIRTY_DAYS_SECONDS + 60).contains(&lifetime),
        "a query token lasts 30 days, got {lifetime} seconds"
    );

    let authorization = format!("Bearer {token}");
    let auth_header = [("Authorization", authorization.as_str())];

    let schema = server
        .get_query(&schema_path(account.project_id), &auth_header)
        .await?;
    assert_eq!(schema.status_code(), 200, "{}", schema.body_str());
    assert!(
        schema.body_str().contains("CREATE TABLE"),
        "{}",
        schema.body_str()
    );

    for (index, path) in ["/pricing", "/signup"].into_iter().enumerate() {
        let hit = TrackHit::pageview(&account.project_name, path, &page_id(index as u8 + 1));
        let ingest = server
            .post_ingest_for(&account.project_name, &hit.to_form(), &[])
            .await?;
        assert_eq!(ingest.status_code(), 200, "{}", ingest.body_str());
    }
    server.flush().await;
    // Keeps the shared-memory project database alive for the rest of the test.
    let _anchor = server.project_db(&account.project_id.to_string());

    let sql = "SELECT path FROM paths ORDER BY path";
    let first_page: Value = server
        .post_json(
            &query_path(account.project_id),
            &query_body(sql, Some(1), None),
            &auth_header,
        )
        .await?
        .json();
    assert_eq!(first_page["columns"], json!(["path"]));
    assert_eq!(first_page["rows"], json!([["/pricing"]]));
    assert_eq!(first_page["limit"], json!(1));
    assert_eq!(first_page["has_more"], json!(true));

    let second_page: Value = server
        .post_json(
            &query_path(account.project_id),
            &query_body(sql, Some(1), Some(1)),
            &auth_header,
        )
        .await?
        .json();
    assert_eq!(second_page["rows"], json!([["/signup"]]));
    assert_eq!(second_page["offset"], json!(1));
    assert_eq!(second_page["has_more"], json!(false));

    delete_account(&server, &account.account_name).await;
    Ok(())
}

#[tokio::test]
async fn a_query_token_reaches_no_other_project_and_nothing_without_one() -> io::Result<()> {
    let server = TestServer::spawn("query-token-scope").await?;
    let account = signup(&server, "querytokenscope").await?;
    let token = mint_token(&server, &account).await?;
    let authorization = format!("Bearer {token}");
    let auth_header = [("Authorization", authorization.as_str())];

    // The token names its project; asking for another one is not authorized,
    // whichever endpoint asks.
    let other_project = server
        .get_query(&schema_path(DEMO_PROJECT_ID), &auth_header)
        .await?;
    assert_eq!(
        other_project.status_code(),
        401,
        "{}",
        other_project.body_str()
    );
    let other_project = server
        .post_json(
            &query_path(DEMO_PROJECT_ID),
            &query_body("SELECT 1", None, None),
            &auth_header,
        )
        .await?;
    assert_eq!(
        other_project.status_code(),
        401,
        "{}",
        other_project.body_str()
    );

    let unknown_token = format!("Bearer rsq_{}", "0".repeat(32));
    let unknown = server
        .get_query(
            &schema_path(account.project_id),
            &[("Authorization", unknown_token.as_str())],
        )
        .await?;
    assert_eq!(unknown.status_code(), 401, "{}", unknown.body_str());
    assert_eq!(
        unknown.json::<Value>()["error"],
        json!("invalid token"),
        "the 401 names the credential the caller actually sent"
    );

    let unauthenticated = server
        .get_query(&schema_path(account.project_id), &[])
        .await?;
    assert_eq!(
        unauthenticated.status_code(),
        401,
        "{}",
        unauthenticated.body_str()
    );

    // Minting one is a session-only operation: a token cannot mint another.
    let self_minted = server
        .post_json(&query_token_path(account.project_id), "", &auth_header)
        .await?;
    assert_eq!(self_minted.status_code(), 401, "{}", self_minted.body_str());

    delete_account(&server, &account.account_name).await;
    Ok(())
}

#[tokio::test]
async fn minting_a_query_token_validates_the_project_and_the_method() -> io::Result<()> {
    let server = TestServer::spawn("query-token-requests").await?;
    let account = signup(&server, "querytokenrequests").await?;
    let cookie = format!("rr_session={}", account.session_id);
    let cookie_header = [("Cookie", cookie.as_str())];

    // Project id lives in the path; incomplete or non-numeric segments do not
    // resolve to a route. Unmatched paths get the bot honeypot (HTTP 200 + gzip HTML).
    let missing_project = server
        .post_json("/v2/session/projects/query-token", "", &cookie_header)
        .await?;
    assert_eq!(missing_project.status_code(), 200);
    assert_eq!(
        missing_project.header("content-encoding"),
        Some("gzip"),
        "{}",
        missing_project.body_str()
    );

    let invalid_project = server
        .post_json(&query_token_path("abc"), "", &cookie_header)
        .await?;
    assert_eq!(invalid_project.status_code(), 200);
    assert_eq!(
        invalid_project.header("content-encoding"),
        Some("gzip"),
        "{}",
        invalid_project.body_str()
    );

    // A project on someone else's account is not the caller's to mint for.
    let foreign_project = server
        .post_json(&query_token_path(DEMO_PROJECT_ID), "", &cookie_header)
        .await?;
    assert_eq!(
        foreign_project.status_code(),
        401,
        "{}",
        foreign_project.body_str()
    );

    let wrong_method = server
        .get_query(&query_token_path(account.project_id), &cookie_header)
        .await?;
    assert_eq!(wrong_method.status_code(), 405);
    assert_eq!(
        wrong_method.json::<Value>()["error"],
        json!("method not allowed; use POST")
    );

    delete_account(&server, &account.account_name).await;
    Ok(())
}

struct TestAccount {
    account_name: String,
    project_name: String,
    project_id: i32,
    session_id: String,
}

async fn signup(server: &TestServer, tag: &str) -> io::Result<TestAccount> {
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
                "password": PASSWORD,
                "plan": "starter",
                "events": "100k",
                "billing": "monthly",
            })
            .to_string(),
            &[],
        )
        .await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    let body: Value = response.json();
    let project_id = body["projects"][&project_name]["id"]
        .as_str()
        .expect("project id")
        .parse()
        .expect("numeric project id");

    Ok(TestAccount {
        account_name,
        project_name,
        project_id,
        session_id: session_id_from_cookie(&response),
    })
}

async fn mint_token(server: &TestServer, account: &TestAccount) -> io::Result<String> {
    let response = server
        .post_json(
            &query_token_path(account.project_id),
            "",
            &[("Cookie", &format!("rr_session={}", account.session_id))],
        )
        .await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    Ok(response.json::<Value>()["token"]
        .as_str()
        .expect("token")
        .to_string())
}

fn query_body(sql: &str, limit: Option<u32>, offset: Option<u32>) -> String {
    let mut body = json!({ "sql": sql });
    if let Some(limit) = limit {
        body["limit"] = json!(limit);
    }
    if let Some(offset) = offset {
        body["offset"] = json!(offset);
    }
    body.to_string()
}

fn session_id_from_cookie(response: &HttpResponse) -> String {
    response
        .header("set-cookie")
        .expect("set-cookie header")
        .split(';')
        .next()
        .and_then(|pair| pair.trim().strip_prefix("rr_session="))
        .expect("session cookie value")
        .to_string()
}

fn unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after the unix epoch")
        .as_secs() as i64
}

async fn delete_account(server: &TestServer, account_name: &str) {
    let conn = revscope::db::open_main_database_write(server.context.clone())
        .await
        .expect("open main database");
    conn.execute("DELETE FROM accounts WHERE name = $1", &[&account_name])
        .await
        .expect("delete test account");
}
