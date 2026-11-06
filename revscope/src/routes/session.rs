//! Reading the session a request presents, and the cookie that carries it.

use revscope_api::auth::{API_TOKEN_PREFIX, SESSION_COOKIE as AUTH_SESSION_COOKIE_NAME};

use crate::auth::{SessionCookie, session_token};
use crate::http::{Response, host_supports_secure_cookies};
use crate::query_token::{self, ProjectAuth};
use crate::tokens::SESSION_MAX_AGE_SECONDS;

use super::responses::unauthorized_invalid_session;

/// The session's cache key, or `None` when the token is not one we could have
/// issued. Nothing is cached here: only [`crate::auth::session_user`] adds
/// entries, after Postgres confirms them.
pub(super) fn session_cache_key(session: &SessionCookie) -> Option<u128> {
    session_token(session).ok().map(|token| token.key())
}

/// The session a request presents, as the key its per-session concurrency
/// slots are counted under. Not checked against Postgres: that happens later,
/// in `guard`, and a made-up id only limits itself.
pub(crate) fn request_session_key(
    cookie_header: Option<&str>,
    authorization_header: Option<&str>,
) -> Option<u128> {
    session_cache_key(&resolve_session(cookie_header, authorization_header)?)
}

pub(super) fn resolve_session(
    cookie_header: Option<&str>,
    authorization_header: Option<&str>,
) -> Option<SessionCookie> {
    parse_session_cookie(cookie_header)
        .or_else(|| parse_session_authorization(authorization_header))
}

/// The query and schema routes are reachable both from the dashboard and from
/// clients holding a project-scoped token, such as the MCP server.
pub(super) fn resolve_project_auth(
    cookie_header: Option<&str>,
    authorization_header: Option<&str>,
) -> Option<ProjectAuth> {
    if let Some(secret) = query_token::parse_authorization(authorization_header) {
        return Some(ProjectAuth::QueryToken(secret));
    }
    resolve_session(cookie_header, authorization_header).map(ProjectAuth::Session)
}

/// Checks that a session-backed caller's token is well formed and hands back
/// its cache key, so a session the handler goes on to reject can be evicted.
/// Token-backed callers have no session to cache.
pub(super) fn prepare_project_auth(auth: &ProjectAuth) -> Result<Option<u128>, Response> {
    match auth {
        ProjectAuth::Session(session) => session_cache_key(session)
            .map(Some)
            .ok_or_else(unauthorized_invalid_session),
        ProjectAuth::QueryToken(_) => Ok(None),
    }
}

/// `SameSite=Strict` is enough to keep the cookie off cross-site requests
/// because the dashboard and the API only ever differ by subdomain, which the
/// browser still counts as same-site.
pub(super) fn session_cookie(session_id: &str, host: Option<&str>) -> (&'static str, String) {
    (
        "Set-Cookie",
        format!(
            "{AUTH_SESSION_COOKIE_NAME}={session_id}; Path=/; Max-Age={SESSION_MAX_AGE_SECONDS}; HttpOnly; SameSite=Strict{}",
            secure_cookie_attribute(host)
        ),
    )
}

pub(super) fn expired_session_cookie(host: Option<&str>) -> (&'static str, String) {
    (
        "Set-Cookie",
        format!(
            "{AUTH_SESSION_COOKIE_NAME}=; Path=/; Max-Age=0; HttpOnly; SameSite=Strict{}",
            secure_cookie_attribute(host)
        ),
    )
}

fn secure_cookie_attribute(host: Option<&str>) -> &'static str {
    if host_supports_secure_cookies(host) {
        "; Secure"
    } else {
        ""
    }
}

fn parse_session_authorization(authorization_header: Option<&str>) -> Option<SessionCookie> {
    let authorization_header = authorization_header?;
    let token = authorization_header
        .strip_prefix("Bearer ")
        .unwrap_or(authorization_header);
    session_from_token(token.strip_prefix(API_TOKEN_PREFIX).unwrap_or(token))
}

fn session_from_token(token: &str) -> Option<SessionCookie> {
    let token = token.trim();
    if token.is_empty() {
        return None;
    }
    Some(SessionCookie {
        session_id: token.to_string(),
    })
}

fn parse_session_cookie(cookie_header: Option<&str>) -> Option<SessionCookie> {
    session_from_token(crate::http::cookie_value(
        cookie_header?,
        AUTH_SESSION_COOKIE_NAME,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_cookie_is_http_only_and_same_site_strict() {
        let (name, value) = session_cookie("018f-session", Some("api.revscope.co"));
        assert_eq!(name, "Set-Cookie");
        assert_eq!(
            value,
            format!(
                "rr_session=018f-session; Path=/; Max-Age={SESSION_MAX_AGE_SECONDS}; HttpOnly; SameSite=Strict; Secure"
            )
        );
    }

    #[test]
    fn session_cookie_drops_secure_for_localhost() {
        let (_, value) = session_cookie("018f-session", Some("localhost:4320"));
        assert!(!value.contains("Secure"), "{value}");
        let (_, loopback) = session_cookie("018f-session", Some("127.0.0.1:4320"));
        assert!(!loopback.contains("Secure"), "{loopback}");
        let (_, cleared) = expired_session_cookie(Some("localhost:4320"));
        assert!(cleared.contains("Max-Age=0"), "{cleared}");
        assert!(!cleared.contains("Secure"), "{cleared}");
    }

    #[test]
    fn reads_session_from_a_bare_cookie_value() {
        let session =
            parse_session_cookie(Some("other=1; rr_session=018f-session")).expect("session");
        assert_eq!(session.session_id, "018f-session");
    }
}
