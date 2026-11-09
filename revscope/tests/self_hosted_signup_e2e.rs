#![cfg(feature = "self-hosted")]

mod common;

use std::io;
use std::time::{SystemTime, UNIX_EPOCH};

use common::TestServer;
use serde_json::{Value, json};
use tokio_postgres::NoTls;

const SIGNUP_PATH: &str = "/v2/public/auth/signup";
const LOGIN_PATH: &str = "/v2/public/auth/login";
const RESET_PASSWORD_PATH: &str = "/v2/public/auth/reset-password";
const ME_PATH: &str = "/v2/session/me";
const USERS_PATH: &str = "/v2/session/users";
const INVITE_PATH: &str = "/v2/session/users/invite";
/// What the signup form sends, as if the user had edited the name derived
/// from the website.
const FIRST_PROJECT_NAME: &str = "my_site";
/// The seeded demo account, standing in for any account other than the shared one.
const OTHER_ACCOUNT_ID: &str = "c5460626-00b6-4d98-b95d-ac0bacc24296";
const PASSWORD: &str = "a-long-enough-password";

/// One test, because signup is only open while the instance has no users.
#[tokio::test]
async fn first_user_signs_up_and_invites_the_rest() -> io::Result<()> {
    let scratch = ScratchSchema::create().await;
    let server = TestServer::spawn("self-hosted-signup").await?;
    let conn = revscope::db::open_main_database_write(server.context.clone())
        .await
        .expect("main db");
    assert!(signup_open(&server).await?);

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let racers = [
        format!("racer-a-{unique}@example.com"),
        format!("racer-b-{unique}@example.com"),
    ];
    let invitee = format!("invitee-{unique}@example.com");

    // No plan, events, or billing: none apply when self-hosting.
    let signup_body = |email: &str| {
        json!({
            "name": "Ada Lovelace",
            "email": email,
            "password": PASSWORD,
            "account_name": "www.example.com",
            "project_name": FIRST_PROJECT_NAME,
            "timezone": "Europe/London",
        })
        .to_string()
    };

    // Any user closes signup, through the endpoint and the signup itself, even
    // one in an account that isn't the shared one.
    conn.execute(
        "INSERT INTO users (account_id, name, email) VALUES ($1::text::uuid, 'Other', 'other@example.com')",
        &[&OTHER_ACCOUNT_ID],
    )
    .await
    .expect("insert other user");
    assert!(!signup_open(&server).await?);
    let blocked = server
        .post_json(SIGNUP_PATH, &signup_body(&racers[0]), &[])
        .await?;
    assert_eq!(blocked.status_code(), 403, "{}", blocked.body_str());
    conn.execute("DELETE FROM users", &[])
        .await
        .expect("delete other user");
    assert!(signup_open(&server).await?);

    // A project by that name that another account owns is refused before
    // anything is written.
    conn.execute(
        "INSERT INTO projects (account_id, name) VALUES ($1::text::uuid, $2)",
        &[&OTHER_ACCOUNT_ID, &FIRST_PROJECT_NAME],
    )
    .await
    .expect("insert foreign project");
    let foreign = server
        .post_json(SIGNUP_PATH, &signup_body(&racers[0]), &[])
        .await?;
    assert_eq!(foreign.status_code(), 409, "{}", foreign.body_str());
    assert_eq!(
        foreign.json::<Value>()["error"],
        json!("A project with this name already exists. Try another name.")
    );
    let shared_accounts: i64 = conn
        .query_one(
            "SELECT COUNT(*) FROM accounts WHERE name = 'self-hosted'",
            &[],
        )
        .await
        .expect("count shared accounts")
        .get(0);
    assert_eq!(shared_accounts, 0, "a refused signup writes nothing");
    conn.execute(
        "DELETE FROM projects WHERE name = $1",
        &[&FIRST_PROJECT_NAME],
    )
    .await
    .expect("delete foreign project");

    // A first signup that fails after its project is written leaves nothing,
    // so a retry under another name doesn't keep the first name's project. The
    // name "é" passes auth.rs's length check, which counts bytes, but fails the
    // users table's, which counts characters, so the user insert fails after
    // the project's. Fixing that check turns this into a 400 that writes
    // nothing, and then this needs another way to fail late.
    let failed = server
        .post_json(
            SIGNUP_PATH,
            &json!({
                "name": "é",
                "email": racers[0],
                "password": PASSWORD,
                "project_name": "abandoned",
            })
            .to_string(),
            // Its own address, since a failed signup uses up one of the
            // address's few signups and the rest of this test needs them.
            &[("CF-Connecting-IP", "203.0.113.70")],
        )
        .await?;
    assert_eq!(failed.status_code(), 500, "{}", failed.body_str());
    let left_behind: i64 = conn
        .query_one(
            "SELECT (SELECT COUNT(*) FROM projects WHERE name = 'abandoned')
                  + (SELECT COUNT(*) FROM accounts WHERE name = 'self-hosted')",
            &[],
        )
        .await
        .expect("count left-behind rows")
        .get(0);
    assert_eq!(left_behind, 0, "a failed signup writes nothing");

    // An older version's failed signup, which wasn't one transaction, could
    // leave the shared account without its subscription period; the next
    // signup reuses the account and opens the period (checked below).
    conn.execute(
        "INSERT INTO accounts (name, trial_ends_at) VALUES ('self-hosted', NULL)",
        &[],
    )
    .await
    .expect("insert leftover account");
    let (body_a, body_b) = (signup_body(&racers[0]), signup_body(&racers[1]));
    let (first, second) = tokio::join!(
        server.post_json(SIGNUP_PATH, &body_a, &[]),
        server.post_json(SIGNUP_PATH, &body_b, &[]),
    );
    let (first, second) = (first?, second?);
    let (winner, loser) = if first.status_code() == 200 {
        (first, second)
    } else {
        (second, first)
    };
    assert_eq!(winner.status_code(), 200, "{}", winner.body_str());
    assert!(
        matches!(loser.status_code(), 403 | 503),
        "only one racing first signup gets in: {} {}",
        loser.status_code(),
        loser.body_str()
    );
    let owner_cookie = winner.header("set-cookie").expect("session cookie");
    let owner_account = session_account(&server, owner_cookie).await?;
    let timezone: String = conn
        .query_one(
            "SELECT s.timezone FROM project_settings s
             INNER JOIN projects p ON p.id = s.project_id
             WHERE p.name = $1",
            &[&FIRST_PROJECT_NAME],
        )
        .await
        .expect("first project timezone")
        .get(0);
    assert_eq!(timezone, "Europe/London");

    let closed = server
        .post_json(SIGNUP_PATH, &signup_body(&invitee), &[])
        .await?;
    assert_eq!(closed.status_code(), 403, "{}", closed.body_str());
    assert!(!signup_open(&server).await?);

    let invite_body =
        json!({ "name": "Grace Hopper", "email": invitee, "is_admin": false, "project_ids": null })
            .to_string();
    let invite = server
        .post_json(INVITE_PATH, &invite_body, &[("Cookie", owner_cookie)])
        .await?;
    assert_eq!(invite.status_code(), 200, "{}", invite.body_str());
    let first_token = invite_token(&invite.json());

    // Inviting again replaces the link.
    let reinvite = server
        .post_json(INVITE_PATH, &invite_body, &[("Cookie", owner_cookie)])
        .await?;
    assert_eq!(reinvite.status_code(), 200, "{}", reinvite.body_str());
    let token = invite_token(&reinvite.json());
    assert_ne!(first_token, token);

    let users: Value = server
        .get_query(USERS_PATH, &[("Cookie", owner_cookie)])
        .await?
        .json();
    let pending: Vec<(&str, bool)> = users["users"]
        .as_array()
        .expect("users array")
        .iter()
        .map(|user| {
            (
                user["email"].as_str().expect("email"),
                user["pending"].as_bool().expect("pending"),
            )
        })
        .collect();
    assert_eq!(pending.len(), 2);
    assert!(pending.contains(&(invitee.as_str(), true)));

    let login_body = json!({ "username": invitee, "password": PASSWORD }).to_string();
    let early_login = server.post_json(LOGIN_PATH, &login_body, &[]).await?;
    assert_eq!(
        early_login.status_code(),
        401,
        "no login before accepting the invite"
    );

    let stale = server
        .post_json(
            RESET_PASSWORD_PATH,
            &json!({ "token": first_token, "password": PASSWORD }).to_string(),
            &[],
        )
        .await?;
    assert_eq!(stale.status_code(), 400, "{}", stale.body_str());
    let accept = server
        .post_json(
            RESET_PASSWORD_PATH,
            &json!({ "token": token, "password": PASSWORD }).to_string(),
            &[],
        )
        .await?;
    assert_eq!(accept.status_code(), 200, "{}", accept.body_str());

    let login = server.post_json(LOGIN_PATH, &login_body, &[]).await?;
    assert_eq!(login.status_code(), 200, "{}", login.body_str());
    let invitee_cookie = login.header("set-cookie").expect("session cookie");
    assert_eq!(
        session_account(&server, invitee_cookie).await?,
        owner_account,
        "the invitee joins the shared account"
    );

    // The first user administers the instance; invitees are members unless
    // made admins, so only the owner may invite.
    assert!(is_admin(&server, owner_cookie).await?);
    assert!(!is_admin(&server, invitee_cookie).await?);
    let member_invite = server
        .post_json(INVITE_PATH, &invite_body, &[("Cookie", invitee_cookie)])
        .await?;
    assert_eq!(
        member_invite.status_code(),
        403,
        "{}",
        member_invite.body_str()
    );
    let taken = server
        .post_json(INVITE_PATH, &invite_body, &[("Cookie", owner_cookie)])
        .await?;
    assert_eq!(taken.status_code(), 409, "{}", taken.body_str());

    // A Google user has no password either, but isn't a pending invite: a
    // teammate re-inviting them would get a reset link to their account.
    let google_user = format!("google-{unique}@example.com");
    conn.execute(
        "WITH u AS (
            INSERT INTO users (account_id, name, email)
            VALUES ($1::text::uuid, 'Grace Hopper', $2)
            RETURNING id
         )
         INSERT INTO user_identities (provider, subject, user_id)
         SELECT 'google', $2, id FROM u",
        &[&owner_account, &google_user],
    )
    .await
    .expect("insert google user");
    let reinvite_google = server
        .post_json(
            INVITE_PATH,
            &json!({
                "name": "Grace Hopper",
                "email": google_user,
                "is_admin": false,
                "project_ids": null,
            })
            .to_string(),
            &[("Cookie", owner_cookie)],
        )
        .await?;
    assert_eq!(
        reinvite_google.status_code(),
        409,
        "{}",
        reinvite_google.body_str()
    );
    let users: Value = server
        .get_query(USERS_PATH, &[("Cookie", owner_cookie)])
        .await?
        .json();
    let google_pending = users["users"]
        .as_array()
        .expect("users array")
        .iter()
        .find(|user| user["email"] == json!(google_user))
        .map(|user| user["pending"].clone());
    assert_eq!(google_pending, Some(json!(false)));

    // The account may add projects and delete them, down to its last.
    let added = server
        .post_json(
            "/v2/session/projects",
            &json!({ "website": format!("extra{unique}") }).to_string(),
            &[("Cookie", owner_cookie)],
        )
        .await?;
    assert_eq!(added.status_code(), 200, "{}", added.body_str());
    let added: Value = added.json();
    let me: Value = server
        .get_query(ME_PATH, &[("Cookie", owner_cookie)])
        .await?
        .json();
    let first_id = me["projects"][FIRST_PROJECT_NAME]["id"]
        .as_str()
        .expect("first project id");
    let removed = server
        .delete_json(
            &format!("/v2/session/projects/{first_id}"),
            &json!({ "confirm_name": FIRST_PROJECT_NAME }).to_string(),
            &[("Cookie", owner_cookie)],
        )
        .await?;
    assert_eq!(removed.status_code(), 200, "{}", removed.body_str());
    assert!(!signup_open(&server).await?);
    let refused = server
        .delete_json(
            &format!("/v2/session/projects/{}", added["id"].as_str().expect("id")),
            &json!({ "confirm_name": added["name"] }).to_string(),
            &[("Cookie", owner_cookie)],
        )
        .await?;
    assert_eq!(refused.status_code(), 409, "{}", refused.body_str());

    let period = conn
        .query_one(
            "SELECT plan_type::text, events_monthly_limit, actual_price_cents
             FROM account_subscriptions
             WHERE account_id = $1::text::uuid AND ended_at IS NULL",
            &[&owner_account],
        )
        .await
        .expect("open subscription period");
    assert_eq!(period.get::<_, String>(0), "enterprise");
    assert_eq!(period.get::<_, Option<i32>>(1), None, "no event limit");
    assert_eq!(period.get::<_, i32>(2), 0);

    drop(conn);
    drop(server);
    drop(scratch);
    Ok(())
}

/// The session's account id, checking it has just the project signup created.
async fn session_account(server: &TestServer, cookie: &str) -> io::Result<String> {
    let me = server.get_query(ME_PATH, &[("Cookie", cookie)]).await?;
    assert_eq!(me.status_code(), 200, "{}", me.body_str());
    let me: Value = me.json();
    let projects = me["projects"].as_object().expect("projects object");
    assert_eq!(
        projects.keys().collect::<Vec<_>>(),
        vec![FIRST_PROJECT_NAME]
    );
    Ok(me["account_id"].as_str().expect("account id").to_string())
}

async fn is_admin(server: &TestServer, cookie: &str) -> io::Result<bool> {
    let me: Value = server
        .get_query(ME_PATH, &[("Cookie", cookie)])
        .await?
        .json();
    Ok(me["is_admin"].as_bool().expect("is_admin"))
}

fn invite_token(response: &Value) -> String {
    let url = response["invite_url"].as_str().expect("invite url");
    assert!(url.contains("/reset-password?token="), "{url}");
    assert!(url.ends_with("&invite=1"), "{url}");
    url.split("token=")
        .nth(1)
        .and_then(|rest| rest.split('&').next())
        .expect("token in invite url")
        .to_string()
}

async fn signup_open(server: &TestServer) -> io::Result<bool> {
    let response = server.get_query(SIGNUP_PATH, &[]).await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    let body: Value = response.json();
    Ok(body["open"].as_bool().expect("open"))
}

/// A new, empty schema that the test server uses as its main database in
/// place of the shared e2e one, where other tests' users would keep signup
/// closed. A schema rather than a database, since the e2e role may not be
/// allowed to create databases.
struct ScratchSchema {
    admin_url: String,
    name: String,
}

impl ScratchSchema {
    async fn create() -> Self {
        revscope::test_env::load_e2e_env();
        let admin_url = std::env::var("DATABASE_URL")
            .expect("DATABASE_URL must be set to run e2e tests (export it or add it to .e2e.env)");
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let name = format!("self_hosted_e2e_{}_{nanos}", std::process::id());
        admin_execute(&admin_url, &format!("CREATE SCHEMA {name}")).await;
        // SAFETY: this binary's one test sets it before anything reads it.
        unsafe { std::env::set_var("DATABASE_URL", with_search_path(&admin_url, &name)) };
        Self { admin_url, name }
    }
}

/// Runs on panic too, so a failing test doesn't leave its schema behind. On its
/// own thread and runtime because `Drop` can't await and the test's runtime may
/// be unwinding.
impl Drop for ScratchSchema {
    fn drop(&mut self) {
        let admin_url = self.admin_url.clone();
        let sql = format!("DROP SCHEMA {} CASCADE", self.name);
        let dropped = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime")
                .block_on(admin_execute(&admin_url, &sql));
        })
        .join();
        if dropped.is_err() {
            eprintln!("failed to drop scratch schema {}", self.name);
        }
    }
}

async fn admin_execute(admin_url: &str, sql: &str) {
    let (client, connection) = tokio_postgres::connect(admin_url, NoTls)
        .await
        .expect("connect to the e2e postgres server");
    tokio::spawn(connection);
    client.batch_execute(sql).await.expect(sql);
}

/// `url` with every connection's `search_path` set to `schema`. Only URL-form
/// connection strings (`postgres://…`) are supported.
fn with_search_path(url: &str, schema: &str) -> String {
    assert!(url.contains("://"), "DATABASE_URL must be a URL");
    let (base, query) = url.split_once('?').unwrap_or((url, ""));
    // A worktree's URL selects that worktree's schema, which this replaces.
    let mut params: Vec<&str> = query
        .split('&')
        .filter(|param| !param.is_empty() && !selects_only_a_schema(param))
        .collect();
    assert!(
        !params.iter().any(|param| param.starts_with("options=")),
        "DATABASE_URL already sets other options, which this would override"
    );
    let search_path = format!("options=-csearch_path%3D{schema}");
    params.push(&search_path);
    format!("{base}?{}", params.join("&"))
}

fn selects_only_a_schema(param: &str) -> bool {
    param
        .strip_prefix("options=-csearch_path%3D")
        .is_some_and(|schema| {
            schema
                .chars()
                .all(|char| char.is_ascii_alphanumeric() || char == '_')
        })
}
