//! The error answers every API route shares.

use std::time::Duration;

use revscope_api::errors::{ErrorBody, ErrorCode};

use crate::auth::AuthError;
use crate::http::Response;
use crate::query_token::ProjectAuth;
use crate::server_context::ServerContext;

/// The body every error here answers with: the message for a person, and the
/// code for a client to act on.
fn error_response(status: &str, code: ErrorCode, message: impl Into<String>) -> Response {
    let body = serde_json::to_string(&ErrorBody::new(code, message))
        .expect("a body of strings always serializes");
    Response::json(status, body)
}

pub(super) fn bad_request(message: &str) -> Response {
    error_response("400 Bad Request", ErrorCode::InvalidRequest, message)
}

pub(super) fn map_project_auth_error(error: AuthError, context: &str) -> Response {
    match error {
        AuthError::InvalidSession | AuthError::InvalidCredentials => unauthorized_invalid_session(),
        other => map_auth_error(other, context),
    }
}

pub(crate) fn too_many_requests(retry_after: Duration) -> Response {
    rate_limited(
        retry_after,
        "too many attempts; please wait a few minutes and try again",
    )
}

/// Invites earn back slowly, so "a few minutes" would send people back too soon.
pub(super) fn too_many_invites(retry_after: Duration) -> Response {
    rate_limited(
        retry_after,
        "too many invites sent recently; please try again later",
    )
}

fn rate_limited(retry_after: Duration, message: &str) -> Response {
    // Rounded up: a client that waits exactly this long must not be refused.
    let seconds = retry_after.as_secs() + u64::from(retry_after.subsec_nanos() > 0);
    let body = serde_json::to_string(&ErrorBody::new(ErrorCode::RateLimited, message))
        .expect("a body of strings always serializes");
    Response::json_with_headers(
        "429 Too Many Requests",
        body,
        &[("Retry-After", seconds.to_string())],
    )
}

pub(super) fn method_not_allowed(advice: &str) -> Response {
    error_response(
        "405 Method Not Allowed",
        ErrorCode::MethodNotAllowed,
        format!("method not allowed; {advice}"),
    )
}

pub(super) fn unauthorized_not_authenticated() -> Response {
    error_response(
        "401 Unauthorized",
        ErrorCode::NotAuthenticated,
        "not authenticated",
    )
}

pub(super) fn unauthorized_invalid_session() -> Response {
    error_response(
        "401 Unauthorized",
        ErrorCode::InvalidSession,
        "invalid session",
    )
}

pub(super) fn unauthorized_invalid_token() -> Response {
    error_response(
        "401 Unauthorized",
        ErrorCode::InvalidSession,
        "invalid token",
    )
}

/// Turns a handler's rejection into a 401 naming the credential that failed,
/// dropping a cached session so the next request re-checks it.
pub(super) fn unauthorized_rejected(
    server_context: &ServerContext,
    auth: &ProjectAuth,
    session_cache_key: Option<u128>,
) -> Response {
    if let Some(cache_key) = session_cache_key {
        server_context.session_cache.evict(cache_key);
    }
    match auth {
        ProjectAuth::Session(_) => unauthorized_invalid_session(),
        ProjectAuth::QueryToken(_) => unauthorized_invalid_token(),
    }
}

pub(crate) fn map_auth_error(error: AuthError, context: &str) -> Response {
    let internal = || {
        error_response(
            "500 Internal Server Error",
            ErrorCode::Internal,
            "internal server error",
        )
    };
    match error {
        AuthError::InvalidRequest(message) => {
            error_response("400 Bad Request", ErrorCode::InvalidRequest, message)
        }
        AuthError::InvalidCredentials => error_response(
            "401 Unauthorized",
            ErrorCode::InvalidCredentials,
            "invalid credentials",
        ),
        AuthError::InvalidSession => error_response(
            "401 Unauthorized",
            ErrorCode::InvalidSession,
            "invalid credentials",
        ),
        AuthError::Conflict(message) => {
            error_response("409 Conflict", ErrorCode::Conflict, message)
        }
        AuthError::Forbidden(message) => {
            error_response("403 Forbidden", ErrorCode::Forbidden, message)
        }
        AuthError::NotFound(message) => {
            error_response("404 Not Found", ErrorCode::NotFound, message)
        }
        AuthError::NoProjectAccess => {
            error_response("404 Not Found", ErrorCode::NotFound, "project not found")
        }
        AuthError::Temporary(message) => {
            error_response("503 Service Unavailable", ErrorCode::Temporary, message)
        }
        AuthError::Db(message) => {
            internal().with_internal_error(format!("{context} database error: {message}"))
        }
        AuthError::Query(message) => {
            internal().with_internal_error(format!("{context} query error: {message}"))
        }
        AuthError::InvalidHashFormat => {
            internal().with_internal_error(format!("{context} hash format error"))
        }
    }
}
