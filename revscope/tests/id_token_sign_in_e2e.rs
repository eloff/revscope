// Signing up and logging in with Google and Apple ID tokens on revscope.co.
#![cfg(not(feature = "self-hosted"))]

mod common;

use std::io;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use common::{SIGN_IN_CLIENT_ID, SIGN_IN_KEY_ID, SIGN_IN_PRIVATE_KEY, TestServer};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde_json::{Value, json};

const ID_TOKEN_PATH: &str = "/v2/public/auth/id-token";
const PROVIDERS_PATH: &str = "/v2/public/auth/providers";
const SIGNUP_PATH: &str = "/v2/public/auth/signup";
const ME_PATH: &str = "/v2/session/me";

const GOOGLE_ISSUER: &str = "https://accounts.google.com";
const APPLE_ISSUER: &str = "https://appleid.apple.com";

fn unique_slug(tag: &str) -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("idt{tag}{}{n}", nanos % 1_000_000_000_000)
}

fn id_token(issuer: &str, subject: &str, email: Option<&str>, verified: Value) -> String {
    let exp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_secs()
        + 600;
    let mut claims = json!({
        "iss": issuer,
        "aud": SIGN_IN_CLIENT_ID,
        "sub": subject,
        "exp": exp,
        "email_verified": verified,
    });
    if let Some(email) = email {
        claims["email"] = json!(email);
    }
    if issuer == GOOGLE_ISSUER {
        claims["name"] = json!("Grace Hopper");
    }
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(SIGN_IN_KEY_ID.to_owned());
    let key = EncodingKey::from_rsa_pem(SIGN_IN_PRIVATE_KEY.as_bytes()).expect("test key");
    jsonwebtoken::encode(&header, &claims, &key).expect("sign test token")
}

fn signup_fields(website: &str) -> Value {
    json!({
        "account_name": website,
        "plan": "free",
        "events": "",
        "billing": "monthly",
        "timezone": "Europe/London",
    })
}

fn session_cookie(response: &common::HttpResponse) -> String {
    let cookie = response.header("set-cookie").expect("session cookie");
    cookie.split(';').next().expect("cookie pair").to_owned()
}

async fn delete_account(server: &TestServer, website: &str) {
    let conn = revscope::db::open_main_database_write(server.context.clone())
        .await
        .expect("open main database");
    conn.execute("DELETE FROM accounts WHERE name = $1", &[&website])
        .await
        .expect("delete test account");
}

#[tokio::test]
async fn providers_lists_the_configured_client_ids() -> io::Result<()> {
    let server = TestServer::spawn("id-token-providers").await?;
    let response = server.get_query(PROVIDERS_PATH, &[]).await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    let body: Value = response.json();
    assert_eq!(body["google"], json!(SIGN_IN_CLIENT_ID));
    assert_eq!(body["apple"], json!(SIGN_IN_CLIENT_ID));
    Ok(())
}

#[tokio::test]
async fn google_signs_up_then_logs_in_to_the_same_user() -> io::Result<()> {
    let server = TestServer::spawn("id-token-google").await?;
    let slug = unique_slug("g");
    let website = format!("{slug}.example");
    let email = format!("{slug}@example.com");
    let token = id_token(GOOGLE_ISSUER, &slug, Some(&email), json!(true));

    // Logging in before signing up is refused, and creates nothing.
    let response = server
        .post_json(
            ID_TOKEN_PATH,
            &json!({ "provider": "google", "id_token": token }).to_string(),
            &[],
        )
        .await?;
    assert_eq!(response.status_code(), 404, "{}", response.body_str());
    assert!(response.body_str().contains("Sign up first"));

    let response = server
        .post_json(
            ID_TOKEN_PATH,
            &json!({ "provider": "google", "id_token": token, "signup": signup_fields(&website) })
                .to_string(),
            &[],
        )
        .await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    let body: Value = response.json();
    assert_eq!(body["name"], json!("Grace Hopper"));
    assert_eq!(body["email"], json!(email));
    // Without the private billing module every hosted signup is complimentary.
    let expected_plan = if cfg!(feature = "billing") {
        "free"
    } else {
        "enterprise"
    };
    assert_eq!(body["plan"], json!(expected_plan));
    let user_id = body["user_id"].as_str().expect("user id").to_owned();
    let project = body["projects"]
        .as_object()
        .and_then(|projects| projects.values().next())
        .expect("signup creates a project");
    assert_eq!(project["settings"]["timezone"], json!("Europe/London"));

    let response = server
        .post_json(
            ID_TOKEN_PATH,
            &json!({ "provider": "google", "id_token": token }).to_string(),
            &[],
        )
        .await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    assert_eq!(response.json::<Value>()["user_id"], json!(user_id));
    let cookie = session_cookie(&response);
    let me = server
        .get_query(ME_PATH, &[("Cookie", cookie.as_str())])
        .await?;
    assert_eq!(me.status_code(), 200, "{}", me.body_str());
    assert_eq!(me.json::<Value>()["user_id"], json!(user_id));

    // Signing up again with the same Google account just logs in.
    let response = server
        .post_json(
            ID_TOKEN_PATH,
            &json!({
                "provider": "google",
                "id_token": token,
                "signup": signup_fields(&format!("{slug}-again.example")),
            })
            .to_string(),
            &[],
        )
        .await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    assert_eq!(response.json::<Value>()["user_id"], json!(user_id));

    delete_account(&server, &website).await;
    Ok(())
}

#[tokio::test]
async fn a_sign_in_never_takes_over_a_user_with_the_same_email() -> io::Result<()> {
    let server = TestServer::spawn("id-token-no-email-link").await?;
    let slug = unique_slug("a");
    let website = format!("{slug}.example");
    let email = format!("{slug}@example.com");
    // Nothing proves this signup owns the address: it could be anyone's.
    let response = server
        .post_json(
            SIGNUP_PATH,
            &json!({
                "name": "Ada Lovelace",
                "account_name": website,
                "email": email.to_uppercase(),
                "password": "a-long-enough-password",
                "plan": "free",
            })
            .to_string(),
            &[],
        )
        .await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());

    // Apple sends `email_verified` as a string.
    let token = id_token(APPLE_ISSUER, &slug, Some(&email), json!("true"));
    for body in [
        json!({ "provider": "apple", "id_token": token }),
        json!({
            "provider": "apple",
            "id_token": token,
            "signup": signup_fields(&format!("{slug}-other.example")),
        }),
    ] {
        let response = server
            .post_json(ID_TOKEN_PATH, &body.to_string(), &[])
            .await?;
        assert_eq!(response.status_code(), 409, "{}", response.body_str());
        assert_eq!(
            response.json::<Value>()["error"],
            json!(
                "An account with this email already exists. Log in the way you signed up, or use Forgot password to set a password."
            )
        );
        assert!(response.header("set-cookie").is_none());
    }
    let conn = revscope::db::open_main_database_read(server.context.clone())
        .await
        .expect("open main database");
    let identities: i64 = conn
        .query_one(
            "SELECT COUNT(*) FROM user_identities WHERE provider = 'apple' AND subject = $1",
            &[&slug],
        )
        .await
        .expect("count identities")
        .get(0);
    assert_eq!(identities, 0);

    delete_account(&server, &website).await;
    Ok(())
}

#[tokio::test]
async fn an_unverified_email_cannot_sign_up() -> io::Result<()> {
    let server = TestServer::spawn("id-token-unverified").await?;
    let slug = unique_slug("u");
    let email = format!("{slug}@example.com");
    let token = id_token(GOOGLE_ISSUER, &slug, Some(&email), json!(false));

    let response = server
        .post_json(
            ID_TOKEN_PATH,
            &json!({ "provider": "google", "id_token": token }).to_string(),
            &[],
        )
        .await?;
    assert_eq!(response.status_code(), 400, "{}", response.body_str());

    let response = server
        .post_json(
            ID_TOKEN_PATH,
            &json!({
                "provider": "google",
                "id_token": token,
                "signup": signup_fields(&format!("{slug}.example")),
            })
            .to_string(),
            &[],
        )
        .await?;
    assert_eq!(response.status_code(), 400, "{}", response.body_str());
    assert_eq!(
        response.json::<Value>()["error"],
        json!("Google hasn't verified your email address yet.")
    );
    let conn = revscope::db::open_main_database_read(server.context.clone())
        .await
        .expect("open main database");
    let users: i64 = conn
        .query_one("SELECT COUNT(*) FROM users WHERE email = $1", &[&email])
        .await
        .expect("count users")
        .get(0);
    assert_eq!(users, 0);
    Ok(())
}
#[tokio::test]
async fn apple_signup_takes_the_name_from_the_browser() -> io::Result<()> {
    let server = TestServer::spawn("id-token-apple-name").await?;
    let slug = unique_slug("n");
    let website = format!("{slug}.example");
    let email = format!("{slug}@privaterelay.appleid.com");
    let token = id_token(APPLE_ISSUER, &slug, Some(&email), json!(true));
    let response = server
        .post_json(
            ID_TOKEN_PATH,
            &json!({
                "provider": "apple",
                "id_token": token,
                "name": "Katherine Johnson",
                "signup": signup_fields(&website),
            })
            .to_string(),
            &[],
        )
        .await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    assert_eq!(response.json::<Value>()["name"], json!("Katherine Johnson"));

    delete_account(&server, &website).await;
    Ok(())
}

#[tokio::test]
async fn bad_tokens_and_signup_fields_are_refused_with_a_message() -> io::Result<()> {
    let server = TestServer::spawn("id-token-refused").await?;
    let slug = unique_slug("r");
    let email = format!("{slug}@example.com");
    let google = id_token(GOOGLE_ISSUER, &slug, Some(&email), json!(true));
    let cases = [
        (
            json!({ "provider": "github", "id_token": google }),
            400,
            "that sign-in method isn't available",
        ),
        (
            json!({ "provider": "apple", "id_token": google }),
            400,
            "Apple sign-in didn't work. Try again.",
        ),
        (
            json!({ "provider": "google", "id_token": "not-a-token" }),
            400,
            "Google sign-in didn't work. Try again.",
        ),
        (
            json!({ "provider": "google", "id_token": "x".repeat(9000) }),
            400,
            "Google sign-in didn't work. Try again.",
        ),
        (
            json!({
                "provider": "google",
                "id_token": id_token(GOOGLE_ISSUER, &slug, None, json!(true)),
            }),
            400,
            "Google didn't share your email address.",
        ),
        (
            json!({
                "provider": "google",
                "id_token": google,
                "signup": signup_fields("localhost"),
            }),
            400,
            "Website must include a domain (e.g. mywebsite.com).",
        ),
        (
            json!({ "id_token": google }),
            400,
            "invalid sign-in payload",
        ),
    ];
    for (body, status, message) in cases {
        let response = server
            .post_json(ID_TOKEN_PATH, &body.to_string(), &[])
            .await?;
        assert_eq!(response.status_code(), status, "{}", response.body_str());
        assert_eq!(response.json::<Value>()["error"], json!(message), "{body}");
    }
    Ok(())
}

#[tokio::test]
async fn neither_a_found_account_nor_a_rejected_form_uses_up_a_signup() -> io::Result<()> {
    let server = TestServer::spawn("id-token-signup-limit").await?;
    let slug = unique_slug("l");
    let mut websites = Vec::new();
    let mut sign_up = |n: usize| {
        let website = format!("{slug}-{n}.example");
        websites.push(website.clone());
        let email = format!("{slug}-{n}@example.com");
        let token = id_token(
            GOOGLE_ISSUER,
            &format!("{slug}-{n}"),
            Some(&email),
            json!(true),
        );
        json!({ "provider": "google", "id_token": token, "signup": signup_fields(&website) })
            .to_string()
    };
    let first = sign_up(0);
    let response = server.post_json(ID_TOKEN_PATH, &first, &[]).await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    // Signing in again from the signup page finds the account, and a rejected
    // form writes nothing, so neither uses up a signup.
    for _ in 0..6 {
        let response = server.post_json(ID_TOKEN_PATH, &first, &[]).await?;
        assert_eq!(response.status_code(), 200, "{}", response.body_str());
    }
    let rejected = id_token(
        GOOGLE_ISSUER,
        &format!("{slug}-x"),
        Some("x@example.com"),
        json!(true),
    );
    for _ in 0..6 {
        let body = json!({
            "provider": "google",
            "id_token": rejected,
            "signup": signup_fields("localhost"),
        });
        let response = server
            .post_json(ID_TOKEN_PATH, &body.to_string(), &[])
            .await?;
        assert_eq!(response.status_code(), 400, "{}", response.body_str());
    }
    let mut statuses = Vec::new();
    for n in 1..6 {
        let response = server.post_json(ID_TOKEN_PATH, &sign_up(n), &[]).await?;
        statuses.push(response.status_code());
    }
    assert_eq!(statuses, [200, 200, 200, 200, 429]);

    for website in &websites {
        delete_account(&server, website).await;
    }
    Ok(())
}
