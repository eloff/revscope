// Hosted signup creates a per-website account with a plan; self-hosted signup does not.
#![cfg(not(feature = "self-hosted"))]

//! The hosted MCP server at `/v2/mcp`, from the 401 an unconfigured client gets
//! to a tool call answered with real project data.
//!
//! The flow under test is the one an agent actually walks: discover the OAuth
//! metadata, register, have the signed-in user approve, exchange the code for a
//! token, then speak MCP with it. The token covers the whole account, so the
//! tool calls name the project they want and are held to that account's.

mod common;

use std::io;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as BASE64URL;
use common::{DEMO_PROJECT_NAME, DEMO_PROJECT_NAME_B, HttpResponse, TestServer, TrackHit, page_id};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const SIGNUP_PATH: &str = "/v2/public/auth/signup";
const PASSWORD: &str = "a-long-enough-password";
const MCP_PATH: &str = "/v2/mcp";
const REGISTER_PATH: &str = "/v2/public/mcp/register";
const TOKEN_PATH: &str = "/v2/public/mcp/token";
const AUTHORIZE_PATH: &str = "/v2/session/mcp/authorize";
const REVOKE_PATH: &str = "/v2/session/mcp/revoke";
const PROTECTED_RESOURCE_PATH: &str = "/.well-known/oauth-protected-resource/v2/mcp";
const AUTHORIZATION_SERVER_PATH: &str = "/.well-known/oauth-authorization-server";
const REDIRECT_URI: &str = "http://127.0.0.1:33418/oauth/callback";
/// The example pair from RFC 7636 appendix B, so the challenge is checkable by hand.
const CODE_VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const ONE_YEAR_SECONDS: i64 = 365 * 24 * 60 * 60;
/// Both are required of an MCP POST by the Streamable HTTP transport.
const MCP_ACCEPT: &str = "application/json, text/event-stream";

#[tokio::test]
async fn an_unauthenticated_client_is_pointed_at_the_oauth_metadata() -> io::Result<()> {
    let server = TestServer::spawn("mcp-discovery").await?;

    let challenged = server
        .post_json(MCP_PATH, &initialize_request(), &[("Accept", MCP_ACCEPT)])
        .await?;
    assert_eq!(challenged.status_code(), 401, "{}", challenged.body_str());
    let challenge = challenged
        .header("www-authenticate")
        .expect("www-authenticate header");
    assert!(
        challenge.contains(&format!(
            "resource_metadata=\"https://ingest.example.test{PROTECTED_RESOURCE_PATH}\""
        )),
        "the challenge names the document that starts the flow: {challenge}"
    );
    // A client running in a browser has to be able to read that header.
    assert!(
        challenged
            .header("access-control-expose-headers")
            .is_some_and(|exposed| exposed.contains("WWW-Authenticate")),
        "{:?}",
        challenged.header("access-control-expose-headers")
    );

    let resource: Value = server.get_query(PROTECTED_RESOURCE_PATH, &[]).await?.json();
    assert_eq!(
        resource["resource"],
        json!("https://ingest.example.test/v2/mcp")
    );
    assert_eq!(
        resource["authorization_servers"],
        json!(["https://ingest.example.test"])
    );

    let authorization_server: Value = server
        .get_query(AUTHORIZATION_SERVER_PATH, &[])
        .await?
        .json();
    assert_eq!(
        authorization_server["registration_endpoint"],
        json!(format!("https://ingest.example.test{REGISTER_PATH}"))
    );
    assert_eq!(
        authorization_server["token_endpoint"],
        json!(format!("https://ingest.example.test{TOKEN_PATH}"))
    );
    assert!(
        authorization_server["authorization_endpoint"]
            .as_str()
            .expect("authorization_endpoint")
            .ends_with("/oauth/authorize"),
        "the consent step is a page in the web app: {authorization_server}"
    );
    assert_eq!(
        authorization_server["code_challenge_methods_supported"],
        json!(["S256"])
    );
    assert_eq!(
        authorization_server["token_endpoint_auth_methods_supported"],
        json!(["none"])
    );

    Ok(())
}

#[tokio::test]
async fn a_registered_client_earns_a_token_and_reads_its_project() -> io::Result<()> {
    let mut server = TestServer::spawn("mcp-flow").await?;
    let account = signup(&server, "mcpflow").await?;

    let client_id = register_client(&server).await?;
    let redirect = approve(&server, &account, &client_id, Some("opaque-state")).await?;
    let code = query_parameter(&redirect, "code").expect("code in the redirect");
    assert_eq!(
        query_parameter(&redirect, "state").as_deref(),
        Some("opaque-state"),
        "the client's state comes back untouched: {redirect}"
    );

    let granted = server
        .post_form(
            TOKEN_PATH,
            &token_request(&code, CODE_VERIFIER, &client_id),
            &[],
        )
        .await?;
    assert_eq!(granted.status_code(), 200, "{}", granted.body_str());
    let granted: Value = granted.json();
    let token = granted["access_token"]
        .as_str()
        .expect("access_token")
        .to_string();
    assert!(
        token.starts_with("rsm_"),
        "an MCP token is recognizable in an Authorization header: {token}"
    );
    assert_eq!(granted["token_type"], json!("Bearer"));
    assert_eq!(granted["expires_in"], json!(ONE_YEAR_SECONDS));

    let authorization = format!("Bearer {token}");
    let headers = [
        ("Authorization", authorization.as_str()),
        ("Accept", MCP_ACCEPT),
    ];

    let initialized = mcp_result(&server, &initialize_request(), &headers).await?;
    assert_eq!(initialized["serverInfo"]["name"], json!("revscope"));
    assert!(
        initialized["capabilities"]["tools"].is_object(),
        "the server advertises tools: {initialized}"
    );

    let listed = mcp_result(&server, &jsonrpc("tools/list", json!({}), 2), &headers).await?;
    let mut names: Vec<&str> = listed["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool name"))
        .collect();
    names.sort_unstable();
    assert_eq!(names, vec!["get_schema", "list_projects", "run_query"]);
    let run_query_schema = listed["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .find(|tool| tool["name"] == json!("run_query"))
        .expect("run_query is advertised")["inputSchema"]
        .clone();
    assert!(
        run_query_schema["required"]
            .as_array()
            .expect("required list")
            .contains(&json!("project_name")),
        "the client is told the project is not optional: {run_query_schema}"
    );

    let projects = mcp_result(
        &server,
        &jsonrpc("tools/call", json!({ "name": "list_projects" }), 3),
        &headers,
    )
    .await?;
    assert_eq!(
        tool_text(&projects),
        format!("1 project(s):\n{}", account.project_name),
        "the one project signup created is the one the token can reach"
    );

    let schema = mcp_result(
        &server,
        &jsonrpc("tools/call", json!({ "name": "get_schema" }), 4),
        &headers,
    )
    .await?;
    assert!(
        tool_text(&schema).contains("CREATE TABLE"),
        "{}",
        tool_text(&schema)
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

    let queried = mcp_result(
        &server,
        &jsonrpc(
            "tools/call",
            json!({
                "name": "run_query",
                "arguments": {
                    "project_name": account.project_name,
                    "sql": "SELECT path FROM paths ORDER BY path",
                    "limit": 1,
                },
            }),
            5,
        ),
        &headers,
    )
    .await?;
    let text = tool_text(&queried);
    assert!(
        text.starts_with("1 row(s), limit 1, offset 0, more rows available (next offset 1)"),
        "the summary tells the agent to paginate: {text}"
    );
    assert!(text.contains("/pricing"), "{text}");

    // A statement the model got wrong is content, not a protocol error, so it
    // can read the message and try again.
    let rejected = mcp_result(
        &server,
        &jsonrpc(
            "tools/call",
            json!({
                "name": "run_query",
                "arguments": {
                    "project_name": account.project_name,
                    "sql": "DELETE FROM paths",
                },
            }),
            6,
        ),
        &headers,
    )
    .await?;
    assert_eq!(rejected["isError"], json!(true), "{rejected}");

    delete_account(&server, &account.account_name).await;
    Ok(())
}

/// A missing or unrecognized project name is answered with the names that would
/// have worked, so the agent can correct itself rather than guess.
#[tokio::test]
async fn a_query_without_a_usable_project_name_is_told_what_to_use() -> io::Result<()> {
    let server = TestServer::spawn("mcp-project-name").await?;
    let account = signup(&server, "mcpprojectname").await?;
    let authorization = mcp_authorization(&server, &account).await?;
    let headers = [
        ("Authorization", authorization.as_str()),
        ("Accept", MCP_ACCEPT),
    ];

    // There is no default project, so an omitted name is a question, not a
    // silent pick.
    let omitted = query_error(&server, &headers, json!({ "sql": "SELECT 1" }), 2).await?;
    assert!(
        omitted.starts_with("missing project name; available projects are:"),
        "{omitted}"
    );
    assert!(omitted.contains(&account.project_name), "{omitted}");

    let unknown = query_error(
        &server,
        &headers,
        json!({ "project_name": "no_such_project", "sql": "SELECT 1" }),
        3,
    )
    .await?;
    assert!(
        unknown.starts_with("unknown project name \"no_such_project\";"),
        "{unknown}"
    );

    // A real project on another account is reported exactly like one that does
    // not exist: the answer must not confirm the name is taken.
    let foreign = query_error(
        &server,
        &headers,
        json!({ "project_name": DEMO_PROJECT_NAME, "sql": "SELECT 1" }),
        4,
    )
    .await?;
    assert_eq!(
        foreign,
        format!(
            "unknown project name \"{DEMO_PROJECT_NAME}\"; available projects are: {}",
            account.project_name
        ),
        "a token must not reach across accounts"
    );

    delete_account(&server, &account.account_name).await;
    Ok(())
}

#[tokio::test]
async fn the_demo_token_reads_the_demo_and_no_other_project() -> io::Result<()> {
    let demo_token = format!("rsm_{}", "d".repeat(32));
    let server = TestServer::spawn_with_mcp_demo_token("mcp-demo-token", &demo_token).await?;
    let account = signup(&server, "mcpdemotoken").await?;
    let authorization = format!("Bearer {demo_token}");
    let headers = [
        ("Authorization", authorization.as_str()),
        ("Accept", MCP_ACCEPT),
    ];

    let projects = mcp_result(
        &server,
        &jsonrpc("tools/call", json!({ "name": "list_projects" }), 2),
        &headers,
    )
    .await?;
    assert_eq!(
        tool_text(&projects),
        format!("1 project(s):\n{DEMO_PROJECT_NAME}")
    );

    let schema = mcp_result(
        &server,
        &jsonrpc("tools/call", json!({ "name": "get_schema" }), 3),
        &headers,
    )
    .await?;
    assert!(
        tool_text(&schema).contains("CREATE TABLE"),
        "{}",
        tool_text(&schema)
    );

    let queried = mcp_result(
        &server,
        &jsonrpc(
            "tools/call",
            json!({
                "name": "run_query",
                "arguments": {
                    "project_name": DEMO_PROJECT_NAME,
                    "sql": "SELECT count(*) AS n FROM paths",
                },
            }),
            4,
        ),
        &headers,
    )
    .await?;
    assert_eq!(queried["isError"], json!(false), "{queried}");
    assert!(
        tool_text(&queried).starts_with("1 row(s)"),
        "{}",
        tool_text(&queried)
    );

    for (other, id) in [(DEMO_PROJECT_NAME_B, 5), (account.project_name.as_str(), 6)] {
        let refused = query_error(
            &server,
            &headers,
            json!({ "project_name": other, "sql": "SELECT 1" }),
            id,
        )
        .await?;
        assert_eq!(
            refused,
            format!("unknown project name {other:?}; available projects are: {DEMO_PROJECT_NAME}"),
            "the demo token must not reach another account's project"
        );
    }

    // Configuring the demo token must not open the demo to account tokens.
    let account_authorization = mcp_authorization(&server, &account).await?;
    let account_headers = [
        ("Authorization", account_authorization.as_str()),
        ("Accept", MCP_ACCEPT),
    ];
    let foreign = query_error(
        &server,
        &account_headers,
        json!({ "project_name": DEMO_PROJECT_NAME, "sql": "SELECT 1" }),
        7,
    )
    .await?;
    assert_eq!(
        foreign,
        format!(
            "unknown project name \"{DEMO_PROJECT_NAME}\"; available projects are: {}",
            account.project_name
        )
    );

    delete_account(&server, &account.account_name).await;
    Ok(())
}

/// The same token the test above configures, sent to a server that configured
/// none, so the demo answers only because a token was set; which variable sets
/// it is `mcp::token`'s unit test.
#[tokio::test]
async fn without_a_configured_demo_token_the_demo_is_unreachable() -> io::Result<()> {
    let server = TestServer::spawn("mcp-no-demo-token").await?;
    let authorization = format!("Bearer rsm_{}", "d".repeat(32));
    let refused = server
        .post_json(
            MCP_PATH,
            &jsonrpc(
                "tools/call",
                json!({
                    "name": "run_query",
                    "arguments": { "project_name": DEMO_PROJECT_NAME, "sql": "SELECT 1" },
                }),
                1,
            ),
            &[
                ("Authorization", authorization.as_str()),
                ("Accept", MCP_ACCEPT),
            ],
        )
        .await?;
    assert_eq!(refused.status_code(), 401, "{}", refused.body_str());
    Ok(())
}

#[tokio::test]
async fn only_a_live_token_and_a_browser_session_open_the_two_doors() -> io::Result<()> {
    let server = TestServer::spawn("mcp-scope").await?;
    let account = signup(&server, "mcpscope").await?;

    // Approving needs the browser session; the minted token cannot mint another.
    let unauthenticated = server.post_json(AUTHORIZE_PATH, "{}", &[]).await?;
    assert_eq!(
        unauthenticated.status_code(),
        401,
        "{}",
        unauthenticated.body_str()
    );

    let unknown_token = format!("Bearer rsm_{}", "0".repeat(32));
    let unknown = server
        .post_json(
            MCP_PATH,
            &initialize_request(),
            &[
                ("Authorization", unknown_token.as_str()),
                ("Accept", MCP_ACCEPT),
            ],
        )
        .await?;
    assert_eq!(unknown.status_code(), 401, "{}", unknown.body_str());
    assert!(
        unknown.header("www-authenticate").is_some(),
        "an expired token is told where to get a new one"
    );

    delete_account(&server, &account.account_name).await;
    Ok(())
}

#[tokio::test]
async fn revoking_from_settings_deletes_every_mcp_token_on_the_account() -> io::Result<()> {
    let server = TestServer::spawn("mcp-revoke").await?;
    let account = signup(&server, "mcprevoke").await?;
    let cookie = session_cookie(&account);
    let authorization = mcp_authorization(&server, &account).await?;
    let headers = [
        ("Authorization", authorization.as_str()),
        ("Accept", MCP_ACCEPT),
    ];

    // Put the token in the verified-token cache so revoke has to drop that too,
    // not only the row.
    let listed = mcp_result(
        &server,
        &jsonrpc("tools/call", json!({ "name": "list_projects" }), 2),
        &headers,
    )
    .await?;
    assert!(
        tool_text(&listed).contains(&account.project_name),
        "{}",
        tool_text(&listed)
    );

    let unauthenticated = server.post_json(REVOKE_PATH, "", &[]).await?;
    assert_eq!(
        unauthenticated.status_code(),
        401,
        "{}",
        unauthenticated.body_str()
    );

    let query_token_path = format!("/v2/session/projects/{}/query-token", account.project_id);
    let minted = server
        .post_json(&query_token_path, "", &[("Cookie", cookie.as_str())])
        .await?;
    assert_eq!(minted.status_code(), 200, "{}", minted.body_str());
    let query_token = minted.json::<Value>()["token"]
        .as_str()
        .expect("query token")
        .to_string();

    let revoked = server
        .post_json(REVOKE_PATH, "", &[("Cookie", cookie.as_str())])
        .await?;
    assert_eq!(revoked.status_code(), 200, "{}", revoked.body_str());
    assert_eq!(revoked.json::<Value>()["revoked"], json!(1));

    let rejected = server
        .post_json(MCP_PATH, &initialize_request(), &headers)
        .await?;
    assert_eq!(rejected.status_code(), 401, "{}", rejected.body_str());

    let query_authorization = format!("Bearer {query_token}");
    let schema = server
        .get_query(
            &format!("/v2/query/projects/{}/schema", account.project_id),
            &[("Authorization", query_authorization.as_str())],
        )
        .await?;
    assert_eq!(
        schema.status_code(),
        200,
        "a query token is a different kind and must survive: {}",
        schema.body_str()
    );

    let empty = server
        .post_json(REVOKE_PATH, "", &[("Cookie", cookie.as_str())])
        .await?;
    assert_eq!(empty.status_code(), 200, "{}", empty.body_str());
    assert_eq!(empty.json::<Value>()["revoked"], json!(0));

    delete_account(&server, &account.account_name).await;
    Ok(())
}

#[tokio::test]
async fn the_oauth_endpoints_reject_a_forged_or_replayed_exchange() -> io::Result<()> {
    let server = TestServer::spawn("mcp-oauth-errors").await?;
    let account = signup(&server, "mcpoautherrors").await?;
    let client_id = register_client(&server).await?;

    // A redirect URI the client never registered is refused before any code
    // exists, which is what keeps a code from being delivered elsewhere.
    let mismatched = server
        .post_json(
            AUTHORIZE_PATH,
            &json!({
                "client_id": client_id,
                "redirect_uri": "https://evil.example.com/callback",
                "code_challenge": code_challenge(CODE_VERIFIER),
                "approve": true,
            })
            .to_string(),
            &[("Cookie", &session_cookie(&account))],
        )
        .await?;
    assert_eq!(mismatched.status_code(), 400, "{}", mismatched.body_str());
    assert_eq!(
        mismatched.json::<Value>()["error"],
        json!("invalid_request")
    );

    // Declining still redirects, so the client learns the answer.
    let denied = server
        .post_json(
            AUTHORIZE_PATH,
            &json!({
                "client_id": client_id,
                "redirect_uri": REDIRECT_URI,
                "state": "denied-state",
                "code_challenge": code_challenge(CODE_VERIFIER),
                "approve": false,
            })
            .to_string(),
            &[("Cookie", &session_cookie(&account))],
        )
        .await?;
    assert_eq!(denied.status_code(), 200, "{}", denied.body_str());
    let denied = redirect_url(&denied);
    assert_eq!(
        query_parameter(&denied, "error").as_deref(),
        Some("access_denied"),
        "{denied}"
    );
    assert!(query_parameter(&denied, "code").is_none(), "{denied}");

    let redirect = approve(&server, &account, &client_id, None).await?;
    let code = query_parameter(&redirect, "code").expect("code in the redirect");

    // Without the verifier the code is worthless, which is the whole point of
    // PKCE for a client that holds no secret.
    let wrong_verifier = server
        .post_form(
            TOKEN_PATH,
            &token_request(&code, &CODE_VERIFIER.replace('d', "e"), &client_id),
            &[],
        )
        .await?;
    assert_eq!(
        wrong_verifier.status_code(),
        400,
        "{}",
        wrong_verifier.body_str()
    );
    assert_eq!(
        wrong_verifier.json::<Value>()["error"],
        json!("invalid_grant")
    );

    // That failed attempt consumed the code, so even the right verifier is now
    // too late: a guessed code is not worth guessing at twice.
    let replayed = server
        .post_form(
            TOKEN_PATH,
            &token_request(&code, CODE_VERIFIER, &client_id),
            &[],
        )
        .await?;
    assert_eq!(replayed.status_code(), 400, "{}", replayed.body_str());
    assert_eq!(replayed.json::<Value>()["error"], json!("invalid_grant"));

    // Only the authorization code grant exists; there is no refresh token to ask for.
    let unsupported = server
        .post_form(TOKEN_PATH, "grant_type=refresh_token&refresh_token=x", &[])
        .await?;
    assert_eq!(unsupported.status_code(), 400, "{}", unsupported.body_str());
    assert_eq!(
        unsupported.json::<Value>()["error"],
        json!("unsupported_grant_type")
    );

    // Registration is the one unauthenticated write, so what it accepts matters.
    let bad_registration = server
        .post_json(
            REGISTER_PATH,
            &json!({
                "client_name": "Malicious",
                "redirect_uris": ["javascript:alert(1)"],
            })
            .to_string(),
            &[],
        )
        .await?;
    assert_eq!(
        bad_registration.status_code(),
        400,
        "{}",
        bad_registration.body_str()
    );
    assert_eq!(
        bad_registration.json::<Value>()["error"],
        json!("invalid_redirect_uri")
    );

    delete_account(&server, &account.account_name).await;
    Ok(())
}

#[tokio::test]
async fn registration_is_rate_limited_per_address() -> io::Result<()> {
    let server = TestServer::spawn("mcp-register-limit").await?;
    for _ in 0..60 {
        register_client(&server).await?;
    }
    let refused = server
        .post_json(
            REGISTER_PATH,
            &json!({ "redirect_uris": [REDIRECT_URI] }).to_string(),
            &[],
        )
        .await?;
    assert_eq!(refused.status_code(), 429, "{}", refused.body_str());
    assert!(
        refused.header("retry-after").is_some(),
        "a refused client is told when to retry"
    );
    Ok(())
}

#[tokio::test]
async fn cleanup_deletes_day_old_clients_that_never_got_a_token() -> io::Result<()> {
    let server = TestServer::spawn("mcp-client-cleanup").await?;
    let account = signup(&server, "mcpclientcleanup").await?;

    let used = register_client(&server).await?;
    let redirect = approve(&server, &account, &used, None).await?;
    let code = query_parameter(&redirect, "code").expect("code in the redirect");
    let granted = server
        .post_form(TOKEN_PATH, &token_request(&code, CODE_VERIFIER, &used), &[])
        .await?;
    assert_eq!(granted.status_code(), 200, "{}", granted.body_str());
    let abandoned = register_client(&server).await?;
    let fresh = register_client(&server).await?;

    let conn = revscope::db::open_main_database_write(server.context.clone())
        .await
        .map_err(io::Error::other)?;
    conn.execute(
        "UPDATE oauth_clients SET created_at = NOW() - INTERVAL '2 days'
         WHERE id = ANY($1::text[]::uuid[])",
        &[&vec![used.clone(), abandoned.clone()]],
    )
    .await
    .map_err(io::Error::other)?;

    revscope::run_main_db_cleanup(server.context.clone()).await;

    let remaining: Vec<String> = conn
        .query(
            "SELECT id::text FROM oauth_clients WHERE id = ANY($1::text[]::uuid[]) ORDER BY id",
            &[&vec![used.clone(), abandoned, fresh.clone()]],
        )
        .await
        .map_err(io::Error::other)?
        .iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(remaining, vec![used, fresh]);

    delete_account(&server, &account.account_name).await;
    Ok(())
}

/// Registers a public client the way an MCP client does on first contact.
async fn register_client(server: &TestServer) -> io::Result<String> {
    let response = server
        .post_json(
            REGISTER_PATH,
            &json!({
                "client_name": "Test MCP client",
                "redirect_uris": [REDIRECT_URI],
                "grant_types": ["authorization_code"],
                "response_types": ["code"],
                "token_endpoint_auth_method": "none",
            })
            .to_string(),
            &[],
        )
        .await?;
    assert_eq!(response.status_code(), 201, "{}", response.body_str());
    let body: Value = response.json();
    assert_eq!(body["client_name"], json!("Test MCP client"));
    assert_eq!(body["token_endpoint_auth_method"], json!("none"));
    Ok(body["client_id"].as_str().expect("client_id").to_string())
}

/// What the consent page does once the user approves. There is no project to
/// pick: the grant is for the account the session belongs to.
async fn approve(
    server: &TestServer,
    account: &TestAccount,
    client_id: &str,
    state: Option<&str>,
) -> io::Result<String> {
    let cookie = session_cookie(account);

    // The page names the client from this, which also proves the redirect URI
    // belongs to it before a browser is ever pointed at it.
    let described = server
        .get_query(
            &format!(
                "{AUTHORIZE_PATH}?client_id={client_id}&redirect_uri={}",
                urlencode(REDIRECT_URI)
            ),
            &[("Cookie", cookie.as_str())],
        )
        .await?;
    assert_eq!(described.status_code(), 200, "{}", described.body_str());
    assert_eq!(
        described.json::<Value>()["client_name"],
        json!("Test MCP client")
    );

    let mut body = json!({
        "client_id": client_id,
        "redirect_uri": REDIRECT_URI,
        "code_challenge": code_challenge(CODE_VERIFIER),
        "code_challenge_method": "S256",
        "approve": true,
    });
    if let Some(state) = state {
        body["state"] = json!(state);
    }
    let response = server
        .post_json(
            AUTHORIZE_PATH,
            &body.to_string(),
            &[("Cookie", cookie.as_str())],
        )
        .await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    Ok(redirect_url(&response))
}

/// Walks the whole flow — register, approve, exchange — and returns the
/// `Authorization` header value the resulting token is used through.
async fn mcp_authorization(server: &TestServer, account: &TestAccount) -> io::Result<String> {
    let client_id = register_client(server).await?;
    let redirect = approve(server, account, &client_id, None).await?;
    let code = query_parameter(&redirect, "code").expect("code in the redirect");
    let granted = server
        .post_form(
            TOKEN_PATH,
            &token_request(&code, CODE_VERIFIER, &client_id),
            &[],
        )
        .await?;
    assert_eq!(granted.status_code(), 200, "{}", granted.body_str());
    Ok(format!(
        "Bearer {}",
        granted.json::<Value>()["access_token"]
            .as_str()
            .expect("access_token")
    ))
}

/// Calls `run_query`, asserts it came back as a tool error, and returns the
/// message the agent would read.
async fn query_error(
    server: &TestServer,
    headers: &[(&str, &str)],
    arguments: Value,
    id: u32,
) -> io::Result<String> {
    let result = mcp_result(
        server,
        &jsonrpc(
            "tools/call",
            json!({ "name": "run_query", "arguments": arguments }),
            id,
        ),
        headers,
    )
    .await?;
    assert_eq!(result["isError"], json!(true), "{result}");
    Ok(tool_text(&result))
}

/// Sends one JSON-RPC request to `/v2/mcp` and unwraps its `result`.
async fn mcp_result(
    server: &TestServer,
    body: &str,
    headers: &[(&str, &str)],
) -> io::Result<Value> {
    let response = server.post_json(MCP_PATH, body, headers).await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    let payload: Value = response.json();
    assert!(
        payload["error"].is_null(),
        "the request failed at the protocol level: {payload}"
    );
    Ok(payload["result"].clone())
}

fn jsonrpc(method: &str, params: Value, id: u32) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }).to_string()
}

fn initialize_request() -> String {
    jsonrpc(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": "test-client", "version": "0.1.0" },
        }),
        1,
    )
}

/// The text of a tool result, which is where both tools put their answer.
fn tool_text(result: &Value) -> String {
    result["content"]
        .as_array()
        .expect("tool content")
        .iter()
        .filter_map(|block| block["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn token_request(code: &str, code_verifier: &str, client_id: &str) -> String {
    serde_urlencoded::to_string([
        ("grant_type", "authorization_code"),
        ("code", code),
        ("code_verifier", code_verifier),
        ("client_id", client_id),
        ("redirect_uri", REDIRECT_URI),
    ])
    .expect("encode token request")
}

fn code_challenge(code_verifier: &str) -> String {
    BASE64URL.encode(Sha256::digest(code_verifier.as_bytes()))
}

fn redirect_url(response: &HttpResponse) -> String {
    response.json::<Value>()["redirect_url"]
        .as_str()
        .expect("redirect_url")
        .to_string()
}

fn query_parameter(url: &str, name: &str) -> Option<String> {
    let (_base, query) = url.split_once('?')?;
    serde_urlencoded::from_str::<Vec<(String, String)>>(query)
        .ok()?
        .into_iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value)
}

fn urlencode(value: &str) -> String {
    serde_urlencoded::to_string([("v", value)])
        .expect("encode")
        .trim_start_matches("v=")
        .to_string()
}

struct TestAccount {
    account_name: String,
    project_name: String,
    project_id: i32,
    session_id: String,
}

fn session_cookie(account: &TestAccount) -> String {
    format!("rr_session={}", account.session_id)
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

async fn delete_account(server: &TestServer, account_name: &str) {
    let conn = revscope::db::open_main_database_write(server.context.clone())
        .await
        .expect("open main database");
    conn.execute("DELETE FROM accounts WHERE name = $1", &[&account_name])
        .await
        .expect("delete test account");
}
