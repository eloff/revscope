// Signing up and logging in with Google and Apple ID tokens on revscope.co.
#![cfg(not(feature = "self-hosted"))]

mod common;

use std::io;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use common::{APPLE_ISSUER, GOOGLE_ISSUER, SIGN_IN_CLIENT_ID, TestServer, id_token};
use serde_json::{Value, json};

const ID_TOKEN_PATH: &str = "/v2/public/auth/id-token";
const PROVIDERS_PATH: &str = "/v2/public/auth/providers";
const SIGNUP_PATH: &str = "/v2/public/auth/signup";
const ME_PATH: &str = "/v2/session/me";

fn unique_slug(tag: &str) -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("idt{tag}{}{n}", nanos % 1_000_000_000_000)
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

fn last_login_method(response: &common::HttpResponse) -> Option<&str> {
    response.set_cookies.iter().find_map(|cookie| {
        let (pair, _) = cookie.split_once(';')?;
        pair.strip_prefix("revscope_last_login_method=")
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
    assert_eq!(last_login_method(&response), None);

    let response = server
        .post_json(
            ID_TOKEN_PATH,
            &json!({ "provider": "google", "id_token": token, "signup": signup_fields(&website) })
                .to_string(),
            &[],
        )
        .await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    assert_eq!(last_login_method(&response), Some("google"));
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
    assert_eq!(last_login_method(&response), Some("google"));
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
async fn a_sign_in_never_takes_over_a_user_who_has_not_proved_their_email() -> io::Result<()> {
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
                "An account with this email already exists. To connect this sign-in, use Forgot password to set a new password, then try again. That confirms the email is yours and signs out your other devices and connected tools."
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
async fn a_second_provider_with_the_same_email_joins_the_same_user() -> io::Result<()> {
    let server = TestServer::spawn("id-token-second-provider").await?;
    let slug = unique_slug("p");
    let website = format!("{slug}.example");
    let email = format!("{slug}@example.com");
    let google = id_token(GOOGLE_ISSUER, &slug, Some(&email), json!(true));
    let response = server
        .post_json(
            ID_TOKEN_PATH,
            &json!({ "provider": "google", "id_token": google, "signup": signup_fields(&website) })
                .to_string(),
            &[],
        )
        .await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    let user_id = response.json::<Value>()["user_id"].clone();

    let apple = id_token(APPLE_ISSUER, &slug, Some(&email), json!("true"));
    // The second time finds the link rather than making it.
    for _ in 0..2 {
        let response = server
            .post_json(
                ID_TOKEN_PATH,
                &json!({ "provider": "apple", "id_token": apple }).to_string(),
                &[],
            )
            .await?;
        assert_eq!(response.status_code(), 200, "{}", response.body_str());
        assert_eq!(response.json::<Value>()["user_id"], user_id);
        assert_eq!(last_login_method(&response), Some("apple"));
    }

    delete_account(&server, &website).await;
    Ok(())
}

#[tokio::test]
async fn only_a_first_login_at_a_provider_from_the_login_page_joins_a_user() -> io::Result<()> {
    let server = TestServer::spawn("id-token-link-limits").await?;
    let slug = unique_slug("q");
    let website = format!("{slug}.example");
    let email = format!("{slug}@example.com");
    let google = id_token(GOOGLE_ISSUER, &slug, Some(&email), json!(true));
    let response = server
        .post_json(
            ID_TOKEN_PATH,
            &json!({ "provider": "google", "id_token": google, "signup": signup_fields(&website) })
                .to_string(),
            &[],
        )
        .await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());

    let other_subject = format!("{slug}-other");
    let other_google = id_token(GOOGLE_ISSUER, &other_subject, Some(&email), json!(true));
    let apple = id_token(APPLE_ISSUER, &slug, Some(&email), json!("true"));
    for (body, error) in [
        (
            json!({ "provider": "google", "id_token": other_google }),
            "An account with this email already uses a different login with this provider. Log in with that one, or use Forgot password.",
        ),
        (
            json!({
                "provider": "apple",
                "id_token": apple,
                "signup": signup_fields(&format!("{slug}-other.example")),
            }),
            "An account with this email already exists. Log in with this sign-in instead.",
        ),
    ] {
        let response = server
            .post_json(ID_TOKEN_PATH, &body.to_string(), &[])
            .await?;
        assert_eq!(response.status_code(), 409, "{}", response.body_str());
        assert_eq!(response.json::<Value>()["error"], json!(error));
    }
    let conn = revscope::db::open_main_database_read(server.context.clone())
        .await
        .expect("open main database");
    let identities: i64 = conn
        .query_one(
            "SELECT COUNT(*) FROM user_identities WHERE subject = ANY($1)",
            &[&vec![slug.clone(), other_subject]],
        )
        .await
        .expect("count identities")
        .get(0);
    assert_eq!(identities, 1, "only the signup's own login");
    drop(conn);

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
    assert_eq!(last_login_method(&response), Some("apple"));

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
