// Hosted signup creates a per-website account with a plan; self-hosted signup does not.
#![cfg(not(feature = "self-hosted"))]

//! Login, forgot password, reset password, and logout, end to end, with the
//! rate limits on them. The reset link only ever leaves the server in an
//! email, so this binary points MailChannels at a local stand-in that keeps
//! every message it is sent.

mod common;

use std::io;
use std::time::{SystemTime, UNIX_EPOCH};

use common::mail::{
    RESET_LINK, emails_sent_to, links_sent_to, new_link_sent_to, start_mail_server,
};
use common::{GOOGLE_ISSUER, HttpResponse, TestServer, id_token, workspace_id_token};
use serde_json::{Value, json};

const SIGNUP_PATH: &str = "/v2/public/auth/signup";
const LOGIN_PATH: &str = "/v2/public/auth/login";
const LOGOUT_PATH: &str = "/v2/public/auth/logout";
const FORGOT_PASSWORD_PATH: &str = "/v2/public/auth/forgot-password";
const RESET_PASSWORD_PATH: &str = "/v2/public/auth/reset-password";
const ID_TOKEN_PATH: &str = "/v2/public/auth/id-token";
const EMAIL_NOT_PROVEN: &str = "An account with this email already exists. To connect this sign-in, use Forgot password to set a new password, then try again. That confirms the email is yours and signs out your other devices and connected tools.";
const ME_PATH: &str = "/v2/session/me";
const PASSWORD: &str = "a-long-enough-password";
const NEW_PASSWORD: &str = "another-long-enough-password";

#[tokio::test]
async fn a_reset_link_sets_a_new_password_and_ends_every_session() -> io::Result<()> {
    start_mail_server();
    let server = TestServer::spawn("password-reset").await?;
    let account = signup(&server, "pwreset").await?;
    let old_cookie = format!("rr_session={}", account.session_id);
    assert_eq!(me_status(&server, &old_cookie).await?, 200);
    let minted = server
        .post_json(
            &format!("/v2/session/projects/{}/query-token", account.project_id),
            "",
            &[("Cookie", old_cookie.as_str())],
        )
        .await?;
    assert_eq!(minted.status_code(), 200, "{}", minted.body_str());
    let query_bearer = format!(
        "Bearer {}",
        minted.json::<Value>()["token"].as_str().expect("token")
    );
    let schema_path = format!("/v2/query/projects/{}/schema", account.project_id);
    let schema = server
        .get_query(&schema_path, &[("Authorization", query_bearer.as_str())])
        .await?;
    assert_eq!(schema.status_code(), 200, "{}", schema.body_str());

    let forgot = server
        .post_json(
            FORGOT_PASSWORD_PATH,
            &json!({ "email": account.email }).to_string(),
            &[],
        )
        .await?;
    assert_eq!(forgot.status_code(), 200, "{}", forgot.body_str());
    let token = new_link_sent_to(&account.email, RESET_LINK, 0).await;

    let too_short = reset(&server, &token, "short").await?;
    assert_eq!(too_short.status_code(), 400, "{}", too_short.body_str());

    let with_space = reset(&server, &token, "a long enough password").await?;
    assert_eq!(with_space.status_code(), 400, "{}", with_space.body_str());
    assert_eq!(
        with_space.json::<Value>()["error"],
        json!("password cannot contain whitespace or non-printable characters")
    );
    assert_eq!(
        login(&server, &account.email, PASSWORD)
            .await?
            .status_code(),
        200,
        "a rejected reset must leave the old password in place"
    );

    let reset_response = reset(&server, &token, NEW_PASSWORD).await?;
    assert_eq!(
        reset_response.status_code(),
        200,
        "{}",
        reset_response.body_str()
    );

    // A reset is how someone locks out whoever had their password, so the
    // sessions signed in under the old one end with it. This first one also
    // ends query and MCP tokens: until it, whoever signed up with the address
    // could have been anyone.
    assert_eq!(me_status(&server, &old_cookie).await?, 401);
    let schema = server
        .get_query(&schema_path, &[("Authorization", query_bearer.as_str())])
        .await?;
    assert_eq!(schema.status_code(), 401, "{}", schema.body_str());
    assert_eq!(
        login(&server, &account.email, PASSWORD)
            .await?
            .status_code(),
        401
    );
    let signed_in = login(&server, &account.email, NEW_PASSWORD).await?;
    assert_eq!(signed_in.status_code(), 200, "{}", signed_in.body_str());
    let new_cookie = format!("rr_session={}", session_id_from_cookie(&signed_in));
    assert_eq!(me_status(&server, &new_cookie).await?, 200);

    // The link is single-use.
    let replayed = reset(&server, &token, "yet-another-long-password").await?;
    assert_eq!(replayed.status_code(), 400, "{}", replayed.body_str());
    assert_eq!(
        replayed.json::<Value>()["error"],
        json!("password reset link is invalid or expired")
    );
    assert_eq!(me_status(&server, &new_cookie).await?, 200);

    // Later resets leave the tokens made since.
    let minted = server
        .post_json(
            &format!("/v2/session/projects/{}/query-token", account.project_id),
            "",
            &[("Cookie", new_cookie.as_str())],
        )
        .await?;
    assert_eq!(minted.status_code(), 200, "{}", minted.body_str());
    let query_bearer = format!(
        "Bearer {}",
        minted.json::<Value>()["token"].as_str().expect("token")
    );
    set_password_by_email(&server, &account.email, "yet-another-long-password").await?;
    let schema = server
        .get_query(&schema_path, &[("Authorization", query_bearer.as_str())])
        .await?;
    assert_eq!(schema.status_code(), 200, "{}", schema.body_str());

    delete_account(&server, &account.account_name).await;
    Ok(())
}

#[tokio::test]
async fn an_account_signed_up_without_a_password_is_claimed_with_a_reset_link() -> io::Result<()> {
    start_mail_server();
    let server = TestServer::spawn("password-claim").await?;
    let slug = format!("claim{}", unique_suffix());
    let account_name = format!("{slug}.example");
    let email = format!("{slug}@example.com");
    let signup = |issue_token: bool| {
        json!({
            "name": "Ada Agent",
            "account_name": account_name,
            "email": email,
            "plan": "starter",
            "events": "100k",
            "billing": "monthly",
            "issue_token": issue_token,
        })
        .to_string()
    };

    // Without a token nothing could reach the account, so a password is required.
    let refused = server.post_json(SIGNUP_PATH, &signup(false), &[]).await?;
    assert_eq!(refused.status_code(), 400, "{}", refused.body_str());

    let response = server.post_json(SIGNUP_PATH, &signup(true), &[]).await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    assert_eq!(response.header("set-cookie"), None, "a token, not a cookie");
    let token = response.json::<Value>()["api_token"]
        .as_str()
        .expect("an API token")
        .to_string();
    let bearer = format!("Bearer {token}");
    let me = server
        .get_query(ME_PATH, &[("Authorization", &bearer)])
        .await?;
    assert_eq!(me.status_code(), 200, "{}", me.body_str());
    common::signup::mark_subscription_paid(&server, &account_name).await;

    // It isn't a pending invite, or an admin could send themselves its reset link and
    // take it over, and the token holder could demote the only admin.
    let reinvite = server
        .post_json(
            "/v2/session/users/invite",
            &json!({ "name": "Ada Agent", "email": email, "is_admin": false, "project_ids": null })
                .to_string(),
            &[("Authorization", &bearer)],
        )
        .await?;
    assert_eq!(reinvite.status_code(), 409, "{}", reinvite.body_str());

    // No password to log in with until its owner sets one.
    assert_eq!(login(&server, &email, PASSWORD).await?.status_code(), 401);
    let forgot = server
        .post_json(
            FORGOT_PASSWORD_PATH,
            &json!({ "email": email }).to_string(),
            &[],
        )
        .await?;
    assert_eq!(forgot.status_code(), 200, "{}", forgot.body_str());
    let reset_token = new_link_sent_to(&email, RESET_LINK, 0).await;
    assert_eq!(
        reset(&server, &reset_token, NEW_PASSWORD)
            .await?
            .status_code(),
        200
    );
    assert_eq!(
        login(&server, &email, NEW_PASSWORD).await?.status_code(),
        200
    );

    // Claiming it locks out whoever held the token, like any other session.
    let me = server
        .get_query(ME_PATH, &[("Authorization", &bearer)])
        .await?;
    assert_eq!(me.status_code(), 401, "{}", me.body_str());

    delete_account(&server, &account_name).await;
    Ok(())
}

#[tokio::test]
async fn a_reset_link_proves_the_email_so_a_provider_can_join_the_password_user() -> io::Result<()>
{
    start_mail_server();
    let server = TestServer::spawn("password-proves-email").await?;
    let account = signup(&server, "pwproves").await?;
    let subject = format!("subject-{}", account.email);
    assert_eq!(
        google_sign_in(&server, &subject, &account.email)
            .await?
            .json::<Value>()["error"],
        json!(EMAIL_NOT_PROVEN),
        "nothing has proved this signup owns the address"
    );

    set_password_by_email(&server, &account.email, NEW_PASSWORD).await?;

    // A signup would count as a new customer and start a checkout for it.
    let from_signup = server
        .post_json(
            ID_TOKEN_PATH,
            &json!({
                "provider": "google",
                "id_token": workspace_id_token(&subject, &account.email),
                "signup": {
                    "account_name": format!("other-{}", account.account_name),
                    "plan": "free",
                    "events": "",
                    "billing": "monthly",
                },
            })
            .to_string(),
            &[],
        )
        .await?;
    assert_eq!(from_signup.status_code(), 409, "{}", from_signup.body_str());
    assert_eq!(
        from_signup.json::<Value>()["error"],
        json!("An account with this email already exists. Log in with this sign-in instead.")
    );

    let linked = google_sign_in(&server, &subject, &account.email).await?;
    assert_eq!(linked.status_code(), 200, "{}", linked.body_str());
    let by_password = login(&server, &account.email, NEW_PASSWORD).await?;
    assert_eq!(by_password.status_code(), 200, "{}", by_password.body_str());
    assert_eq!(
        linked.json::<Value>()["user_id"],
        by_password.json::<Value>()["user_id"]
    );

    delete_account(&server, &account.account_name).await;
    Ok(())
}

#[tokio::test]
async fn forgot_password_adds_a_password_to_a_google_user() -> io::Result<()> {
    start_mail_server();
    let server = TestServer::spawn("password-on-google").await?;
    let slug = format!("pwgoogle{}", unique_suffix());
    let account_name = format!("{slug}.example");
    let email = format!("{slug}@example.com");
    let token = id_token(GOOGLE_ISSUER, &slug, Some(&email), json!(true));
    let created = server
        .post_json(
            ID_TOKEN_PATH,
            &json!({
                "provider": "google",
                "id_token": token,
                "signup": {
                    "account_name": account_name,
                    "plan": "free",
                    "events": "",
                    "billing": "monthly",
                },
            })
            .to_string(),
            &[],
        )
        .await?;
    assert_eq!(created.status_code(), 200, "{}", created.body_str());
    let user_id = created.json::<Value>()["user_id"].clone();

    set_password_by_email(&server, &email, NEW_PASSWORD).await?;

    let by_password = login(&server, &email, NEW_PASSWORD).await?;
    assert_eq!(by_password.status_code(), 200, "{}", by_password.body_str());
    assert_eq!(by_password.json::<Value>()["user_id"], user_id);
    let by_google = google_sign_in(&server, &slug, &email).await?;
    assert_eq!(by_google.status_code(), 200, "{}", by_google.body_str());
    assert_eq!(by_google.json::<Value>()["user_id"], user_id);

    delete_account(&server, &account_name).await;
    Ok(())
}

// The inviter is handed the link, so an admin could invite someone's address,
// set the password themselves, and have that person's Google login join them.
#[tokio::test]
async fn an_invite_link_does_not_prove_the_email() -> io::Result<()> {
    start_mail_server();
    let server = TestServer::spawn("invite-no-proof").await?;
    let admin = signup(&server, "invadmin").await?;
    common::signup::mark_subscription_paid(&server, &admin.account_name).await;
    let admin_cookie = format!("rr_session={}", admin.session_id);
    let email = format!("invitee{}@example.com", unique_suffix());
    let subject = format!("subject-{email}");

    let invite_token = invite(&server, &admin_cookie, &email).await?;
    assert_eq!(
        google_sign_in(&server, &subject, &email)
            .await?
            .json::<Value>()["error"],
        json!(EMAIL_NOT_PROVEN),
        "an invite nobody has accepted"
    );

    assert_eq!(
        reset(&server, &invite_token, PASSWORD).await?.status_code(),
        200
    );
    assert_eq!(
        google_sign_in(&server, &subject, &email)
            .await?
            .json::<Value>()["error"],
        json!(EMAIL_NOT_PROVEN),
        "an invite link the inviter may have used"
    );

    delete_account(&server, &admin.account_name).await;
    Ok(())
}

#[tokio::test]
async fn proving_the_email_ends_invite_links_still_out() -> io::Result<()> {
    start_mail_server();
    let server = TestServer::spawn("invite-link-ended").await?;
    let admin = signup(&server, "invended").await?;
    common::signup::mark_subscription_paid(&server, &admin.account_name).await;
    let admin_cookie = format!("rr_session={}", admin.session_id);
    let email = format!("invitee{}@example.com", unique_suffix());
    let invite_token = invite(&server, &admin_cookie, &email).await?;

    set_password_by_email(&server, &email, NEW_PASSWORD).await?;

    let replayed = reset(&server, &invite_token, PASSWORD).await?;
    assert_eq!(replayed.status_code(), 400, "{}", replayed.body_str());
    assert_eq!(
        login(&server, &email, NEW_PASSWORD).await?.status_code(),
        200
    );

    delete_account(&server, &admin.account_name).await;
    Ok(())
}

#[tokio::test]
async fn an_expired_reset_link_changes_nothing() -> io::Result<()> {
    start_mail_server();
    let server = TestServer::spawn("password-reset-expired").await?;
    let account = signup(&server, "pwexpired").await?;
    let cookie = format!("rr_session={}", account.session_id);

    let forgot = server
        .post_json(
            FORGOT_PASSWORD_PATH,
            &json!({ "email": account.email }).to_string(),
            &[],
        )
        .await?;
    assert_eq!(forgot.status_code(), 200, "{}", forgot.body_str());
    let token = new_link_sent_to(&account.email, RESET_LINK, 0).await;

    // Backdated rather than waited out: the TTL is long.
    let conn = revscope::db::open_main_database_write(server.context.clone())
        .await
        .expect("open main database");
    let expired = conn
        .execute(
            "UPDATE password_reset_tokens
             SET expires_at = NOW() - INTERVAL '1 second'
             WHERE token_hash = sha256(uuid_send($1::text::uuid))",
            &[&token],
        )
        .await
        .expect("expire reset token");
    assert_eq!(expired, 1, "the emailed token is the one stored");
    drop(conn);

    let response = reset(&server, &token, NEW_PASSWORD).await?;
    assert_eq!(response.status_code(), 400, "{}", response.body_str());
    assert_eq!(
        response.json::<Value>()["error"],
        json!("password reset link is invalid or expired")
    );
    assert_eq!(me_status(&server, &cookie).await?, 200);
    assert_eq!(
        login(&server, &account.email, PASSWORD)
            .await?
            .status_code(),
        200
    );
    assert_eq!(
        login(&server, &account.email, NEW_PASSWORD)
            .await?
            .status_code(),
        401
    );

    delete_account(&server, &account.account_name).await;
    Ok(())
}

#[tokio::test]
async fn forgot_password_does_not_reveal_whether_an_email_has_an_account() -> io::Result<()> {
    start_mail_server();
    let server = TestServer::spawn("password-reset-unknown").await?;
    let email = format!("nobody{}@example.com", unique_suffix());

    let forgot = server
        .post_json(
            FORGOT_PASSWORD_PATH,
            &json!({ "email": email }).to_string(),
            &[],
        )
        .await?;
    assert_eq!(forgot.status_code(), 200, "{}", forgot.body_str());
    assert_eq!(forgot.json::<Value>(), json!({ "ok": true }));
    assert!(
        emails_sent_to(&email).is_empty(),
        "no email goes to an address without an account"
    );
    Ok(())
}

#[tokio::test]
async fn reset_password_rejects_tokens_it_did_not_issue() -> io::Result<()> {
    start_mail_server();
    let server = TestServer::spawn("password-reset-invalid").await?;

    for (token, error) in [
        ("", "reset token is required"),
        ("not-a-token", "password reset link is invalid or expired"),
        // Well-formed, but never issued.
        (
            "01890000-0000-7000-8000-000000000000",
            "password reset link is invalid or expired",
        ),
    ] {
        let response = reset(&server, token, NEW_PASSWORD).await?;
        assert_eq!(
            response.status_code(),
            400,
            "{token}: {}",
            response.body_str()
        );
        assert_eq!(response.json::<Value>()["error"], json!(error), "{token}");
    }

    let malformed = server
        .post_json(RESET_PASSWORD_PATH, "{\"token\":1}", &[])
        .await?;
    assert_eq!(malformed.status_code(), 400, "{}", malformed.body_str());
    Ok(())
}

#[tokio::test]
async fn logout_ends_that_session_and_no_other() -> io::Result<()> {
    start_mail_server();
    let server = TestServer::spawn("logout").await?;
    let account = signup(&server, "logout").await?;
    let first_cookie = format!("rr_session={}", account.session_id);
    let first_bearer = format!("Bearer {}", account.session_id);
    let second = login(&server, &account.email, PASSWORD).await?;
    assert_eq!(second.status_code(), 200, "{}", second.body_str());
    let second_cookie = format!("rr_session={}", session_id_from_cookie(&second));

    // Warm the server's session cache, so the check below proves logout
    // evicts it rather than that a cold cache misses.
    assert_eq!(me_status(&server, &first_cookie).await?, 200);

    let logout = server
        .post_json(LOGOUT_PATH, "", &[("Cookie", first_cookie.as_str())])
        .await?;
    assert_eq!(logout.status_code(), 200, "{}", logout.body_str());
    let cleared = logout.header("set-cookie").expect("set-cookie header");
    assert!(
        cleared.starts_with("rr_session=;") && cleared.contains("Max-Age=0"),
        "{cleared}"
    );

    assert_eq!(me_status(&server, &first_cookie).await?, 401);
    let bearer = server
        .get_query(ME_PATH, &[("Authorization", first_bearer.as_str())])
        .await?;
    assert_eq!(bearer.status_code(), 401, "{}", bearer.body_str());
    let project_read = server
        .get_query(
            &format!("/v2/session/projects/{}/settings", account.project_id),
            &[("Cookie", first_cookie.as_str())],
        )
        .await?;
    assert_eq!(
        project_read.status_code(),
        401,
        "{}",
        project_read.body_str()
    );

    assert_eq!(
        me_status(&server, &second_cookie).await?,
        200,
        "the same user's other sessions stay signed in"
    );

    // Logging out twice, or without a session at all, still clears the cookie.
    for headers in [&[("Cookie", first_cookie.as_str())][..], &[]] {
        let again = server.post_json(LOGOUT_PATH, "", headers).await?;
        assert_eq!(again.status_code(), 200, "{}", again.body_str());
        assert!(again.header("set-cookie").is_some());
    }

    delete_account(&server, &account.account_name).await;
    Ok(())
}

#[tokio::test]
async fn login_is_limited_per_email_and_says_when_to_retry() -> io::Result<()> {
    start_mail_server();
    let server = TestServer::spawn("login-limit").await?;
    let email = format!("guess{}@example.com", unique_suffix());

    for attempt in 0..10 {
        let response = login(&server, &email, PASSWORD).await?;
        assert_eq!(response.status_code(), 401, "attempt {attempt}");
    }
    let limited = login(&server, &email.to_uppercase(), PASSWORD).await?;
    assert_eq!(limited.status_code(), 429, "{}", limited.body_str());
    let retry_after: u64 = limited
        .header("retry-after")
        .expect("retry-after header")
        .parse()
        .expect("retry-after seconds");
    assert!((1..=60).contains(&retry_after), "{retry_after}");

    let other = login(&server, &format!("other{email}"), PASSWORD).await?;
    assert_eq!(other.status_code(), 401, "another email has its own limit");
    Ok(())
}

#[tokio::test]
async fn spending_an_emails_login_limit_does_not_lock_out_other_addresses() -> io::Result<()> {
    start_mail_server();
    let server = TestServer::spawn("login-lockout").await?;
    let account = signup(&server, "lockout").await?;
    let body =
        |password: &str| json!({ "username": account.email, "password": password }).to_string();
    let attacker = [("CF-Connecting-IP", "198.51.100.66")];
    let owner = [("CF-Connecting-IP", "203.0.113.9")];

    for _ in 0..10 {
        let guess = server
            .post_json(LOGIN_PATH, &body("not-the-password"), &attacker)
            .await?;
        assert_eq!(guess.status_code(), 401);
    }
    let blocked = server
        .post_json(LOGIN_PATH, &body(PASSWORD), &attacker)
        .await?;
    assert_eq!(blocked.status_code(), 429);
    let owner_login = server
        .post_json(LOGIN_PATH, &body(PASSWORD), &owner)
        .await?;
    assert_eq!(owner_login.status_code(), 200, "{}", owner_login.body_str());

    delete_account(&server, &account.account_name).await;
    Ok(())
}

#[tokio::test]
async fn signup_is_limited_per_address_but_rejected_forms_are_free() -> io::Result<()> {
    start_mail_server();
    let server = TestServer::spawn("signup-limit").await?;
    let account = signup(&server, "signuplimit").await?;
    let signup_as = |email: &str, password: &str| {
        json!({
            "name": "Ada Lovelace",
            "account_name": format!("other{}.example", unique_suffix()),
            "email": email,
            "password": password,
            "plan": "starter",
            "events": "100k",
            "billing": "monthly",
        })
        .to_string()
    };

    for _ in 0..10 {
        let invalid = server
            .post_json(SIGNUP_PATH, &signup_as(&account.email, "short"), &[])
            .await?;
        assert_eq!(invalid.status_code(), 400, "{}", invalid.body_str());
    }
    for attempt in 0..4 {
        let taken = server
            .post_json(SIGNUP_PATH, &signup_as(&account.email, PASSWORD), &[])
            .await?;
        assert_eq!(
            taken.status_code(),
            409,
            "attempt {attempt}: {}",
            taken.body_str()
        );
    }
    let limited = server
        .post_json(SIGNUP_PATH, &signup_as(&account.email, PASSWORD), &[])
        .await?;
    assert_eq!(limited.status_code(), 429, "{}", limited.body_str());

    delete_account(&server, &account.account_name).await;
    Ok(())
}

#[tokio::test]
async fn forgot_password_is_limited_per_email() -> io::Result<()> {
    start_mail_server();
    let server = TestServer::spawn("forgot-limit").await?;
    let body = json!({ "email": format!("nobody{}@example.com", unique_suffix()) }).to_string();
    let forgot = || server.post_json(FORGOT_PASSWORD_PATH, &body, &[]);

    for attempt in 0..3 {
        assert_eq!(forgot().await?.status_code(), 200, "attempt {attempt}");
    }
    assert_eq!(forgot().await?.status_code(), 429);
    Ok(())
}

#[tokio::test]
async fn login_rehashes_a_password_stored_with_fewer_rounds() -> io::Result<()> {
    start_mail_server();
    let server = TestServer::spawn("password-rehash").await?;
    let account = signup(&server, "rehash").await?;
    let conn = revscope::db::open_main_database_write(server.context.clone())
        .await
        .expect("open main database");
    let row = conn
        .query_one(
            "SELECT id::text, account_id::text FROM users WHERE email = $1",
            &[&account.email],
        )
        .await
        .expect("signed up user");
    let user_id: String = row.get(0);
    let account_id: String = row.get(1);
    let old_rounds: u32 = 100_000;
    let salt = format!("{}:{user_id}:{account_id}", account.email);
    let mut derived = [0_u8; 32];
    aws_lc_rs::pbkdf2::derive(
        aws_lc_rs::pbkdf2::PBKDF2_HMAC_SHA256,
        std::num::NonZeroU32::new(old_rounds).expect("nonzero rounds"),
        salt.as_bytes(),
        PASSWORD.as_bytes(),
        &mut derived,
    );
    let mut old_hash = old_rounds.to_be_bytes().to_vec();
    old_hash.extend_from_slice(&derived);
    conn.execute(
        "UPDATE users SET hashed_password = $2 WHERE id = $1::text::uuid",
        &[&user_id, &old_hash],
    )
    .await
    .expect("store an old hash");

    assert_eq!(
        login(&server, &account.email, PASSWORD)
            .await?
            .status_code(),
        200
    );
    let stored: Vec<u8> = conn
        .query_one(
            "SELECT hashed_password FROM users WHERE id = $1::text::uuid",
            &[&user_id],
        )
        .await
        .expect("user")
        .get(0);
    assert_ne!(stored, old_hash);
    assert!(u32::from_be_bytes(stored[..4].try_into().expect("rounds")) > old_rounds);
    assert_eq!(
        login(&server, &account.email, PASSWORD)
            .await?
            .status_code(),
        200
    );
    assert_eq!(
        login(&server, &account.email, "wrong-password-here")
            .await?
            .status_code(),
        401
    );

    drop(conn);
    delete_account(&server, &account.account_name).await;
    Ok(())
}

#[tokio::test]
async fn a_password_from_before_the_character_rule_can_still_log_in() -> io::Result<()> {
    start_mail_server();
    let server = TestServer::spawn("password-whitespace").await?;
    let account = signup(&server, "whitespace").await?;
    let conn = revscope::db::open_main_database_write(server.context.clone())
        .await
        .expect("open main database");
    let row = conn
        .query_one(
            "SELECT id::text, account_id::text FROM users WHERE email = $1",
            &[&account.email],
        )
        .await
        .expect("signed up user");
    let user_id: String = row.get(0);
    let account_id: String = row.get(1);
    let rounds: u32 = 100_000;
    let salt = format!("{}:{user_id}:{account_id}", account.email);
    let legacy_password = "correct horse\u{200b}battery\0";
    let mut derived = [0_u8; 32];
    aws_lc_rs::pbkdf2::derive(
        aws_lc_rs::pbkdf2::PBKDF2_HMAC_SHA256,
        std::num::NonZeroU32::new(rounds).expect("nonzero rounds"),
        salt.as_bytes(),
        legacy_password.as_bytes(),
        &mut derived,
    );
    let mut stored = rounds.to_be_bytes().to_vec();
    stored.extend_from_slice(&derived);
    conn.execute(
        "UPDATE users SET hashed_password = $2 WHERE id = $1::text::uuid",
        &[&user_id, &stored],
    )
    .await
    .expect("store a hash from before the character rule");

    for typed in [legacy_password.to_string(), format!(" {legacy_password} ")] {
        let response = login(&server, &account.email, &typed).await?;
        assert_eq!(response.status_code(), 200, "{}", response.body_str());
    }

    drop(conn);
    delete_account(&server, &account.account_name).await;
    Ok(())
}

/// Requests a reset link for `email` and uses the newest one sent there.
async fn set_password_by_email(server: &TestServer, email: &str, password: &str) -> io::Result<()> {
    // An invite may already be waiting in the same inbox.
    let already_sent = links_sent_to(email, RESET_LINK).len();
    let forgot = server
        .post_json(
            FORGOT_PASSWORD_PATH,
            &json!({ "email": email }).to_string(),
            &[],
        )
        .await?;
    assert_eq!(forgot.status_code(), 200, "{}", forgot.body_str());
    let token = new_link_sent_to(email, RESET_LINK, already_sent).await;
    let response = reset(server, &token, password).await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    Ok(())
}

/// Signs in with a Google account Google vouches for `email` on.
async fn google_sign_in(
    server: &TestServer,
    subject: &str,
    email: &str,
) -> io::Result<HttpResponse> {
    let token = workspace_id_token(subject, email);
    server
        .post_json(
            ID_TOKEN_PATH,
            &json!({ "provider": "google", "id_token": token }).to_string(),
            &[],
        )
        .await
}

/// Invites `email` as a viewer and returns the token from the link given to the inviter.
async fn invite(server: &TestServer, admin_cookie: &str, email: &str) -> io::Result<String> {
    let response = server
        .post_json(
            "/v2/session/users/invite",
            &json!({ "name": "Ivy Invitee", "email": email, "is_admin": false, "project_ids": null })
                .to_string(),
            &[("Cookie", admin_cookie)],
        )
        .await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    let body: Value = response.json();
    let url = body["invite_url"].as_str().expect("invite url");
    Ok(url
        .split("token=")
        .nth(1)
        .and_then(|rest| rest.split('&').next())
        .expect("token in invite url")
        .to_string())
}

async fn reset(server: &TestServer, token: &str, password: &str) -> io::Result<HttpResponse> {
    server
        .post_json(
            RESET_PASSWORD_PATH,
            &json!({ "token": token, "password": password }).to_string(),
            &[],
        )
        .await
}

async fn login(server: &TestServer, email: &str, password: &str) -> io::Result<HttpResponse> {
    server
        .post_json(
            LOGIN_PATH,
            &json!({ "username": email, "password": password }).to_string(),
            &[],
        )
        .await
}

async fn me_status(server: &TestServer, cookie: &str) -> io::Result<u16> {
    Ok(server
        .get_query(ME_PATH, &[("Cookie", cookie)])
        .await?
        .status_code())
}

struct TestAccount {
    account_name: String,
    email: String,
    project_id: i32,
    session_id: String,
}

async fn signup(server: &TestServer, tag: &str) -> io::Result<TestAccount> {
    let slug = format!("{tag}{}", unique_suffix());
    let account_name = format!("{slug}.example");
    let project_name = format!("{slug}example");
    let email = format!("{slug}@example.com");
    let response = server
        .post_json(
            SIGNUP_PATH,
            &json!({
                "name": "Ada Lovelace",
                "account_name": account_name,
                "email": email,
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
        email,
        project_id,
        session_id: session_id_from_cookie(&response),
    })
}

fn unique_suffix() -> String {
    format!(
        "{:x}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after the unix epoch")
            .as_micros()
    )
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
