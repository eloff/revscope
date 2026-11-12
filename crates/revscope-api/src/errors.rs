//! The error body every API route answers with.

use serde::{Deserialize, Serialize};

/// What went wrong, for a client to act on without parsing `error`'s prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidRequest,
    /// The email or password was wrong.
    InvalidCredentials,
    /// No credential was presented.
    NotAuthenticated,
    /// A credential was presented, but it is unknown or expired.
    InvalidSession,
    Forbidden,
    /// Allowed once someone on the account confirms their email address, by a
    /// link we email, a password reset, or a Google or Apple sign-in.
    EmailUnverified,
    NotFound,
    Conflict,
    MethodNotAllowed,
    RateLimited,
    /// Try again shortly.
    Temporary,
    Internal,
    /// A code this client is too old to know.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorBody {
    pub error: String,
    /// Absent on routes that don't set one yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<ErrorCode>,
}

impl ErrorBody {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            error: message.into(),
            code: Some(code),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_this_client_does_not_know_still_parses() {
        let body: ErrorBody =
            serde_json::from_str(r#"{"error":"nope","code":"from_the_future"}"#).unwrap();
        assert_eq!(body.code, Some(ErrorCode::Unknown));
    }

    #[test]
    fn a_body_without_a_code_still_parses() {
        let body: ErrorBody = serde_json::from_str(r#"{"error":"nope"}"#).unwrap();
        assert_eq!(body.code, None);
    }
}
