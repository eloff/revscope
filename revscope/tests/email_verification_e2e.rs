// Confirming a signup's email on revscope.co, and the features that wait for it.
#![cfg(not(feature = "self-hosted"))]

mod common;

use std::io;

use common::mail::{RESET_LINK, VERIFY_LINK, links_sent_to, new_link_sent_to, start_mail_server};
use common::signup::{NewSignup, SIGNUP_PATH, delete_accounts};
use common::{GOOGLE_ISSUER, HttpResponse, TestServer, id_token, workspace_id_token};
use revscope_api::paths::{RESEND_VERIFICATION, VERIFY_EMAIL};
use serde_json::{Value, json};

const ME_PATH: &str = "/v2/session/me";
const ID_TOKEN_PATH: &str = "/v2/public/auth/id-token";
const LINK_INVALID: &str = "This confirmation link is invalid or has expired. Send a new one from the banner at the top of the dashboard, or reset your password, which confirms it too.";

struct SignedUp {
    signup: NewSignup,
    cookie: String,
    project_id: String,
}

async fn sign_up(server: &TestServer, tag: &str) -> io::Result<SignedUp> {
    let signup = NewSignup::new(tag);
    let response = server.post_json(SIGNUP_PATH, &signup.body(), &[]).await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    let body: Value = response.json();
    assert_eq!(body["account_confirmed"], json!(false), "{body}");
    let project_id = body["projects"][&signup.project_name]["id"]
        .as_str()
        .expect("project id")
        .to_string();
    let cookie = response
        .header("set-cookie")
        .and_then(|cookie| cookie.split(';').next())
        .expect("session cookie")
        .to_string();
    Ok(SignedUp {
        signup,
        cookie,
        project_id,
    })
}

async fn verify(server: &TestServer, token: &str) -> io::Result<HttpResponse> {
    server
        .post_json(VERIFY_EMAIL, &json!({ "token": token }).to_string(), &[])
        .await
}

impl SignedUp {
    fn auth(&self) -> [(&'static str, &str); 1] {
        [("Cookie", self.cookie.as_str())]
    }

    async fn me(&self, server: &TestServer) -> io::Result<Value> {
        let response = server.get_query(ME_PATH, &self.auth()).await?;
        assert_eq!(response.status_code(), 200, "{}", response.body_str());
        Ok(response.json())
    }

    /// Each route that waits for a confirmed email, called as the dashboard would.
    async fn gated(&self, server: &TestServer) -> io::Result<Vec<HttpResponse>> {
        let id = &self.project_id;
        let stripe_key = json!({
            "timezone": "UTC",
            "integrations": [{
                "integration_type": "stripe",
                "external_account_id": null,
                "api_key": "rk_test_123",
                "secret": null,
            }],
        });
        Ok(vec![
            server
                .post_json(
                    &format!("/v2/chat/projects/{id}"),
                    &json!({ "message": "How many visitors?" }).to_string(),
                    &self.auth(),
                )
                .await?,
            server
                .post_json(
                    &format!("/v2/session/projects/{id}/ga-imports/oauth-url"),
                    "",
                    &self.auth(),
                )
                .await?,
            server
                .post_json(
                    &format!("/v2/session/projects/{id}/settings"),
                    &stripe_key.to_string(),
                    &self.auth(),
                )
                .await?,
        ])
    }
}

fn waits_for_email(response: &HttpResponse) -> bool {
    response.status_code() == 403 && response.json::<Value>()["code"] == json!("email_unverified")
}

#[tokio::test]
async fn a_password_signup_waits_for_its_emailed_link() -> io::Result<()> {
    start_mail_server();
    static ENCRYPTION_KEY: std::sync::Once = std::sync::Once::new();
    // SAFETY: inside the `Once`, and this is the only test in the binary that
    // saves an integration.
    ENCRYPTION_KEY
        .call_once(|| unsafe { std::env::set_var("INTEGRATION_ENCRYPTION_KEY", "11".repeat(32)) });
    let server = TestServer::spawn("verify-email").await?;
    let account = sign_up(&server, "verify").await?;
    let email = account.signup.email.as_str();

    assert_eq!(
        account.me(&server).await?["account_confirmed"],
        json!(false)
    );
    for response in account.gated(&server).await? {
        assert!(waits_for_email(&response), "{}", response.body_str());
    }
    // An integration stored before confirmation was required keeps working,
    // but pointing it at another account would start a new sync.
    let conn = common::signup::main_db(&server).await;
    conn.execute(
        "INSERT INTO integrations (project_id, integration_type, external_account_id, encrypted_api_key)
         VALUES ($1::text::int, 'cloudflare', 'zone-a', '\\x00'::bytea)",
        &[&account.project_id],
    )
    .await
    .expect("store an integration");
    drop(conn);
    let cloudflare = |zone: &str| {
        json!({
            "timezone": "UTC",
            "integrations": [{
                "integration_type": "cloudflare",
                "external_account_id": zone,
                "api_key": null,
                "secret": null,
            }],
        })
        .to_string()
    };
    let settings_path = format!("/v2/session/projects/{}/settings", account.project_id);
    let moved = server
        .post_json(&settings_path, &cloudflare("zone-b"), &account.auth())
        .await?;
    assert!(waits_for_email(&moved), "{}", moved.body_str());
    let kept = server
        .post_json(&settings_path, &cloudflare("zone-a"), &account.auth())
        .await?;
    assert_eq!(kept.status_code(), 200, "{}", kept.body_str());
    // Saved one over the other, two entries could restart a sync unnoticed.
    let twice = json!({
        "timezone": "UTC",
        "integrations": [
            { "integration_type": "cloudflare", "external_account_id": "zone-b", "api_key": "", "secret": "" },
            { "integration_type": "cloudflare", "external_account_id": "zone-a", "api_key": null, "secret": null },
        ],
    });
    let refused = server
        .post_json(&settings_path, &twice.to_string(), &account.auth())
        .await?;
    assert_eq!(refused.status_code(), 400, "{}", refused.body_str());
    let disconnect = json!({
        "timezone": "UTC",
        "integrations": [{
            "integration_type": "cloudflare",
            "external_account_id": null,
            "api_key": "",
            "secret": "",
        }],
    });
    let disconnected = server
        .post_json(&settings_path, &disconnect.to_string(), &account.auth())
        .await?;
    assert_eq!(
        disconnected.status_code(),
        200,
        "{}",
        disconnected.body_str()
    );
    // The rest of the settings form still saves.
    let timezone = server
        .post_json(
            &format!("/v2/session/projects/{}/settings", account.project_id),
            &json!({ "timezone": "UTC", "integrations": [] }).to_string(),
            &account.auth(),
        )
        .await?;
    assert_eq!(timezone.status_code(), 200, "{}", timezone.body_str());

    let token = new_link_sent_to(email, VERIFY_LINK, 0).await;
    let verified = verify(&server, &token).await?;
    assert_eq!(verified.status_code(), 200, "{}", verified.body_str());

    // The session already signed in sees it at once, not after its cache ages out.
    assert_eq!(account.me(&server).await?["account_confirmed"], json!(true));
    for response in account.gated(&server).await? {
        assert!(!waits_for_email(&response), "{}", response.body_str());
    }

    let replayed = verify(&server, &token).await?;
    assert_eq!(replayed.status_code(), 400, "{}", replayed.body_str());
    assert_eq!(replayed.json::<Value>()["error"], json!(LINK_INVALID));

    delete_accounts(&server, &[&account.signup.account_name]).await;
    Ok(())
}

#[tokio::test]
async fn another_link_can_be_sent_a_few_times_and_either_one_works() -> io::Result<()> {
    start_mail_server();
    let server = TestServer::spawn("verify-email-resend").await?;
    let account = sign_up(&server, "verifyresend").await?;
    let email = account.signup.email.as_str();
    let first = new_link_sent_to(email, VERIFY_LINK, 0).await;

    for sent in 1..=3 {
        let response = server
            .post_json(RESEND_VERIFICATION, "", &account.auth())
            .await?;
        assert_eq!(response.status_code(), 200, "{}", response.body_str());
        new_link_sent_to(email, VERIFY_LINK, sent).await;
    }
    let limited = server
        .post_json(RESEND_VERIFICATION, "", &account.auth())
        .await?;
    assert_eq!(limited.status_code(), 429, "{}", limited.body_str());

    // An older email still works after newer ones were sent.
    let verified = verify(&server, &first).await?;
    assert_eq!(verified.status_code(), 200, "{}", verified.body_str());
    // Once confirmed, there's nothing to send, so it isn't limited either.
    let response = server
        .post_json(RESEND_VERIFICATION, "", &account.auth())
        .await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    let sent = links_sent_to(email, VERIFY_LINK);
    let later = verify(&server, sent.last().expect("a link")).await?;
    assert_eq!(
        later.status_code(),
        400,
        "a confirmed user's other links end"
    );

    delete_accounts(&server, &[&account.signup.account_name]).await;
    Ok(())
}

#[tokio::test]
async fn links_the_server_did_not_send_are_refused() -> io::Result<()> {
    let server = TestServer::spawn("verify-email-invalid").await?;
    for token in [
        "",
        "not-a-token",
        // Well-formed, but never issued.
        "01890000-0000-4000-8000-000000000000",
    ] {
        let response = verify(&server, token).await?;
        assert_eq!(
            response.status_code(),
            400,
            "{token}: {}",
            response.body_str()
        );
        assert_eq!(response.json::<Value>()["error"], json!(LINK_INVALID));
    }
    let malformed = server.post_json(VERIFY_EMAIL, "{\"token\":1}", &[]).await?;
    assert_eq!(malformed.status_code(), 400, "{}", malformed.body_str());
    Ok(())
}

#[tokio::test]
async fn a_google_signup_counts_as_confirmed() -> io::Result<()> {
    let server = TestServer::spawn("verify-email-google").await?;
    let signup = NewSignup::new("verifygoogle");
    // An address Google doesn't host still counts here; only linking needs more.
    let token = id_token(
        GOOGLE_ISSUER,
        &signup.slug,
        Some(&signup.email),
        json!(true),
    );
    let response = server
        .post_json(
            ID_TOKEN_PATH,
            &json!({
                "provider": "google",
                "id_token": token,
                "signup": {
                    "account_name": signup.account_name,
                    "plan": "free",
                    "events": "",
                    "billing": "monthly",
                },
            })
            .to_string(),
            &[],
        )
        .await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    assert_eq!(response.json::<Value>()["account_confirmed"], json!(true));

    delete_accounts(&server, &[&signup.account_name]).await;
    Ok(())
}

// Whoever signed up still holds the password and any tokens, so a Google
// sign-in joining the user would share the account with them.
#[tokio::test]
async fn the_link_does_not_let_a_google_sign_in_join_the_user() -> io::Result<()> {
    start_mail_server();
    let server = TestServer::spawn("verify-email-link").await?;
    let account = sign_up(&server, "verifylink").await?;
    let email = account.signup.email.as_str();
    let token = new_link_sent_to(email, VERIFY_LINK, 0).await;
    assert_eq!(verify(&server, &token).await?.status_code(), 200);
    assert_eq!(account.me(&server).await?["account_confirmed"], json!(true));

    let google = workspace_id_token(&account.signup.slug, email);
    let response = server
        .post_json(
            ID_TOKEN_PATH,
            &json!({ "provider": "google", "id_token": google }).to_string(),
            &[],
        )
        .await?;
    assert_eq!(response.status_code(), 409, "{}", response.body_str());
    assert!(
        response.body_str().contains("use Forgot password"),
        "{}",
        response.body_str()
    );

    delete_accounts(&server, &[&account.signup.account_name]).await;
    Ok(())
}

#[tokio::test]
async fn a_confirmed_teammate_turns_the_features_on_for_the_whole_account() -> io::Result<()> {
    start_mail_server();
    let server = TestServer::spawn("verify-email-account").await?;
    let owner = sign_up(&server, "verifyowner").await?;
    // Invites need a paid plan.
    common::signup::mark_subscription_paid(&server, &owner.signup.account_name).await;
    // Warm the owner's cached session, so the check below proves it is dropped.
    assert_eq!(owner.me(&server).await?["account_confirmed"], json!(false));

    let teammate = format!("mate-{}", owner.signup.email);
    let invite = server
        .post_json(
            "/v2/session/users/invite",
            &json!({ "name": "Ivy Teammate", "email": teammate, "is_admin": true, "project_ids": null })
                .to_string(),
            &owner.auth(),
        )
        .await?;
    assert_eq!(invite.status_code(), 200, "{}", invite.body_str());
    // The teammate claims the invite with a reset link from their own inbox,
    // not the invite's, which the inviter holds and which proves nothing.
    let invite_links = links_sent_to(&teammate, RESET_LINK).len();
    let forgot = server
        .post_json(
            "/v2/public/auth/forgot-password",
            &json!({ "email": teammate }).to_string(),
            &[],
        )
        .await?;
    assert_eq!(forgot.status_code(), 200, "{}", forgot.body_str());
    let link = new_link_sent_to(&teammate, RESET_LINK, invite_links).await;
    let reset = server
        .post_json(
            "/v2/public/auth/reset-password",
            &json!({ "token": link, "password": "a-long-enough-password" }).to_string(),
            &[],
        )
        .await?;
    assert_eq!(reset.status_code(), 200, "{}", reset.body_str());

    assert_eq!(owner.me(&server).await?["account_confirmed"], json!(true));
    for response in owner.gated(&server).await? {
        assert!(!waits_for_email(&response), "{}", response.body_str());
    }

    delete_accounts(&server, &[&owner.signup.account_name]).await;
    Ok(())
}
