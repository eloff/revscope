//! Signing up, in and out, password resets, and who the session belongs to.

use revscope_api::auth::{MeProject, MeProjectSettings, MeResponse};
use serde_json::json;

use crate::auth::{
    self, AuthError, AuthSuccess, ForgotPasswordRequest, LoginRequest, MeSuccess,
    ResetPasswordRequest, SignupRequest,
};
use crate::authorize::Account;
use crate::email_verification::{self, VerifyEmailRequest};
use crate::http::Response;
use crate::rate_limit::{AuthRateLimits, RateLimiter, ip_key};
use crate::util::block_on;

use super::request::{ApiRequest, RouteResult};
use super::responses::{map_auth_error, too_many_requests};
use super::session::{
    PASSWORD_LOGIN_METHOD, expired_session_cookie, last_login_method_cookie, session_cookie,
};

/// The login dialog asks this so it can hide its signup link once a
/// self-hosted instance already has its first user.
pub(super) fn signup_status(request: &ApiRequest) -> RouteResult {
    let open = block_on(crate::self_hosted::signup_open(request.context()))
        .map_err(|error| map_auth_error(error, "signup"))?;
    Ok(Response::ok_json(json!({ "open": open })))
}

pub(super) fn signup(request: &ApiRequest) -> RouteResult {
    check_ip(request, |limits| &limits.signup_ip)?;
    let mut issue_token = false;
    let outcome = match serde_json::from_slice::<SignupRequest>(request.body) {
        Ok(payload) => {
            issue_token = payload.issue_token;
            block_on(auth::signup(request.context(), payload))
        }
        Err(_) => Err(AuthError::InvalidRequest("invalid signup payload")),
    };
    // A rejected form costs no hashing or writes, and someone fixing typos
    // shouldn't use up their signups.
    if let (Err(error), Some(ip)) = (&outcome, request.client_ip)
        && auth::is_fixable_signup_error(error)
    {
        request
            .context()
            .auth_rate_limits
            .signup_ip
            .refund(&ip_key(ip));
    }
    Ok(session_auth_response(
        request,
        outcome.map(|success| (success, PASSWORD_LOGIN_METHOD)),
        issue_token,
    ))
}

pub(super) fn login(request: &ApiRequest) -> RouteResult {
    check_ip(request, |limits| &limits.login_ip)?;
    let mut issue_token = false;
    let outcome = match serde_json::from_slice::<LoginRequest>(request.body) {
        Ok(payload) => {
            issue_token = payload.issue_token;
            check(
                &request.context().auth_rate_limits.login_email,
                &email_key(request, &payload.username),
            )?;
            block_on(auth::login(request.context(), payload))
        }
        Err(_) => Err(AuthError::InvalidRequest("invalid login payload")),
    };
    Ok(session_auth_response(
        request,
        outcome.map(|success| (success, PASSWORD_LOGIN_METHOD)),
        issue_token,
    ))
}

pub(super) fn logout(request: &ApiRequest) -> RouteResult {
    if let Some(session) = request.session() {
        block_on(auth::logout(request.context(), &session))
            .map_err(|error| map_auth_error(error, "logout"))?;
    }
    // The cookie is cleared even without a session so a client holding
    // an unparseable or already-deleted one still ends up logged out.
    Ok(Response::json_with_headers(
        "200 OK",
        json!({ "ok": true }).to_string(),
        &[expired_session_cookie(request.parsed.host)],
    ))
}

pub(super) fn forgot_password(request: &ApiRequest) -> RouteResult {
    check_ip(request, |limits| &limits.forgot_password_ip)?;
    let outcome = match serde_json::from_slice::<ForgotPasswordRequest>(request.body) {
        Ok(payload) => {
            check(
                &request.context().auth_rate_limits.forgot_password_email,
                &email_key(request, &payload.email),
            )?;
            auth::request_password_reset(request.context(), payload)
        }
        Err(_) => Err(AuthError::InvalidRequest("invalid forgot password payload")),
    };
    ok_or_auth_error(outcome, "forgot_password")
}

pub(super) fn reset_password(request: &ApiRequest) -> RouteResult {
    let outcome = match serde_json::from_slice::<ResetPasswordRequest>(request.body) {
        Ok(payload) => block_on(auth::reset_password(request.context(), payload)),
        Err(_) => Err(AuthError::InvalidRequest("invalid reset password payload")),
    };
    ok_or_auth_error(outcome, "reset_password")
}

pub(super) fn verify_email(request: &ApiRequest) -> RouteResult {
    let outcome = match serde_json::from_slice::<VerifyEmailRequest>(request.body) {
        Ok(payload) => block_on(email_verification::verify_email(
            request.context(),
            &payload.token,
        )),
        Err(_) => Err(AuthError::InvalidRequest("invalid verify email payload")),
    };
    ok_or_auth_error(outcome, "verify_email")
}

pub(super) fn resend_verification(request: &ApiRequest, account: &Account) -> RouteResult {
    let user = account.user();
    if !user.account_confirmed {
        check(
            &request.context().auth_rate_limits.verification_email_user,
            &user.user_id,
        )?;
        let email = auth::normalize_email(&user.email)
            .map_err(|error| map_auth_error(error, "resend_verification"))?;
        email_verification::send_verification(request.context(), email.normalized);
    }
    Ok(Response::ok_json(json!({ "ok": true })))
}

pub(super) fn me(request: &ApiRequest, account: &Account) -> RouteResult {
    let current_user = block_on(auth::me(request.context(), account.user().clone()))
        .map_err(|error| request.session_error(Some(account.session()), error, "me"))?;
    Ok(Response::ok_json(me_response(&current_user)))
}

/// The address is checked before the body is parsed, so a client over its
/// limit is refused before any per-email bucket is created for it. Requests
/// with no known address (only in tests that bypass the listener) skip it.
pub(super) fn check_ip(
    request: &ApiRequest,
    limiter: impl FnOnce(&AuthRateLimits) -> &RateLimiter,
) -> Result<(), Response> {
    let Some(ip) = request.client_ip else {
        return Ok(());
    };
    check(limiter(&request.context().auth_rate_limits), &ip_key(ip))
}

fn check(limiter: &RateLimiter, key: &str) -> Result<(), Response> {
    limiter.check(key).map_err(too_many_requests)
}

/// See [`crate::rate_limit`] for why the address is part of an email's key.
fn email_key(request: &ApiRequest, email: &str) -> String {
    let email = email.trim().to_ascii_lowercase();
    match request.client_ip {
        Some(ip) => format!("{email}\n{}", ip_key(ip)),
        None => email,
    }
}

fn ok_or_auth_error(outcome: Result<(), AuthError>, context: &str) -> RouteResult {
    outcome
        .map(|()| Response::ok_json(json!({ "ok": true })))
        .map_err(|error| map_auth_error(error, context))
}

/// Signup and login answer with the session cookie and the same body as
/// `/v2/session/me`, so a client that just authenticated never has to follow up
/// with a second request to learn who it is. The session id itself stays in the
/// `HttpOnly` cookie and out of the body, unless the caller asked for an API
/// token, which it then holds in place of a cookie.
pub(super) fn session_auth_response(
    request: &ApiRequest,
    outcome: Result<(AuthSuccess, &'static str), AuthError>,
    issue_token: bool,
) -> Response {
    let (success, login_method) = match outcome {
        Ok(success) => success,
        Err(error) => return map_auth_error(error, "signup/login"),
    };
    let server_context = request.context();
    let host = request.parsed.host;
    let session = success.session;
    let answer = block_on(async {
        let api_token = if issue_token {
            Some(auth::extend_session_to_api_token(server_context.clone(), &session).await?)
        } else {
            None
        };
        let user = auth::session_user(server_context.clone(), &session).await?;
        let current_user = auth::me(server_context, user).await?;
        Ok::<_, AuthError>((current_user, api_token))
    });
    match answer {
        Ok((current_user, api_token)) => {
            let mut body = me_response(&current_user);
            body.api_token = api_token;
            let body =
                serde_json::to_string(&body).expect("a response of strings always serializes");
            if issue_token {
                Response::json("200 OK", body)
            } else {
                Response::json_with_headers(
                    "200 OK",
                    body,
                    // Session last: the e2e helpers' `header("set-cookie")` answers the last one.
                    &[
                        last_login_method_cookie(login_method, host, request.parsed.origin),
                        session_cookie(session.session_id.as_str(), host),
                    ],
                )
            }
        }
        Err(error) => map_auth_error(error, "me"),
    }
}

fn me_response(current_user: &MeSuccess) -> MeResponse {
    let projects = current_user
        .projects
        .iter()
        .map(|(name, project)| {
            (
                name.clone(),
                MeProject {
                    id: project.id.clone(),
                    settings: MeProjectSettings {
                        timezone: project.settings.timezone.clone(),
                    },
                },
            )
        })
        .collect();
    MeResponse {
        name: current_user.user.name.clone(),
        email: current_user.user.email.clone(),
        account_id: current_user.user.account_id.clone(),
        user_id: current_user.user.user_id.clone(),
        projects,
        plan: current_user.user.plan.clone(),
        is_admin: current_user.user.is_admin,
        account_confirmed: current_user.user.account_confirmed,
        gravatar_url: current_user.gravatar_url.clone(),
        data_retention_months: current_user.user.retention_months,
        unpaid_subscription: current_user.unpaid_subscription.clone(),
        api_token: None,
    }
}
