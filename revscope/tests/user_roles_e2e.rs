// Hosted signup makes the account's first admin; self-hosted is covered by
// self_hosted_signup_e2e.rs.
#![cfg(not(feature = "self-hosted"))]

//! Admins and everyone else: inviting users to the account or to some of its
//! projects, changing their access, and removing them.

mod common;

use std::io;
use std::time::{SystemTime, UNIX_EPOCH};

use common::{HttpResponse, TestServer};
use serde_json::{Value, json};

const SIGNUP_PATH: &str = "/v2/public/auth/signup";
const LOGIN_PATH: &str = "/v2/public/auth/login";
const RESET_PASSWORD_PATH: &str = "/v2/public/auth/reset-password";
const ME_PATH: &str = "/v2/session/me";
const USERS_PATH: &str = "/v2/session/users";
const INVITE_PATH: &str = "/v2/session/users/invite";
const UPDATE_PATH: &str = "/v2/session/users/update";
const REMOVE_PATH: &str = "/v2/session/users/remove";
const CREATE_PROJECT_PATH: &str = "/v2/session/projects";
#[cfg(feature = "billing")]
const BILLING_PATH: &str = "/v2/session/billing";
const MCP_REGISTER_PATH: &str = "/v2/public/mcp/register";
const MCP_AUTHORIZE_PATH: &str = "/v2/session/mcp/authorize";
const PASSWORD: &str = "a-long-enough-password";

struct Account {
    account_name: String,
    slug: String,
    project_id: String,
    cookie: String,
}

#[tokio::test]
async fn admins_invite_scope_and_remove_the_rest_of_the_account() -> io::Result<()> {
    let server = TestServer::spawn_on_disk("user-roles").await?;
    let owner = signup(&server, "roles").await?;
    let me = get(&server, ME_PATH, &owner.cookie).await?;
    assert_eq!(me.json::<Value>()["is_admin"], json!(true));

    let second = server
        .post_json(
            CREATE_PROJECT_PATH,
            &json!({ "website": format!("shop.{}", owner.account_name) }).to_string(),
            &[("Cookie", owner.cookie.as_str())],
        )
        .await?;
    assert_eq!(second.status_code(), 200, "{}", second.body_str());
    let second_id: i32 = second.json::<Value>()["id"]
        .as_str()
        .expect("id")
        .parse()
        .expect("numeric id");
    let first_id: i32 = owner.project_id.parse().expect("numeric id");

    // A member limited to the second project.
    let limited_email = format!("limited-{}@example.com", owner.slug);
    let limited = invite_and_accept(
        &server,
        &owner.cookie,
        json!({
            "name": "Lim Ited",
            "email": limited_email,
            "is_admin": false,
            "project_ids": [second_id],
        }),
    )
    .await?;
    let limited_me = get(&server, ME_PATH, &limited.cookie)
        .await?
        .json::<Value>();
    assert_eq!(limited_me["is_admin"], json!(false));
    let names: Vec<&String> = limited_me["projects"]
        .as_object()
        .expect("projects")
        .keys()
        .collect();
    assert_eq!(names.len(), 1, "{names:?}");
    assert_eq!(
        limited_me["projects"][names[0]]["id"],
        json!(second_id.to_string())
    );

    // Reads reach only the project they were given. The rest answer 404, as
    // for a project that doesn't exist, while the session stays live.
    let overview = |id: i32| format!("/v2/read/projects/{id}/overview?start=0&end=86400");
    assert_eq!(
        get(&server, &overview(second_id), &limited.cookie)
            .await?
            .status_code(),
        200
    );
    assert_eq!(
        get(&server, &overview(first_id), &limited.cookie)
            .await?
            .status_code(),
        404
    );
    let settings = format!("/v2/session/projects/{first_id}/settings");
    assert_eq!(
        get(&server, &settings, &limited.cookie)
            .await?
            .status_code(),
        404
    );

    // Long-lived tokens follow the same scope.
    for (id, status) in [(second_id, 200), (first_id, 404)] {
        let minted = post(
            &server,
            &format!("/v2/session/projects/{id}/query-token"),
            json!({}),
            &limited.cookie,
        )
        .await?;
        assert_eq!(minted.status_code(), status, "{id}: {}", minted.body_str());
    }

    // A download token is redeemed without a session, so it is checked
    // against the holder's access when used, not just when minted.
    let download = post(
        &server,
        &format!("/v2/session/projects/{second_id}/download-token"),
        json!({}),
        &limited.cookie,
    )
    .await?;
    assert_eq!(download.status_code(), 200, "{}", download.body_str());
    let download_path = format!(
        "/v2/download/projects/{second_id}?token={}",
        download.json::<Value>()["token"].as_str().expect("token")
    );
    assert_eq!(
        server.get_query(&download_path, &[]).await?.status_code(),
        200
    );

    // Members read their projects' settings but only admins change them.
    let own_settings = format!("/v2/session/projects/{second_id}/settings");
    assert_eq!(
        get(&server, &own_settings, &limited.cookie)
            .await?
            .status_code(),
        200
    );
    let saved = post(
        &server,
        &own_settings,
        json!({ "timezone": "UTC", "payment_success_path": null, "custom_domain": null, "integrations": [] }),
        &limited.cookie,
    )
    .await?;
    assert_eq!(saved.status_code(), 403, "{}", saved.body_str());

    // Admin-only routes. Billing exists only in a build with its private module.
    #[cfg(feature = "billing")]
    assert_eq!(
        get(&server, BILLING_PATH, &limited.cookie)
            .await?
            .status_code(),
        403
    );
    assert_eq!(
        get(&server, USERS_PATH, &limited.cookie)
            .await?
            .status_code(),
        403
    );
    let member_invite = post(
        &server,
        INVITE_PATH,
        json!({ "name": "Sneaky", "email": format!("sneaky-{}@example.com", owner.slug), "is_admin": true, "project_ids": null }),
        &limited.cookie,
    )
    .await?;
    assert_eq!(
        member_invite.status_code(),
        403,
        "{}",
        member_invite.body_str()
    );
    let member_project = post(
        &server,
        CREATE_PROJECT_PATH,
        json!({ "website": format!("blog.{}", owner.account_name) }),
        &limited.cookie,
    )
    .await?;
    assert_eq!(member_project.status_code(), 403);
    let promote_self = post(
        &server,
        UPDATE_PATH,
        json!({ "user_id": limited.user_id, "is_admin": true, "project_ids": null }),
        &limited.cookie,
    )
    .await?;
    assert_eq!(promote_self.status_code(), 403);

    // Connecting an agent grants every project for a year, so only admins can.
    let client_id = register_mcp_client(&server).await?;
    let consent = post(
        &server,
        MCP_AUTHORIZE_PATH,
        json!({
            "client_id": client_id,
            "redirect_uri": "http://127.0.0.1:33418/callback",
            "code_challenge": "a".repeat(43),
            "approve": true,
        }),
        &limited.cookie,
    )
    .await?;
    assert_eq!(consent.status_code(), 403, "{}", consent.body_str());
    assert!(
        consent.body_str().contains("only an account admin"),
        "{}",
        consent.body_str()
    );

    // The admin sees everyone with their access.
    let users = get(&server, USERS_PATH, &owner.cookie).await?;
    assert_eq!(users.status_code(), 200, "{}", users.body_str());
    let users = users.json::<Value>();
    let listed = find_user(&users, &limited_email);
    assert_eq!(listed["is_admin"], json!(false));
    assert_eq!(listed["project_ids"], json!([second_id]));
    assert_eq!(listed["pending"], json!(false));

    // A project from another account can't be granted.
    let stranger = signup(&server, "rolesother").await?;
    let foreign_project: i32 = stranger.project_id.parse().expect("numeric id");
    let foreign = post(
        &server,
        UPDATE_PATH,
        json!({ "user_id": limited.user_id, "is_admin": false, "project_ids": [foreign_project] }),
        &owner.cookie,
    )
    .await?;
    assert_eq!(foreign.status_code(), 400, "{}", foreign.body_str());
    // Nor can another account's admin touch this account's users.
    for path in [UPDATE_PATH, REMOVE_PATH] {
        let response = post(
            &server,
            path,
            json!({ "user_id": limited.user_id, "is_admin": true, "project_ids": null }),
            &stranger.cookie,
        )
        .await?;
        assert_eq!(
            response.status_code(),
            404,
            "{path}: {}",
            response.body_str()
        );
    }

    // Leaving out the role is refused rather than read as a demotion.
    let roleless = post(
        &server,
        UPDATE_PATH,
        json!({ "user_id": limited.user_id, "project_ids": null }),
        &owner.cookie,
    )
    .await?;
    assert_eq!(roleless.status_code(), 400, "{}", roleless.body_str());
    // Likewise leaving out the projects isn't read as every project.
    let scopeless = post(
        &server,
        UPDATE_PATH,
        json!({ "user_id": limited.user_id, "is_admin": false }),
        &owner.cookie,
    )
    .await?;
    assert_eq!(scopeless.status_code(), 400, "{}", scopeless.body_str());

    // Widening access shows at once, without logging in again.
    let widened = post(
        &server,
        UPDATE_PATH,
        json!({ "user_id": limited.user_id, "is_admin": false, "project_ids": null }),
        &owner.cookie,
    )
    .await?;
    assert_eq!(widened.status_code(), 200, "{}", widened.body_str());
    let limited_me = get(&server, ME_PATH, &limited.cookie)
        .await?
        .json::<Value>();
    assert_eq!(
        limited_me["projects"].as_object().expect("projects").len(),
        2
    );
    assert_eq!(
        get(&server, &overview(first_id), &limited.cookie)
            .await?
            .status_code(),
        200
    );
    // Reaching every project still doesn't make a member an admin.
    let consent = post(
        &server,
        MCP_AUTHORIZE_PATH,
        json!({
            "client_id": client_id,
            "redirect_uri": "http://127.0.0.1:33418/callback",
            "code_challenge": "a".repeat(43),
            "approve": true,
        }),
        &limited.cookie,
    )
    .await?;
    assert_eq!(consent.status_code(), 403, "{}", consent.body_str());

    // Narrowing access again revokes the tokens it no longer covers.
    let minted = post(
        &server,
        &format!("/v2/session/projects/{first_id}/query-token"),
        json!({}),
        &limited.cookie,
    )
    .await?;
    assert_eq!(minted.status_code(), 200, "{}", minted.body_str());
    let bearer = format!(
        "Bearer {}",
        minted.json::<Value>()["token"].as_str().expect("token")
    );
    let schema = format!("/v2/query/projects/{first_id}/schema");
    let with_token = |path: String| {
        let bearer = bearer.clone();
        let server = &server;
        async move {
            server
                .get_query(&path, &[("Authorization", bearer.as_str())])
                .await
        }
    };
    assert_eq!(with_token(schema.clone()).await?.status_code(), 200);
    let first_download = post(
        &server,
        &format!("/v2/session/projects/{first_id}/download-token"),
        json!({}),
        &limited.cookie,
    )
    .await?;
    assert_eq!(
        first_download.status_code(),
        200,
        "{}",
        first_download.body_str()
    );
    let first_download_path = format!(
        "/v2/download/projects/{first_id}?token={}",
        first_download.json::<Value>()["token"]
            .as_str()
            .expect("token")
    );
    let narrowed = post(
        &server,
        UPDATE_PATH,
        json!({ "user_id": limited.user_id, "is_admin": false, "project_ids": [second_id] }),
        &owner.cookie,
    )
    .await?;
    assert_eq!(narrowed.status_code(), 200, "{}", narrowed.body_str());
    assert_eq!(with_token(schema).await?.status_code(), 401);
    // A download token isn't stored, so it outlives the narrowing, but it
    // names only the project its holder lost: as dead as the query token.
    // Whichever project it's presented for, so a dead token can't be told
    // apart from a live one by asking about another project.
    let other_project_path = first_download_path.replacen(
        &format!("/projects/{first_id}?"),
        &format!("/projects/{second_id}?"),
        1,
    );
    for path in [&first_download_path, &other_project_path] {
        let narrowed_download = server.get_query(path, &[]).await?;
        assert_eq!(
            narrowed_download.status_code(),
            401,
            "{path}: {}",
            narrowed_download.body_str()
        );
        assert_eq!(
            narrowed_download.json::<Value>()["error"],
            json!("invalid session"),
            "{path}"
        );
    }

    // Admins can't change or remove themselves.
    let owner_id = me.json::<Value>()["user_id"]
        .as_str()
        .expect("user_id")
        .to_string();
    for path in [UPDATE_PATH, REMOVE_PATH] {
        let response = post(
            &server,
            path,
            json!({ "user_id": owner_id, "is_admin": false, "project_ids": null }),
            &owner.cookie,
        )
        .await?;
        assert_eq!(
            response.status_code(),
            403,
            "{path}: {}",
            response.body_str()
        );
    }

    // A second admin demotes the first, who loses admin routes at once.
    let deputy = invite_and_accept(
        &server,
        &owner.cookie,
        json!({
            "name": "Dep Uty",
            "email": format!("deputy-{}@example.com", owner.slug),
            "is_admin": true, "project_ids": null,
        }),
    )
    .await?;
    assert_eq!(
        get(&server, USERS_PATH, &deputy.cookie)
            .await?
            .status_code(),
        200
    );
    #[cfg(feature = "billing")]
    assert_eq!(
        get(&server, BILLING_PATH, &deputy.cookie)
            .await?
            .status_code(),
        200
    );
    let demoted = post(
        &server,
        UPDATE_PATH,
        json!({ "user_id": owner_id, "is_admin": false, "project_ids": null }),
        &deputy.cookie,
    )
    .await?;
    assert_eq!(demoted.status_code(), 200, "{}", demoted.body_str());
    assert_eq!(
        get(&server, USERS_PATH, &owner.cookie).await?.status_code(),
        403
    );
    #[cfg(feature = "billing")]
    assert_eq!(
        get(&server, BILLING_PATH, &owner.cookie)
            .await?
            .status_code(),
        403
    );
    let owner_me = get(&server, ME_PATH, &owner.cookie).await?.json::<Value>();
    assert_eq!(owner_me["is_admin"], json!(false));
    let revenge = post(
        &server,
        UPDATE_PATH,
        json!({ "user_id": deputy.user_id, "is_admin": false, "project_ids": null }),
        &owner.cookie,
    )
    .await?;
    assert_eq!(revenge.status_code(), 403);

    // Removing a user ends their sessions.
    let removed = post(
        &server,
        REMOVE_PATH,
        json!({ "user_id": limited.user_id }),
        &deputy.cookie,
    )
    .await?;
    assert_eq!(removed.status_code(), 200, "{}", removed.body_str());
    assert_eq!(
        get(&server, ME_PATH, &limited.cookie).await?.status_code(),
        401
    );
    let stale_download = server.get_query(&download_path, &[]).await?;
    assert_eq!(
        stale_download.status_code(),
        401,
        "{}",
        stale_download.body_str()
    );
    let users = get(&server, USERS_PATH, &deputy.cookie)
        .await?
        .json::<Value>();
    assert!(
        users["users"]
            .as_array()
            .expect("users")
            .iter()
            .all(|user| user["email"] != json!(limited_email)),
        "{users}"
    );

    delete_account(&server, &owner.account_name).await;
    delete_account(&server, &stranger.account_name).await;
    Ok(())
}

#[tokio::test]
async fn an_invite_must_name_at_least_one_project_when_limited() -> io::Result<()> {
    let server = TestServer::spawn("user-roles-empty").await?;
    let owner = signup(&server, "rolesempty").await?;
    let response = post(
        &server,
        INVITE_PATH,
        json!({
            "name": "No Projects",
            "email": format!("none-{}@example.com", owner.slug),
            "is_admin": false,
            "project_ids": [],
        }),
        &owner.cookie,
    )
    .await?;
    assert_eq!(response.status_code(), 400, "{}", response.body_str());
    assert!(
        response.body_str().contains("choose at least one project"),
        "{}",
        response.body_str()
    );
    let users = get(&server, USERS_PATH, &owner.cookie)
        .await?
        .json::<Value>();
    assert_eq!(users["users"].as_array().expect("users").len(), 1);

    delete_account(&server, &owner.account_name).await;
    Ok(())
}

struct Member {
    user_id: String,
    cookie: String,
}

async fn invite_and_accept(
    server: &TestServer,
    admin_cookie: &str,
    invite: Value,
) -> io::Result<Member> {
    let email = invite["email"].as_str().expect("email").to_string();
    let response = post(server, INVITE_PATH, invite, admin_cookie).await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    let url = response.json::<Value>()["invite_url"]
        .as_str()
        .expect("invite_url")
        .to_string();
    let token = url
        .split("token=")
        .nth(1)
        .and_then(|rest| rest.split('&').next())
        .expect("token in invite url");
    let accepted = server
        .post_json(
            RESET_PASSWORD_PATH,
            &json!({ "token": token, "password": PASSWORD }).to_string(),
            &[],
        )
        .await?;
    assert_eq!(accepted.status_code(), 200, "{}", accepted.body_str());
    let login = server
        .post_json(
            LOGIN_PATH,
            &json!({ "username": email, "password": PASSWORD }).to_string(),
            &[],
        )
        .await?;
    assert_eq!(login.status_code(), 200, "{}", login.body_str());
    let user_id = login.json::<Value>()["user_id"]
        .as_str()
        .expect("user_id")
        .to_string();
    Ok(Member {
        user_id,
        cookie: cookie_pair(&login),
    })
}

fn find_user<'a>(users: &'a Value, email: &str) -> &'a Value {
    users["users"]
        .as_array()
        .expect("users")
        .iter()
        .find(|user| user["email"] == json!(email))
        .unwrap_or_else(|| panic!("{email} in {users}"))
}

async fn register_mcp_client(server: &TestServer) -> io::Result<String> {
    let response = server
        .post_json(
            MCP_REGISTER_PATH,
            &json!({
                "client_name": "Roles test client",
                "redirect_uris": ["http://127.0.0.1:33418/callback"],
                "token_endpoint_auth_method": "none",
            })
            .to_string(),
            &[],
        )
        .await?;
    assert_eq!(response.status_code(), 201, "{}", response.body_str());
    Ok(response.json::<Value>()["client_id"]
        .as_str()
        .expect("client_id")
        .to_string())
}

async fn get(server: &TestServer, path: &str, cookie: &str) -> io::Result<HttpResponse> {
    server.get_query(path, &[("Cookie", cookie)]).await
}

async fn post(
    server: &TestServer,
    path: &str,
    body: Value,
    cookie: &str,
) -> io::Result<HttpResponse> {
    server
        .post_json(path, &body.to_string(), &[("Cookie", cookie)])
        .await
}

fn cookie_pair(response: &HttpResponse) -> String {
    response
        .header("set-cookie")
        .expect("set-cookie header")
        .split(';')
        .next()
        .expect("cookie pair")
        .trim()
        .to_string()
}

async fn signup(server: &TestServer, tag: &str) -> io::Result<Account> {
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
                "plan": "growth",
                "events": "100k",
                "billing": "monthly",
            })
            .to_string(),
            &[],
        )
        .await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    let project_id = response.json::<Value>()["projects"][&project_name]["id"]
        .as_str()
        .expect("project id")
        .to_string();
    Ok(Account {
        account_name,
        slug,
        project_id,
        cookie: cookie_pair(&response),
    })
}

async fn delete_account(server: &TestServer, account_name: &str) {
    let conn = revscope::db::open_main_database_write(server.context.clone())
        .await
        .expect("open main database");
    conn.execute("DELETE FROM accounts WHERE name = $1", &[&account_name])
        .await
        .expect("delete test account");
}
