// Hosted signup creates a per-website account with a plan; self-hosted signup does not.
#![cfg(not(feature = "self-hosted"))]

mod common;

use std::io;

use common::signup::{
    NewSignup, PASSWORD, SIGNUP_PATH, assert_rejected, delete_accounts, fetch_account, main_db,
};
use common::{HttpResponse, TestServer, TrackHit, count_rows, page_id, query_string};
use serde_json::{Value, json};

const LOGIN_PATH: &str = "/v2/public/auth/login";
const LOGOUT_PATH: &str = "/v2/public/auth/logout";
const ME_PATH: &str = "/v2/session/me";

#[tokio::test]
async fn signup_creates_account_project_user_and_session() -> io::Result<()> {
    let server = TestServer::spawn("signup-happy-path").await?;
    let signup = NewSignup::new("happy");

    let response = server.post_json(SIGNUP_PATH, &signup.body(), &[]).await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());

    let body: Value = response.json();
    assert_eq!(body["name"], json!("Ada Lovelace"));
    assert_eq!(body["email"], json!(signup.email));
    let gravatar_url = body["gravatar_url"].as_str().expect("gravatar url");
    let gravatar_hash = gravatar_url
        .strip_prefix("https://www.gravatar.com/avatar/")
        .and_then(|rest| rest.strip_suffix("?d=mp&s=80"))
        .unwrap_or_default();
    assert_eq!(
        gravatar_hash.len(),
        32,
        "unexpected gravatar url {gravatar_url}"
    );

    let account = fetch_account(&server, &signup.account_name)
        .await
        .expect("signup must create an account");
    assert_eq!(body["account_id"], json!(account.id));
    assert_eq!(body["plan"], json!(account.plan));
    assert!(
        account.trial_ends_at_is_null,
        "trials only start once payments are enabled"
    );

    let project = fetch_project(&server, &signup.project_name)
        .await
        .expect("signup must create a project for the website");
    assert_eq!(project.account_id, account.id);
    let projects = body["projects"].as_object().expect("projects object");
    let project_entry = projects
        .get(&signup.project_name)
        .and_then(Value::as_object)
        .expect("project entry object");
    assert_eq!(
        project_entry.get("id").and_then(Value::as_str),
        Some(project.id.to_string().as_str()),
        "signup response must map the project name to its id"
    );
    assert_eq!(
        project_entry
            .get("settings")
            .and_then(|settings| settings.get("timezone"))
            .and_then(Value::as_str),
        Some("UTC"),
        "new projects default to UTC timezone"
    );
    assert_eq!(
        fetch_project_timezone(&server, project.id).await.as_deref(),
        Some("UTC"),
        "signup inserts project_settings even when no timezone is sent"
    );
    assert_eq!(projects.len(), 1, "signup creates a single project");

    let user = fetch_user(&server, &signup.email)
        .await
        .expect("signup must create a user");
    assert_eq!(body["user_id"], json!(user.id));
    assert_eq!(user.account_id, account.id);
    assert_eq!(user.name, "Ada Lovelace");
    assert_eq!(
        user.original_email, None,
        "lowercase email needs no original"
    );
    assert!(user.has_password, "password must be hashed and stored");
    assert!(user.has_last_login_at, "signup counts as a login");

    assert!(
        body.get("session_id").is_none(),
        "the session belongs in the cookie, not the body: {body}"
    );
    let session_id = session_id_from_cookie(&response);
    assert_eq!(
        session_ids_for_user(&server, &user.id, &[session_id.as_str()]).await,
        [session_id.as_str()],
        "only the hash of the cookie's token is stored"
    );

    delete_accounts(&server, &[&signup.account_name]).await;
    Ok(())
}

#[tokio::test]
async fn signup_stores_the_browser_timezone_on_the_new_project() -> io::Result<()> {
    let server = TestServer::spawn("signup-timezone").await?;
    let mut signup = NewSignup::new("tzok");
    signup.timezone = Some("America/New_York".to_string());

    let response = server.post_json(SIGNUP_PATH, &signup.body(), &[]).await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());

    let body: Value = response.json();
    let timezone = body["projects"][&signup.project_name]["settings"]["timezone"]
        .as_str()
        .expect("timezone in signup response");
    assert_eq!(timezone, "America/New_York");

    let project = fetch_project(&server, &signup.project_name)
        .await
        .expect("signup must create a project");
    assert_eq!(
        fetch_project_timezone(&server, project.id).await.as_deref(),
        Some("America/New_York")
    );

    delete_accounts(&server, &[&signup.account_name]).await;
    Ok(())
}

#[tokio::test]
async fn signup_falls_back_to_utc_when_the_timezone_is_invalid() -> io::Result<()> {
    let server = TestServer::spawn("signup-bad-timezone").await?;
    let mut accounts = Vec::new();

    let too_long = "x".repeat(65);
    for (tag, timezone) in [("empty", ""), ("blank", "   "), ("long", too_long.as_str())] {
        let mut signup = NewSignup::new(tag);
        signup.timezone = Some(timezone.to_string());
        let response = server.post_json(SIGNUP_PATH, &signup.body(), &[]).await?;
        assert_eq!(
            response.status_code(),
            200,
            "{tag}: {}",
            response.body_str()
        );
        assert_eq!(
            response.json::<Value>()["projects"][&signup.project_name]["settings"]["timezone"],
            json!("UTC"),
            "{tag} must not block signup"
        );
        let project = fetch_project(&server, &signup.project_name)
            .await
            .expect("project");
        assert_eq!(
            fetch_project_timezone(&server, project.id).await.as_deref(),
            Some("UTC"),
            "{tag}"
        );
        accounts.push(signup.account_name);
    }

    let names: Vec<&str> = accounts.iter().map(String::as_str).collect();
    delete_accounts(&server, &names).await;
    Ok(())
}

#[tokio::test]
async fn signup_creates_a_project_database_that_accepts_tracking_hits() -> io::Result<()> {
    let mut server = TestServer::spawn("signup-project-db").await?;
    let signup = NewSignup::new("projectdb");

    let response = server.post_json(SIGNUP_PATH, &signup.body(), &[]).await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    let project = fetch_project(&server, &signup.project_name)
        .await
        .expect("signup must create a project");

    let hit = TrackHit::pageview(&signup.project_name, "/pricing", &page_id(1));
    let ingest = server
        .post_ingest_for(&signup.project_name, &hit.to_form(), &[])
        .await?;
    assert_eq!(ingest.status_code(), 200, "{}", ingest.body_str());
    server.flush().await;

    let conn = server.project_db(&project.id.to_string());
    assert_eq!(count_rows(&conn, "pageviews"), 1);
    assert_eq!(query_string(&conn, "SELECT path FROM paths"), "/pricing");

    delete_accounts(&server, &[&signup.account_name]).await;
    Ok(())
}

#[tokio::test]
async fn signup_session_authenticates_subsequent_api_requests() -> io::Result<()> {
    let server = TestServer::spawn("signup-session").await?;
    let signup = NewSignup::new("session");

    let response = server.post_json(SIGNUP_PATH, &signup.body(), &[]).await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    let signed_up: Value = response.json();
    let session_id = session_id_from_cookie(&response);

    // The signup body is already what /v2/session/me answers, so a client that just
    // authenticated has everything it needs without a second round trip.
    let cookie = format!("rr_session={session_id}");
    let by_cookie = server
        .get_query(ME_PATH, &[("Cookie", cookie.as_str())])
        .await?;
    assert_eq!(by_cookie.status_code(), 200, "{}", by_cookie.body_str());
    assert_eq!(by_cookie.json::<Value>(), signed_up);

    let authorization = format!("Bearer {session_id}");
    let by_bearer = server
        .get_query(ME_PATH, &[("Authorization", authorization.as_str())])
        .await?;
    assert_eq!(by_bearer.status_code(), 200, "{}", by_bearer.body_str());
    let me: Value = by_bearer.json();
    assert_eq!(me["user_id"], signed_up["user_id"]);
    assert_eq!(me["email"], json!(signup.email));
    assert_eq!(me["plan"], signed_up["plan"]);

    delete_accounts(&server, &[&signup.account_name]).await;
    Ok(())
}

#[tokio::test]
async fn logout_deletes_the_session_and_clears_the_cookie() -> io::Result<()> {
    let server = TestServer::spawn("logout").await?;
    let signup = NewSignup::new("logout");

    let response = server.post_json(SIGNUP_PATH, &signup.body(), &[]).await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    let session_id = session_id_from_cookie(&response);
    let cookie = format!("rr_session={session_id}");

    let logout = server
        .post_json(LOGOUT_PATH, "", &[("Cookie", cookie.as_str())])
        .await?;
    assert_eq!(logout.status_code(), 200, "{}", logout.body_str());
    let cleared = logout.header("set-cookie").expect("set-cookie");
    assert!(cleared.contains("rr_session=;"), "{cleared}");
    assert!(cleared.contains("Max-Age=0"), "{cleared}");

    let user = fetch_user(&server, &signup.email).await.expect("user");
    assert!(
        session_ids_for_user(&server, &user.id, &[])
            .await
            .is_empty(),
        "logout must delete the session so the cookie cannot be replayed"
    );
    let replayed = server
        .get_query(ME_PATH, &[("Cookie", cookie.as_str())])
        .await?;
    assert_eq!(replayed.status_code(), 401, "{}", replayed.body_str());

    delete_accounts(&server, &[&signup.account_name]).await;
    Ok(())
}

/// The session id the API just set, checked for the attributes that keep it out
/// of scripts and off cross-site requests. These helpers hit a non-loopback
/// `Host`, so the cookie must carry `Secure` the way production does.
fn session_id_from_cookie(response: &HttpResponse) -> String {
    let cookie = response.header("set-cookie").expect("set-cookie header");
    assert!(cookie.contains("HttpOnly"), "{cookie}");
    assert!(cookie.contains("SameSite=Strict"), "{cookie}");
    assert!(cookie.contains("Path=/"), "{cookie}");
    assert!(cookie.contains("Secure"), "{cookie}");
    cookie
        .split(';')
        .next()
        .and_then(|pair| pair.trim().strip_prefix("rr_session="))
        .expect("session cookie value")
        .to_string()
}

#[tokio::test]
async fn login_omits_secure_cookie_flag_on_localhost() -> io::Result<()> {
    let server = TestServer::spawn("login-localhost-cookie").await?;
    let signup = NewSignup::new("localcookie");

    let created = server.post_json(SIGNUP_PATH, &signup.body(), &[]).await?;
    assert_eq!(created.status_code(), 200, "{}", created.body_str());

    // A second Host header wins in our parser; this mimics the browser talking
    // to the API as http://localhost during local development.
    let login = server
        .post_json(
            LOGIN_PATH,
            &json!({ "username": signup.email, "password": PASSWORD }).to_string(),
            &[("Host", "localhost:4320")],
        )
        .await?;
    assert_eq!(login.status_code(), 200, "{}", login.body_str());
    let cookie = login.header("set-cookie").expect("set-cookie");
    assert!(cookie.contains("HttpOnly"), "{cookie}");
    assert!(
        !cookie.contains("Secure"),
        "localhost must not set Secure or the browser drops the cookie: {cookie}"
    );

    delete_accounts(&server, &[&signup.account_name]).await;
    Ok(())
}

#[tokio::test]
async fn signup_normalizes_the_email_and_login_accepts_any_case() -> io::Result<()> {
    let server = TestServer::spawn("signup-email-case").await?;
    let mut signup = NewSignup::new("emailcase");
    let original_email = format!("Ada.{}@Example.COM", signup.slug);
    let normalized_email = original_email.to_ascii_lowercase();
    signup.email = original_email.clone();

    let response = server.post_json(SIGNUP_PATH, &signup.body(), &[]).await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    assert_eq!(response.json::<Value>()["email"], json!(original_email));

    let user = fetch_user(&server, &normalized_email)
        .await
        .expect("email must be stored lowercase");
    assert_eq!(
        user.original_email.as_deref(),
        Some(original_email.as_str())
    );

    // The password salt uses the address as typed, so both spellings must work.
    for username in [original_email.as_str(), normalized_email.as_str()] {
        let login = server
            .post_json(
                LOGIN_PATH,
                &json!({ "username": username, "password": PASSWORD }).to_string(),
                &[],
            )
            .await?;
        assert_eq!(login.status_code(), 200, "{username}: {}", login.body_str());
        assert_eq!(login.json::<Value>()["user_id"], json!(user.id));
    }

    delete_accounts(&server, &[&signup.account_name]).await;
    Ok(())
}

/// Without the private billing module there is no catalog, so whatever plan
/// the form names, the account is complimentary.
#[cfg(not(feature = "billing"))]
#[tokio::test]
async fn signup_without_billing_is_complimentary_whatever_the_plan() -> io::Result<()> {
    let server = TestServer::spawn("signup-complimentary").await?;
    let mut signup = NewSignup::new("complimentary");
    signup.plan = "no-such-plan".to_string();
    signup.events = "42k".to_string();

    let response = server.post_json(SIGNUP_PATH, &signup.body(), &[]).await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    assert_eq!(response.json::<Value>()["plan"], json!("enterprise"));
    let account = fetch_account(&server, &signup.account_name)
        .await
        .expect("account");
    assert_eq!(account.plan, "enterprise");
    assert_eq!(account.events_monthly_limit, None);
    assert_eq!(account.regular_price_cents, 0);
    assert_eq!(account.actual_price_cents, 0);

    delete_accounts(&server, &[&signup.account_name]).await;
    Ok(())
}

#[tokio::test]
async fn signup_rejects_a_duplicate_email_and_rolls_back_the_account() -> io::Result<()> {
    let server = TestServer::spawn("signup-duplicate-email").await?;
    let first = NewSignup::new("dupemail");
    let response = server.post_json(SIGNUP_PATH, &first.body(), &[]).await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());

    let mut second = NewSignup::new("dupemailtwo");
    second.email = first.email.to_ascii_uppercase();
    let response = server.post_json(SIGNUP_PATH, &second.body(), &[]).await?;
    assert_eq!(response.status_code(), 409, "{}", response.body_str());
    assert_eq!(
        response.json::<Value>()["error"],
        json!("An account with this email already exists. Log in instead.")
    );
    assert!(
        fetch_account(&server, &second.account_name).await.is_none(),
        "the rejected signup must not leave an account behind"
    );
    assert!(
        fetch_project(&server, &second.project_name).await.is_none(),
        "the rejected signup must not leave a project behind"
    );

    delete_accounts(&server, &[&first.account_name, &second.account_name]).await;
    Ok(())
}

#[tokio::test]
async fn signup_rejects_a_duplicate_website() -> io::Result<()> {
    let server = TestServer::spawn("signup-duplicate-website").await?;
    let first = NewSignup::new("dupsite");
    let response = server.post_json(SIGNUP_PATH, &first.body(), &[]).await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());

    let mut second = NewSignup::new("dupsitetwo");
    second.account_name = first.account_name.clone();
    let response = server.post_json(SIGNUP_PATH, &second.body(), &[]).await?;
    assert_eq!(response.status_code(), 409, "{}", response.body_str());
    assert_eq!(
        response.json::<Value>()["error"],
        json!("An account with this website already exists.")
    );
    assert!(
        fetch_user(&server, &second.email).await.is_none(),
        "the rejected signup must not leave a user behind"
    );

    delete_accounts(&server, &[&first.account_name]).await;
    Ok(())
}

#[tokio::test]
async fn signup_rejects_a_website_that_maps_to_a_taken_project_name() -> io::Result<()> {
    let server = TestServer::spawn("signup-duplicate-project").await?;
    let first = NewSignup::new("dupproject");
    let response = server.post_json(SIGNUP_PATH, &first.body(), &[]).await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());

    // A different website name that still reduces to the same project name.
    let mut second = NewSignup::new("dupprojecttwo");
    second.account_name = format!("{}.Example", first.slug);
    let response = server.post_json(SIGNUP_PATH, &second.body(), &[]).await?;
    assert_eq!(response.status_code(), 409, "{}", response.body_str());
    assert_eq!(
        response.json::<Value>()["error"],
        json!("A project with this name already exists. Try another name.")
    );
    assert!(
        fetch_account(&server, &second.account_name).await.is_none(),
        "the account must be rolled back when the project insert fails"
    );

    delete_accounts(&server, &[&first.account_name, &second.account_name]).await;
    Ok(())
}

#[tokio::test]
async fn signup_names_the_project_as_the_form_sent_it() -> io::Result<()> {
    let server = TestServer::spawn("signup-chosen-project-name").await?;
    let mut signup = NewSignup::new("chosenname");
    let chosen = format!("{}_shop", signup.slug);
    signup.requested_project_name = Some(chosen.clone());
    let response = server.post_json(SIGNUP_PATH, &signup.body(), &[]).await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());

    let account = fetch_account(&server, &signup.account_name)
        .await
        .expect("account");
    let project = fetch_project(&server, &chosen)
        .await
        .expect("the project has the name the form sent");
    assert_eq!(project.account_id, account.id);
    assert!(
        fetch_project(&server, &signup.project_name).await.is_none(),
        "no project under the derived name"
    );

    let mut invalid = NewSignup::new("badchosen");
    invalid.requested_project_name = Some("My-Shop".to_string());
    assert_rejected(
        &server,
        &invalid,
        "Project name must be at least 3 lowercase letters, numbers or underscores.",
    )
    .await?;
    let mut reserved = NewSignup::new("reservedchosen");
    reserved.requested_project_name = Some("demo".to_string());
    assert_rejected(
        &server,
        &reserved,
        "This project name is reserved. Try a different name.",
    )
    .await?;

    delete_accounts(&server, &[&signup.account_name]).await;
    Ok(())
}

#[tokio::test]
async fn trying_taken_project_names_does_not_use_up_signups() -> io::Result<()> {
    let server = TestServer::spawn("signup-taken-name-refund").await?;
    let first = NewSignup::new("takenrefund");
    let response = server.post_json(SIGNUP_PATH, &first.body(), &[]).await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());

    // More tries than an address gets signups, all from one address.
    let address = [("CF-Connecting-IP", "203.0.113.80")];
    for _ in 0..6 {
        let mut taken = NewSignup::new("takenrefundtry");
        taken.requested_project_name = Some(first.project_name.clone());
        let response = server
            .post_json(SIGNUP_PATH, &taken.body(), &address)
            .await?;
        assert_eq!(response.status_code(), 409, "{}", response.body_str());
        assert!(
            fetch_account(&server, &taken.account_name).await.is_none(),
            "a taken name is refused before anything is written"
        );
    }
    let fixed = NewSignup::new("takenrefundfixed");
    let response = server
        .post_json(SIGNUP_PATH, &fixed.body(), &address)
        .await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());

    delete_accounts(&server, &[&first.account_name, &fixed.account_name]).await;
    Ok(())
}

#[tokio::test]
async fn signup_strips_scheme_and_path_from_the_website() -> io::Result<()> {
    let server = TestServer::spawn("signup-normalize-website").await?;
    let mut signup = NewSignup::new("stripurl");
    let expected_website = signup.account_name.clone();
    signup.account_name = format!("https://{expected_website}/getting-started?ref=1#top");

    let response = server.post_json(SIGNUP_PATH, &signup.body(), &[]).await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());

    let account = fetch_account(&server, &expected_website)
        .await
        .expect("website must be stored without scheme or path");
    assert!(
        fetch_account(&server, &signup.account_name).await.is_none(),
        "raw URL must not be stored as the account name"
    );

    let project = fetch_project(&server, &signup.project_name)
        .await
        .expect("project name still derives from the host");
    assert_eq!(project.account_id, account.id);

    delete_accounts(&server, &[&expected_website]).await;
    Ok(())
}

#[tokio::test]
async fn signup_rejects_invalid_fields_with_a_message_for_the_form() -> io::Result<()> {
    let server = TestServer::spawn("signup-validation").await?;

    let mut short_name = NewSignup::new("badname");
    short_name.user_name = "A".to_string();
    assert_rejected(&server, &short_name, "name must be at least 2 characters").await?;

    let mut short_website = NewSignup::new("badwebsite");
    short_website.account_name = "ab".to_string();
    assert_rejected(
        &server,
        &short_website,
        "Website must be at least 5 characters.",
    )
    .await?;

    let mut no_domain_website = NewSignup::new("nodomain");
    no_domain_website.account_name = "mywebsite".to_string();
    assert_rejected(
        &server,
        &no_domain_website,
        "Website must include a domain (e.g. mywebsite.com).",
    )
    .await?;

    let mut few_letters_website = NewSignup::new("fewletters");
    few_letters_website.account_name = "a---.b".to_string();
    assert_rejected(
        &server,
        &few_letters_website,
        "Website must contain at least 3 letters or numbers for the project name.",
    )
    .await?;

    let mut reserved_website = NewSignup::new("reserved");
    // Derives to project name "demo" while still looking like a domain.
    reserved_website.account_name = "de.mo".to_string();
    assert_rejected(
        &server,
        &reserved_website,
        "This project name is reserved. Try a different name.",
    )
    .await?;

    let mut short_password = NewSignup::new("badpassword");
    short_password.password = "too short".to_string();
    assert_rejected(
        &server,
        &short_password,
        "password must be at least 10 characters",
    )
    .await?;

    for (tag, password) in [
        ("innerspace", "a-long-enough password"),
        ("leadingspace", " a-long-enough-password"),
        ("trailingtab", "a-long-enough-password\t"),
        ("unicodespace", "a-long-enough\u{3000}password"),
        ("nextline", "a-long-enough\u{85}password"),
        ("byteordermark", "\u{feff}a-long-enough-password"),
        ("nullbyte", "a-long-enough\0password"),
        ("backspace", "a-long-enough\u{8}password"),
        ("zerowidth", "a-long-enough\u{200b}password"),
    ] {
        let mut signup = NewSignup::new(tag);
        signup.password = password.to_string();
        assert_rejected(
            &server,
            &signup,
            "password cannot contain whitespace or non-printable characters",
        )
        .await?;
    }

    let mut missing_at_sign = NewSignup::new("bademail");
    missing_at_sign.email = "ada.example.com".to_string();
    assert_rejected(&server, &missing_at_sign, "email must be valid").await?;

    let mut blank_email = NewSignup::new("blankemail");
    blank_email.email = "   ".to_string();
    assert_rejected(&server, &blank_email, "email is required").await?;

    Ok(())
}

#[tokio::test]
async fn signup_rejects_malformed_requests() -> io::Result<()> {
    let server = TestServer::spawn("signup-malformed").await?;

    let truncated = server.post_json(SIGNUP_PATH, "{\"name\": ", &[]).await?;
    assert_eq!(truncated.status_code(), 400, "{}", truncated.body_str());
    assert_eq!(
        truncated.json::<Value>()["error"],
        json!("invalid signup payload")
    );

    // GET reports whether signup is open, which it always is on the hosted build.
    let status = server.get_query(SIGNUP_PATH, &[]).await?;
    assert_eq!(status.status_code(), 200, "{}", status.body_str());
    assert_eq!(status.json::<Value>(), json!({ "open": true }));

    let wrong_method = server.delete(SIGNUP_PATH, &[]).await?;
    assert_eq!(
        wrong_method.status_code(),
        405,
        "{}",
        wrong_method.body_str()
    );
    assert_eq!(
        wrong_method.json::<Value>()["error"],
        json!("method not allowed; use GET or POST")
    );

    Ok(())
}

#[derive(Debug)]
struct ProjectRow {
    id: i32,
    account_id: String,
}

#[derive(Debug)]
struct UserRow {
    id: String,
    account_id: String,
    name: String,
    original_email: Option<String>,
    has_password: bool,
    has_last_login_at: bool,
}

async fn fetch_project_timezone(server: &TestServer, project_id: i32) -> Option<String> {
    let conn = main_db(server).await;
    let row = conn
        .query_opt(
            "SELECT timezone FROM project_settings WHERE project_id = $1",
            &[&project_id],
        )
        .await
        .expect("query project_settings")?;
    Some(row.get(0))
}

async fn fetch_project(server: &TestServer, project_name: &str) -> Option<ProjectRow> {
    let conn = main_db(server).await;
    let row = conn
        .query_opt(
            "SELECT id, account_id::text FROM projects WHERE name = $1",
            &[&project_name],
        )
        .await
        .expect("query project")?;
    Some(ProjectRow {
        id: row.get(0),
        account_id: row.get(1),
    })
}

async fn fetch_user(server: &TestServer, email: &str) -> Option<UserRow> {
    let conn = main_db(server).await;
    let row = conn
        .query_opt(
            "SELECT
                id::text,
                account_id::text,
                name,
                original_email,
                hashed_password IS NOT NULL,
                last_login_at IS NOT NULL
             FROM users
             WHERE email = $1",
            &[&email],
        )
        .await
        .expect("query user")?;
    Some(UserRow {
        id: row.get(0),
        account_id: row.get(1),
        name: row.get(2),
        original_email: row.get(3),
        has_password: row.get(4),
        has_last_login_at: row.get(5),
    })
}

/// A user's stored sessions, each as the token in `tokens` whose hash it is,
/// or as the hex of its hash when none is.
async fn session_ids_for_user(server: &TestServer, user_id: &str, tokens: &[&str]) -> Vec<String> {
    let conn = main_db(server).await;
    conn.query(
        "SELECT COALESCE(
                (SELECT t FROM unnest($2::text[]) t
                 WHERE sha256(uuid_send(t::uuid)) = s.token_hash),
                encode(s.token_hash, 'hex'))
         FROM sessions s
         WHERE s.user_id = $1::text::uuid
         ORDER BY s.token_hash",
        &[&user_id, &tokens],
    )
    .await
    .expect("query sessions")
    .iter()
    .map(|row| row.get::<_, String>(0))
    .collect()
}
