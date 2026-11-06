//! How a command fails, and what a script or agent sees when it does.

use std::fmt;
use std::process::ExitCode;

use revscope_api::errors::ErrorCode;
use serde::Serialize;

/// The process exit status. Stable, so scripts and agents can branch on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// Anything without a more specific status below.
    Failure = 1,
    /// The command line or a local input was wrong. Also what clap exits with.
    Usage = 2,
    /// Not signed in, or the credential was refused.
    Auth = 3,
    /// Signed in, but not allowed, such as a plan limit.
    Forbidden = 4,
    NotFound = 5,
    /// It already exists, such as an account for this website.
    Conflict = 6,
    /// The command ran and its answer is "no", such as a verify that found no tracker.
    CheckFailed = 7,
    /// Try again later: the server is busy, rate limiting, or unreachable. EX_TEMPFAIL.
    Retry = 75,
}

impl From<Exit> for ExitCode {
    fn from(exit: Exit) -> Self {
        ExitCode::from(exit as u8)
    }
}

#[derive(Debug)]
pub enum CliError {
    /// The API answered with an error.
    Api {
        status: u16,
        code: Option<ErrorCode>,
        message: String,
        retry_after_seconds: Option<u64>,
    },
    /// The API could not be reached.
    Unreachable { url: String, detail: String },
    /// The API answered 2xx with a body this version can't read.
    BadResponse { path: String, detail: String },
    /// The command was refused before anything was sent.
    Usage(String),
    /// Something on this machine failed, such as writing the credentials file.
    Local(String),
    /// A failure with a status of its own and no API answer behind it.
    Failed { exit: Exit, message: String },
    /// A failure with advice on what to do about it.
    Hinted { error: Box<CliError>, hint: String },
    /// Something already happened on the server, then a later step failed.
    Partial { done: String, failed: Box<CliError> },
}

pub type Result<T> = std::result::Result<T, CliError>;

impl CliError {
    pub fn usage(message: impl Into<String>) -> Self {
        Self::Usage(message.into())
    }

    pub fn local(message: impl Into<String>) -> Self {
        Self::Local(message.into())
    }

    /// Wraps a failure that came after `done` already took effect on the server,
    /// so nobody retries a step that succeeded.
    pub fn after(self, done: impl Into<String>) -> Self {
        Self::Partial {
            done: done.into(),
            failed: Box::new(self),
        }
    }

    pub fn hint(self, hint: impl Into<String>) -> Self {
        Self::Hinted {
            error: Box::new(self),
            hint: hint.into(),
        }
    }

    pub fn exit(&self) -> Exit {
        match self {
            Self::Api {
                status,
                code,
                retry_after_seconds,
                ..
            } => match (code, status) {
                (Some(ErrorCode::RateLimited | ErrorCode::Temporary), _) => Exit::Retry,
                _ if retry_after_seconds.is_some() => Exit::Retry,
                (_, 401) => Exit::Auth,
                (_, 403) => Exit::Forbidden,
                (_, 404) => Exit::NotFound,
                (_, 409) => Exit::Conflict,
                (_, 429 | 502..=504) => Exit::Retry,
                (_, 400..=499) => Exit::Usage,
                _ => Exit::Failure,
            },
            Self::Unreachable { .. } => Exit::Retry,
            Self::BadResponse { .. } | Self::Local(_) => Exit::Failure,
            Self::Usage(_) => Exit::Usage,
            Self::Failed { exit, .. } => *exit,
            Self::Hinted { error, .. } => error.exit(),
            Self::Partial { failed, .. } => failed.exit(),
        }
    }

    /// The form `--json` prints, so failures parse the same way successes do.
    pub fn to_json(&self) -> serde_json::Value {
        #[derive(Serialize)]
        struct Body<'a> {
            message: String,
            exit_code: u8,
            #[serde(skip_serializing_if = "Option::is_none")]
            code: Option<&'a ErrorCode>,
            #[serde(skip_serializing_if = "Option::is_none")]
            status: Option<u16>,
            #[serde(skip_serializing_if = "Option::is_none")]
            retry_after_seconds: Option<u64>,
        }
        let (code, status, retry_after_seconds) = match self.root() {
            Self::Api {
                code,
                status,
                retry_after_seconds,
                ..
            } => (code.as_ref(), Some(*status), *retry_after_seconds),
            _ => (None, None, None),
        };
        serde_json::json!({
            "error": Body {
                message: self.to_string(),
                exit_code: self.exit() as u8,
                code,
                status,
                retry_after_seconds,
            }
        })
    }

    fn root(&self) -> &Self {
        match self {
            Self::Partial { failed, .. } => failed.root(),
            Self::Hinted { error, .. } => error.root(),
            other => other,
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Api {
                status: 401,
                code: Some(ErrorCode::InvalidCredentials),
                ..
            } => write!(formatter, "wrong email or password"),
            Self::Api { status: 401, .. } => write!(
                formatter,
                "not signed in, or the credential expired; run `revscope login`"
            ),
            Self::Api {
                message,
                retry_after_seconds: Some(seconds),
                ..
            } => write!(formatter, "{message} (retry after {seconds}s)"),
            Self::Api {
                status, message, ..
            } if message.is_empty() => write!(formatter, "the API answered {status}"),
            Self::Api { message, .. } => write!(formatter, "{message}"),
            Self::Unreachable { url, detail } => {
                write!(formatter, "could not reach {url}: {detail}")
            }
            Self::BadResponse { path, detail } => write!(
                formatter,
                "could not read the API's answer to {path} ({detail}); upgrading this CLI may help"
            ),
            Self::Usage(message) | Self::Local(message) | Self::Failed { message, .. } => {
                write!(formatter, "{message}")
            }
            Self::Hinted { error, hint } => write!(formatter, "{error}. {hint}"),
            Self::Partial { done, failed } => write!(formatter, "{done}, but then: {failed}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api(status: u16, code: Option<ErrorCode>) -> CliError {
        CliError::Api {
            status,
            code,
            message: "nope".into(),
            retry_after_seconds: None,
        }
    }

    #[test]
    fn each_kind_of_failure_has_its_own_exit_status() {
        assert_eq!(api(401, Some(ErrorCode::InvalidSession)).exit(), Exit::Auth);
        assert_eq!(api(403, Some(ErrorCode::Forbidden)).exit(), Exit::Forbidden);
        assert_eq!(api(404, None).exit(), Exit::NotFound);
        assert_eq!(api(409, None).exit(), Exit::Conflict);
        assert_eq!(api(400, None).exit(), Exit::Usage);
        assert_eq!(api(429, None).exit(), Exit::Retry);
        assert_eq!(api(503, Some(ErrorCode::Temporary)).exit(), Exit::Retry);
        assert_eq!(api(500, None).exit(), Exit::Failure);
    }

    #[test]
    fn a_wrong_password_is_not_reported_as_an_expired_credential() {
        let wrong = api(401, Some(ErrorCode::InvalidCredentials)).to_string();
        assert_eq!(wrong, "wrong email or password");
        let expired = api(401, Some(ErrorCode::InvalidSession)).to_string();
        assert!(expired.contains("revscope login"), "{expired}");
    }

    #[test]
    fn a_partial_failure_says_what_already_happened_and_keeps_the_real_status() {
        let error = api(503, Some(ErrorCode::Temporary)).after("the project was created");
        assert!(error.to_string().starts_with("the project was created"));
        assert_eq!(error.exit(), Exit::Retry);
        assert_eq!(error.to_json()["error"]["status"], 503);
    }

    #[test]
    fn the_json_form_carries_what_a_script_branches_on() {
        let json = api(403, Some(ErrorCode::Forbidden)).to_json();
        assert_eq!(json["error"]["code"], "forbidden");
        assert_eq!(json["error"]["exit_code"], 4);
        assert_eq!(json["error"]["message"], "nope");
    }
}
