//! Signing up, signing in, and who is signed in.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

/// The cookie a browser session travels in.
pub const SESSION_COOKIE: &str = "rr_session";

/// Marks a long-lived CLI credential in an `Authorization: Bearer` header, so a
/// leaked one is recognizable. What follows it is a session id.
pub const API_TOKEN_PREFIX: &str = "rsc_";

/// What a signup needs besides the user's credentials. On revscope.co the website
/// names the new account; on a self-hosted build, where every user joins the one
/// shared account, it only suggests the project name.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NewAccountRequest {
    #[serde(default)]
    pub account_name: String,
    /// The first project's name. Missing, it comes from `account_name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_name: Option<String>,
    /// The plan chosen. Read only by builds with the private billing module.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub plan: String,
    /// The event volume chosen. The free plan ignores it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub events: String,
    /// The billing period chosen. The free plan ignores it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub billing: String,
    /// IANA zone; missing or invalid values become UTC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SignupRequest {
    pub name: String,
    pub email: String,
    /// May be empty with `issue_token` on revscope.co: only the token then reaches the
    /// account, until its owner sets a password with a reset link.
    #[serde(default)]
    pub password: String,
    #[serde(flatten)]
    pub account: NewAccountRequest,
    /// Answer with [`MeResponse::api_token`] instead of a session cookie.
    #[serde(default, skip_serializing_if = "is_false")]
    pub issue_token: bool,
}

fn is_false(value: &bool) -> bool {
    !value
}

impl fmt::Debug for SignupRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SignupRequest")
            .field("name", &self.name)
            .field("email", &self.email)
            .field("password", &"<redacted>")
            .field("account", &self.account)
            .field("issue_token", &self.issue_token)
            .finish()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
    /// Answer with [`MeResponse::api_token`] instead of a session cookie.
    #[serde(default, skip_serializing_if = "is_false")]
    pub issue_token: bool,
}

impl fmt::Debug for LoginRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LoginRequest")
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .field("issue_token", &self.issue_token)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgotPasswordRequest {
    pub email: String,
}

/// The body of `/v2/session/me`, and of a successful signup or login, so a client
/// that just authenticated never needs a second request to learn who it is.
#[derive(Clone, Serialize, Deserialize)]
pub struct MeResponse {
    pub name: String,
    pub email: String,
    pub account_id: String,
    pub user_id: String,
    /// Every project the user may read, by name.
    pub projects: BTreeMap<String, MeProject>,
    pub plan: String,
    pub is_admin: bool,
    /// Whether someone on the account has confirmed their email address, by a
    /// link we emailed, a password reset, or signing in with Google or Apple. Until then Ask AI,
    /// GA imports and integrations answer `email_unverified`. Self-hosted
    /// servers don't verify, and older servers don't say, so both read as true.
    #[serde(default = "verified_unless_told_otherwise")]
    pub account_confirmed: bool,
    pub gravatar_url: String,
    /// Months of history the account's dashboards show, counted back from
    /// today; absent when they show all of it. Older data is kept, just not shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_retention_months: Option<i64>,
    /// A paid subscription that ended because it wasn't paid for, which moved the
    /// account onto the free plan. Absent once a new subscription is paid for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unpaid_subscription: Option<UnpaidSubscription>,
    /// Only on a signup or login that asked for it, and only then: the server keeps
    /// just its hash. Send it as `Authorization: Bearer`. Revoked by logout, and by
    /// a password reset, like any other session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_token: Option<String>,
}

fn verified_unless_told_otherwise() -> bool {
    true
}

/// The terms of a subscription that ended unpaid, and what restoring them costs:
/// one billing period at those terms, starting a new period the day it's paid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnpaidSubscription {
    pub plan: String,
    /// `None` means the plan is unmetered.
    pub events_monthly_limit: Option<i32>,
    /// `monthly` or `yearly`.
    pub billing_period: String,
    pub amount_cents: i32,
    /// Lowercase ISO 4217, e.g. `usd`.
    pub currency: String,
    /// When the account moved onto the free plan, in unix seconds.
    pub ended_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeProject {
    pub id: String,
    pub settings: MeProjectSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeProjectSettings {
    pub timezone: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_password_never_reaches_debug_output() {
        let signup = SignupRequest {
            name: "Ada".into(),
            email: "ada@example.com".into(),
            password: "hunter2-hunter2".into(),
            account: NewAccountRequest::default(),
            issue_token: false,
        };
        assert!(!format!("{signup:?}").contains("hunter2"));
        let login = LoginRequest {
            username: "ada@example.com".into(),
            password: "hunter2-hunter2".into(),
            issue_token: false,
        };
        assert!(!format!("{login:?}").contains("hunter2"));
    }

    #[test]
    fn account_fields_sit_beside_the_credentials_on_the_wire() {
        let signup = SignupRequest {
            name: "Ada".into(),
            email: "ada@example.com".into(),
            password: "x".into(),
            account: NewAccountRequest {
                account_name: "example.com".into(),
                plan: "free".into(),
                ..NewAccountRequest::default()
            },
            issue_token: false,
        };
        let wire = serde_json::to_value(&signup).unwrap();
        assert_eq!(wire["account_name"], "example.com");
        assert_eq!(wire["plan"], "free");
        assert!(wire.get("events").is_none());
        assert!(wire.get("issue_token").is_none());
    }
}
