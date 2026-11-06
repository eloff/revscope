//! Every route that needs credentials is the boundary between customers.
//! Tables list them: project routes, and session routes that act on the
//! caller's account. The tests walk them: nothing gets in without live
//! credentials, nothing gets into another account's project, and the
//! account's own session gets past authorization on every route, so the
//! tables themselves are known to be live. The tests that sign up run only in
//! the hosted build, where signup creates an account with a project.

mod common;

use std::io;
#[cfg(not(feature = "self-hosted"))]
use std::time::{SystemTime, UNIX_EPOCH};

use common::{DEMO_PROJECT_ID, DEMO_PROJECT_ID_B, HttpResponse, TestServer};
#[cfg(not(feature = "self-hosted"))]
use serde_json::{Value, json};

#[cfg(not(feature = "self-hosted"))]
const SIGNUP_PATH: &str = "/v2/public/auth/signup";
#[cfg(not(feature = "self-hosted"))]
const PASSWORD: &str = "a-long-enough-password";

#[derive(Clone, Copy)]
enum Method {
    Get,
    Post,
    Delete,
}

struct ProjectRoute {
    method: Method,
    /// `{id}` is replaced with the project id.
    path: &'static str,
    body: &'static str,
}

const fn route(method: Method, path: &'static str) -> ProjectRoute {
    ProjectRoute {
        method,
        path,
        body: "{}",
    }
}

const fn route_with_body(method: Method, path: &'static str, body: &'static str) -> ProjectRoute {
    ProjectRoute { method, path, body }
}

/// Sent on every request. Routes authorize before they read their input, but
/// the owner's control test needs each request well-formed to get past the
/// handler too. Unused parameters are ignored.
const QUERY: &str = "start=1767225600&end=1769904000&id=1&conversation_id=1\
&payment_id=1&visitor_timestamp_id=0000000000000001&path_id=1&useragent_id=1&source=0&verified=0";

/// Every route that names a project, for each method it accepts. Keep in step
/// with the route table in `src/routes/table.rs`.
const PROJECT_ROUTES: &[ProjectRoute] = &[
    // /v2/session/projects/{id}/…
    route(Method::Delete, "/v2/session/projects/{id}"),
    route(Method::Get, "/v2/session/projects/{id}/settings"),
    route(Method::Post, "/v2/session/projects/{id}/settings"),
    route(Method::Get, "/v2/session/projects/{id}/troubleshooting"),
    route(
        Method::Post,
        "/v2/session/projects/{id}/troubleshooting/check",
    ),
    route(
        Method::Get,
        "/v2/session/projects/{id}/troubleshooting/ingest",
    ),
    route(Method::Post, "/v2/session/projects/{id}/query-token"),
    route(Method::Post, "/v2/session/projects/{id}/download-token"),
    route(Method::Post, "/v2/session/projects/{id}/funnels"),
    route(Method::Delete, "/v2/session/projects/{id}/funnels"),
    route(Method::Post, "/v2/session/projects/{id}/campaigns"),
    route(Method::Delete, "/v2/session/projects/{id}/campaigns"),
    route(Method::Post, "/v2/session/projects/{id}/campaign-spend"),
    // /v2/read/projects/{id}/…
    route(Method::Get, "/v2/read/projects/{id}/overview"),
    route(Method::Get, "/v2/read/projects/{id}/conversions"),
    route(Method::Get, "/v2/read/projects/{id}/payment-activity"),
    route(Method::Get, "/v2/read/projects/{id}/payment-journey"),
    route(Method::Get, "/v2/read/projects/{id}/revenue-sources"),
    route(Method::Get, "/v2/read/projects/{id}/search-terms"),
    route(Method::Get, "/v2/read/projects/{id}/audience"),
    route(Method::Get, "/v2/read/projects/{id}/pages"),
    route(Method::Get, "/v2/read/projects/{id}/page-transitions"),
    route(Method::Get, "/v2/read/projects/{id}/landing-pages"),
    route(Method::Get, "/v2/read/projects/{id}/visitors"),
    route(Method::Get, "/v2/read/projects/{id}/visitor-journey"),
    route(Method::Get, "/v2/read/projects/{id}/events"),
    route(Method::Get, "/v2/read/projects/{id}/crawlers"),
    route(Method::Get, "/v2/read/projects/{id}/crawler-detail"),
    route(Method::Get, "/v2/read/projects/{id}/funnels"),
    route(Method::Get, "/v2/read/projects/{id}/funnel-data"),
    route(Method::Post, "/v2/read/projects/{id}/funnel-preview"),
    route(Method::Get, "/v2/read/projects/{id}/funnel-options"),
    route(Method::Get, "/v2/read/projects/{id}/campaigns"),
    route(Method::Post, "/v2/read/projects/{id}/campaign-preview"),
    // /v2/chat/projects/{id}/…
    route_with_body(
        Method::Post,
        "/v2/chat/projects/{id}",
        r#"{"message":"hello"}"#,
    ),
    route(Method::Get, "/v2/chat/projects/{id}/conversations"),
    route(Method::Delete, "/v2/chat/projects/{id}/conversations"),
    route(Method::Get, "/v2/chat/projects/{id}/messages"),
    // The old Ask paths, still served while app bundles that call them deploy out.
    route_with_body(
        Method::Post,
        "/v2/read/projects/{id}/chat",
        r#"{"message":"hello"}"#,
    ),
    route(Method::Get, "/v2/read/projects/{id}/chat/conversations"),
    route(Method::Delete, "/v2/read/projects/{id}/chat/conversations"),
    route(Method::Get, "/v2/read/projects/{id}/chat/messages"),
    // /v2/query/projects/{id}/…
    route_with_body(
        Method::Post,
        "/v2/query/projects/{id}/sql",
        r#"{"sql":"SELECT 1"}"#,
    ),
    route(Method::Get, "/v2/query/projects/{id}/schema"),
    // /v2/download/projects/{id}
    route(Method::Get, "/v2/download/projects/{id}"),
];

/// Session routes that act on the caller's account rather than a named
/// project, in both builds. Keep these and [`EDITION_ACCOUNT_ROUTES`] in step
/// with the `AuthPolicy::Account` and `AuthPolicy::Admin` entries in
/// `src/routes/table.rs`.
const ACCOUNT_ROUTES: &[ProjectRoute] = &[
    route(Method::Get, "/v2/session/me"),
    // An empty body, so the owner's control request is refused as invalid
    // rather than creating a project.
    route(Method::Post, "/v2/session/projects"),
    route(Method::Get, "/v2/session/mcp/authorize"),
    route(Method::Post, "/v2/session/mcp/authorize"),
    route(Method::Post, "/v2/session/mcp/revoke"),
    // Empty bodies again, so the owner's control requests are refused as
    // invalid rather than inviting, changing or removing anyone.
    route(Method::Get, "/v2/session/users"),
    route(Method::Post, "/v2/session/users/invite"),
    route(Method::Post, "/v2/session/users/update"),
    route(Method::Post, "/v2/session/users/remove"),
];

#[cfg(feature = "billing")]
const EDITION_ACCOUNT_ROUTES: &[ProjectRoute] = &[
    route(Method::Post, "/v2/session/checkout"),
    route(Method::Get, "/v2/session/billing"),
    route_with_body(Method::Post, "/v2/session/billing/plan", PLAN_CHANGE),
    route_with_body(
        Method::Post,
        "/v2/session/billing/plan/preview",
        PLAN_CHANGE,
    ),
];

#[cfg(feature = "self-hosted")]
const EDITION_ACCOUNT_ROUTES: &[ProjectRoute] = &[];

/// A hosted build without the private billing module has no account routes of
/// its own.
#[cfg(not(any(feature = "billing", feature = "self-hosted")))]
const EDITION_ACCOUNT_ROUTES: &[ProjectRoute] = &[];

/// A valid plan change, so the owner's control test gets past the handler.
#[cfg(feature = "billing")]
const PLAN_CHANGE: &str = r#"{"plan":"starter","events":"100k","billing":"monthly"}"#;

/// A project id no account has.
const MISSING_PROJECT_ID: &str = "2000000000";

#[tokio::test]
async fn every_route_rejects_a_request_without_live_credentials() -> io::Result<()> {
    let server = TestServer::spawn("authz-anonymous").await?;
    // A well-formed token the server never issued, so it reaches the Postgres
    // lookup. Expiry has its own test, with a stored session.
    let unissued = format!("rr_session={}", random_uuid_v4());
    let credentials: [(&str, &[(&str, &str)]); 2] = [
        ("no credentials", &[]),
        ("unissued session", &[("Cookie", unissued.as_str())]),
    ];

    let mut failures = Vec::new();
    for (label, headers) in credentials {
        for project_id in [DEMO_PROJECT_ID_B, MISSING_PROJECT_ID] {
            for route in PROJECT_ROUTES {
                let response = send(&server, route, project_id, headers).await?;
                if response.status_code() != 401 {
                    failures.push(format!(
                        "{} [{label}]",
                        describe(route, project_id, &response)
                    ));
                }
            }
        }
        // The demo is public to read, but downloading it can copy its database,
        // so that takes a signed-in account too.
        for route in PROJECT_ROUTES
            .iter()
            .filter(|route| is_download_route(route))
        {
            let response = send(&server, route, DEMO_PROJECT_ID, headers).await?;
            if response.status_code() != 401 {
                failures.push(format!(
                    "{} [{label}]",
                    describe(route, DEMO_PROJECT_ID, &response)
                ));
            }
        }
        // Anyone may read and Ask about the demo, but a caller presenting a
        // dead session is told so rather than served as anonymous.
        if !headers.is_empty() {
            for route in PROJECT_ROUTES
                .iter()
                .filter(|route| is_demo_open_to_anyone(route))
            {
                let response = send(&server, route, DEMO_PROJECT_ID, headers).await?;
                if response.status_code() != 401 {
                    failures.push(format!(
                        "{} [{label}]",
                        describe(route, DEMO_PROJECT_ID, &response)
                    ));
                }
            }
        }
        for route in ACCOUNT_ROUTES.iter().chain(EDITION_ACCOUNT_ROUTES) {
            let response = send(&server, route, "", headers).await?;
            if response.status_code() != 401 {
                failures.push(format!("{} [{label}]", describe(route, "", &response)));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "requests without a live session or token must get 401:\n{}",
        failures.join("\n")
    );
    Ok(())
}

// Hosted signup creates a per-website account with a project; self-hosted signup does not.
#[cfg(not(feature = "self-hosted"))]
#[tokio::test]
async fn every_project_route_rejects_another_accounts_session() -> io::Result<()> {
    let server = TestServer::spawn("authz-cross-account").await?;
    let account = signup(&server, "authzcross").await?;
    let cookie = format!("rr_session={}", account.session_id);
    let bearer = format!("Bearer {}", account.session_id);

    let mut failures = Vec::new();
    // Project 2 belongs to another account. Project 1 is the public demo,
    // which signed-in callers may read, Ask about and download, but no one may
    // change or query with their own credentials.
    for (project_id, routes) in [
        (DEMO_PROJECT_ID_B, PROJECT_ROUTES.iter().collect::<Vec<_>>()),
        (
            DEMO_PROJECT_ID,
            PROJECT_ROUTES
                .iter()
                .filter(|route| !is_demo_open_to_any_account(route))
                .collect(),
        ),
    ] {
        for route in routes {
            for header in [
                ("Cookie", cookie.as_str()),
                ("Authorization", bearer.as_str()),
            ] {
                let response = send(&server, route, project_id, &[header]).await?;
                if !is_rejected(route, project_id, &response) {
                    failures.push(format!(
                        "{} [{}]",
                        describe(route, project_id, &response),
                        header.0
                    ));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "another account's session must not reach a project:\n{}",
        failures.join("\n")
    );

    delete_account(&server, &account.account_name).await;
    Ok(())
}

// Hosted signup creates a per-website account with a project; self-hosted signup does not.
#[cfg(not(feature = "self-hosted"))]
#[tokio::test]
async fn any_signed_in_account_may_download_the_demo() -> io::Result<()> {
    let server = TestServer::spawn_on_disk("authz-demo-download").await?;
    let account = signup(&server, "authzdemodl").await?;
    let cookie = format!("rr_session={}", account.session_id);
    let download_path = format!("/v2/download/projects/{DEMO_PROJECT_ID}");

    let with_session = server
        .get_query(&download_path, &[("Cookie", cookie.as_str())])
        .await?;
    assert_downloaded(&with_session);

    let minted = server
        .post_json(
            &format!("/v2/session/projects/{DEMO_PROJECT_ID}/download-token"),
            "{}",
            &[("Cookie", cookie.as_str())],
        )
        .await?;
    assert_eq!(minted.status_code(), 200, "{}", minted.body_str());
    let token = minted.json::<Value>()["token"]
        .as_str()
        .expect("download token")
        .to_string();
    let with_token = server
        .get_query(&format!("{download_path}?token={token}"), &[])
        .await?;
    assert_downloaded(&with_token);

    // The token names the demo, so it opens no other project's file.
    let elsewhere = server
        .get_query(
            &format!("/v2/download/projects/{DEMO_PROJECT_ID_B}?token={token}"),
            &[],
        )
        .await?;
    assert_eq!(elsewhere.status_code(), 401, "{}", elsewhere.body_str());

    delete_account(&server, &account.account_name).await;
    Ok(())
}

// Hosted signup creates a per-website account with a project; self-hosted signup does not.
#[cfg(not(feature = "self-hosted"))]
#[tokio::test]
async fn every_route_admits_its_own_accounts_session() -> io::Result<()> {
    let server = TestServer::spawn("authz-own-account").await?;
    let account = signup(&server, "authzown").await?;
    let cookie = format!("rr_session={}", account.session_id);
    let project_id = account.project_id.to_string();

    // A control for the rejection tests: if a path here stopped resolving, or
    // an owner were turned away too, a 401 there would prove nothing. Routes
    // naming a conversation this account lacks answer 404, which still means
    // the owner got past authorization.
    let mut failures = Vec::new();
    let account_routes = ACCOUNT_ROUTES
        .iter()
        .chain(EDITION_ACCOUNT_ROUTES)
        .map(|route| (route, ""));
    for (route, project_id) in PROJECT_ROUTES
        .iter()
        .map(|route| (route, project_id.as_str()))
        .chain(account_routes)
    {
        let response = send(&server, route, project_id, &[("Cookie", cookie.as_str())]).await?;
        let status = response.status_code();
        let honeypot = response.header("content-encoding") == Some("gzip");
        if status == 401 || status == 403 || status == 405 || honeypot {
            failures.push(describe(route, project_id, &response));
        }
    }
    assert!(
        failures.is_empty(),
        "the owner's session must reach every route on its account and project:\n{}",
        failures.join("\n")
    );

    delete_account(&server, &account.account_name).await;
    Ok(())
}

// Hosted signup creates a per-website account with a project; self-hosted signup does not.
#[cfg(not(feature = "self-hosted"))]
#[tokio::test]
async fn a_cached_session_stops_working_when_it_expires() -> io::Result<()> {
    let server = TestServer::spawn("authz-cached-expiry").await?;
    let account = signup(&server, "authzcachedexpiry").await?;

    // Expires well inside the session cache's TTL, so only the session's own
    // expiry can end the cached entry before the row is next read.
    let token = random_uuid_v4();
    let conn = revscope::db::open_main_database_write(server.context.clone())
        .await
        .expect("open main database");
    let inserted = conn
        .execute(
            "INSERT INTO sessions (token_hash, user_id, expires_at)
             SELECT sha256(uuid_send($1::text::uuid)), u.id, NOW() + INTERVAL '2 seconds'
             FROM users u
             INNER JOIN accounts a ON a.id = u.account_id
             WHERE a.name = $2",
            &[&token, &account.account_name],
        )
        .await
        .expect("insert short-lived session");
    assert_eq!(inserted, 1, "one user signed up on the account");
    drop(conn);

    let cookie = format!("rr_session={token}");
    let me = server
        .get_query("/v2/session/me", &[("Cookie", cookie.as_str())])
        .await?;
    assert_eq!(me.status_code(), 200, "{}", me.body_str());

    tokio::time::sleep(std::time::Duration::from_millis(3_100)).await;
    let me = server
        .get_query("/v2/session/me", &[("Cookie", cookie.as_str())])
        .await?;
    assert_eq!(
        me.status_code(),
        401,
        "a session cached while live must not outlive its expiry: {}",
        me.body_str()
    );

    delete_account(&server, &account.account_name).await;
    Ok(())
}

// Hosted signup creates a per-website account with a project; self-hosted signup does not.
#[cfg(not(feature = "self-hosted"))]
#[tokio::test]
async fn a_stored_session_past_its_max_age_reaches_nothing() -> io::Result<()> {
    let server = TestServer::spawn("authz-expired").await?;
    let account = signup(&server, "authzexpired").await?;

    // This row is otherwise live, so only the expiry check can refuse it.
    // uuid_send yields the token's 16 bytes, which is what the server hashes.
    let expired_id = random_uuid_v4();
    let conn = revscope::db::open_main_database_write(server.context.clone())
        .await
        .expect("open main database");
    let inserted = conn
        .execute(
            "INSERT INTO sessions (token_hash, user_id, expires_at)
             SELECT sha256(uuid_send($1::text::uuid)), u.id, NOW() - INTERVAL '1 second'
             FROM users u
             INNER JOIN accounts a ON a.id = u.account_id
             WHERE a.name = $2",
            &[&expired_id, &account.account_name],
        )
        .await
        .expect("insert expired session");
    assert_eq!(inserted, 1, "one user signed up on the account");
    drop(conn);

    let cookie = format!("rr_session={expired_id}");
    let project_id = account.project_id.to_string();
    let mut failures = Vec::new();
    for route in PROJECT_ROUTES {
        let response = send(&server, route, &project_id, &[("Cookie", cookie.as_str())]).await?;
        if response.status_code() != 401 {
            failures.push(describe(route, &project_id, &response));
        }
    }
    for route in PROJECT_ROUTES
        .iter()
        .filter(|route| is_demo_open_to_anyone(route))
    {
        let response = send(
            &server,
            route,
            DEMO_PROJECT_ID,
            &[("Cookie", cookie.as_str())],
        )
        .await?;
        if response.status_code() != 401 {
            failures.push(describe(route, DEMO_PROJECT_ID, &response));
        }
    }
    for route in ACCOUNT_ROUTES.iter().chain(EDITION_ACCOUNT_ROUTES) {
        let response = send(&server, route, "", &[("Cookie", cookie.as_str())]).await?;
        if response.status_code() != 401 {
            failures.push(describe(route, "", &response));
        }
    }
    assert!(
        failures.is_empty(),
        "a session past its max age must get 401 on its own project:\n{}",
        failures.join("\n")
    );

    // The same row, unexpired, signs in: so the 401s above came from the
    // expiry and not from a hash that never matched the cookie.
    let conn = revscope::db::open_main_database_write(server.context.clone())
        .await
        .expect("open main database");
    let revived = conn
        .execute(
            "UPDATE sessions SET expires_at = NOW() + INTERVAL '1 hour'
             WHERE token_hash = sha256(uuid_send($1::text::uuid))",
            &[&expired_id],
        )
        .await
        .expect("revive session");
    assert_eq!(revived, 1);
    drop(conn);
    let me = server
        .get_query("/v2/session/me", &[("Cookie", cookie.as_str())])
        .await?;
    assert_eq!(me.status_code(), 200, "{}", me.body_str());

    delete_account(&server, &account.account_name).await;
    Ok(())
}

#[cfg(not(feature = "self-hosted"))]
fn assert_downloaded(response: &HttpResponse) {
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    assert_eq!(
        response.header("content-type"),
        Some("application/vnd.sqlite3")
    );
}

/// Routes an anonymous caller may use on the public demo.
fn is_demo_open_to_anyone(route: &ProjectRoute) -> bool {
    route.path.starts_with("/v2/read/") || route.path.starts_with("/v2/chat/")
}

/// Routes any signed-in account may use on the public demo.
#[cfg(not(feature = "self-hosted"))]
fn is_demo_open_to_any_account(route: &ProjectRoute) -> bool {
    route.path.starts_with("/v2/read/")
        || route.path.starts_with("/v2/chat/")
        || is_download_route(route)
}

/// The download token and the download it authorizes.
fn is_download_route(route: &ProjectRoute) -> bool {
    route.path.starts_with("/v2/download/") || route.path.ends_with("/download-token")
}

#[cfg(not(feature = "self-hosted"))]
/// A foreign session gets 401, except that changing the public demo is
/// refused as read-only rather than as unauthorized.
fn is_rejected(route: &ProjectRoute, project_id: &str, response: &HttpResponse) -> bool {
    match response.status_code() {
        401 => true,
        400 if project_id == DEMO_PROJECT_ID => {
            response.json::<Value>()["error"] == json!("the demo project is read-only")
                && !is_demo_open_to_any_account(route)
        }
        _ => false,
    }
}

async fn send(
    server: &TestServer,
    route: &ProjectRoute,
    project_id: &str,
    headers: &[(&str, &str)],
) -> io::Result<HttpResponse> {
    let path = format!("{}?{QUERY}", route.path.replace("{id}", project_id));
    match route.method {
        Method::Get => server.get_query(&path, headers).await,
        Method::Post => server.post_json(&path, route.body, headers).await,
        Method::Delete => server.delete(&path, headers).await,
    }
}

fn describe(route: &ProjectRoute, project_id: &str, response: &HttpResponse) -> String {
    let method = match route.method {
        Method::Get => "GET",
        Method::Post => "POST",
        Method::Delete => "DELETE",
    };
    format!(
        "  {method} {} -> {} {}",
        route.path.replace("{id}", project_id),
        response.status_code(),
        response.body_str().chars().take(120).collect::<String>()
    )
}

#[cfg(not(feature = "self-hosted"))]
struct TestAccount {
    account_name: String,
    project_id: i32,
    session_id: String,
}

#[cfg(not(feature = "self-hosted"))]
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
        project_id,
        session_id: session_id_from_cookie(&response),
    })
}

/// A token in the form the server issues: a random, hyphenated UUIDv4.
fn random_uuid_v4() -> String {
    let mut bytes = [0_u8; 16];
    getrandom::getrandom(&mut bytes).expect("random bytes");
    let random = u128::from_be_bytes(bytes);
    let value = (random & !(0xF << 76) & !(0x3 << 62)) | (0x4 << 76) | (0x2 << 62);
    let hex = format!("{value:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

#[cfg(not(feature = "self-hosted"))]
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

#[cfg(not(feature = "self-hosted"))]
async fn delete_account(server: &TestServer, account_name: &str) {
    let conn = revscope::db::open_main_database_write(server.context.clone())
        .await
        .expect("open main database");
    conn.execute("DELETE FROM accounts WHERE name = $1", &[&account_name])
        .await
        .expect("delete test account");
}
