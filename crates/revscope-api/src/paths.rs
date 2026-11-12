//! Route paths, as the server resolves them. A server test walks [`all_routes`]
//! through its router, so a path here can't name a route that doesn't exist.

use crate::troubleshooting::TroubleshootingIngestQuery;

pub const SIGNUP: &str = "/v2/public/auth/signup";
pub const LOGIN: &str = "/v2/public/auth/login";
pub const LOGOUT: &str = "/v2/public/auth/logout";
pub const FORGOT_PASSWORD: &str = "/v2/public/auth/forgot-password";
/// Spends the token from a verification email.
pub const VERIFY_EMAIL: &str = "/v2/public/auth/verify-email";
/// Sends the signed-in user another verification email.
pub const RESEND_VERIFICATION: &str = "/v2/session/me/verify-email";
pub const OAUTH_REGISTER: &str = "/v2/public/mcp/register";
pub const OAUTH_TOKEN: &str = "/v2/public/mcp/token";
pub const OAUTH_METADATA: &str = "/.well-known/oauth-authorization-server";
pub const ME: &str = "/v2/session/me";
pub const PROJECTS: &str = "/v2/session/projects";

fn session_project(project_id: &str, rest: &str) -> String {
    format!("/v2/session/projects/{project_id}/{rest}")
}

pub fn settings(project_id: &str) -> String {
    session_project(project_id, "settings")
}

pub fn install(project_id: &str) -> String {
    session_project(project_id, "install")
}

pub fn troubleshooting(project_id: &str) -> String {
    session_project(project_id, "troubleshooting")
}

pub fn troubleshooting_check(project_id: &str) -> String {
    session_project(project_id, "troubleshooting/check")
}

pub fn troubleshooting_ingest(project_id: &str, query: &TroubleshootingIngestQuery) -> String {
    let query = serde_urlencoded::to_string(query).expect("a flat struct always encodes");
    format!(
        "{}?{query}",
        session_project(project_id, "troubleshooting/ingest")
    )
}

pub fn query_sql(project_id: &str) -> String {
    format!("/v2/query/projects/{project_id}/sql")
}

pub fn query_schema(project_id: &str) -> String {
    format!("/v2/query/projects/{project_id}/schema")
}

pub fn ga_imports(project_id: &str) -> String {
    session_project(project_id, "ga-imports")
}

pub fn ga_import(project_id: &str, import_id: i32) -> String {
    format!("{}?id={import_id}", ga_imports(project_id))
}

pub fn ga_import_cancel(project_id: &str, import_id: i32) -> String {
    format!(
        "{}?id={import_id}",
        session_project(project_id, "ga-imports/cancel")
    )
}

pub fn ga_import_connect(project_id: &str) -> String {
    session_project(project_id, "ga-imports/connect")
}

pub fn ga_import_oauth_url(project_id: &str) -> String {
    session_project(project_id, "ga-imports/oauth-url")
}

/// Every `(method, path)` above that goes through the API router. The OAuth
/// metadata document is answered ahead of it, so it isn't one of them.
pub fn all_routes() -> Vec<(&'static str, String)> {
    let id = "1";
    let ingest = TroubleshootingIngestQuery {
        since_ms: 0,
        hostname: Some("example.com".to_string()),
    };
    vec![
        ("POST", SIGNUP.to_string()),
        ("POST", LOGIN.to_string()),
        ("POST", LOGOUT.to_string()),
        ("POST", FORGOT_PASSWORD.to_string()),
        ("POST", VERIFY_EMAIL.to_string()),
        ("POST", RESEND_VERIFICATION.to_string()),
        ("POST", OAUTH_REGISTER.to_string()),
        ("POST", OAUTH_TOKEN.to_string()),
        ("GET", ME.to_string()),
        ("POST", PROJECTS.to_string()),
        ("GET", settings(id)),
        ("POST", settings(id)),
        ("GET", install(id)),
        ("GET", troubleshooting(id)),
        ("POST", troubleshooting_check(id)),
        ("GET", troubleshooting_ingest(id, &ingest)),
        ("POST", query_sql(id)),
        ("GET", query_schema(id)),
        ("GET", ga_imports(id)),
        ("POST", ga_imports(id)),
        ("DELETE", ga_import(id, 1)),
        ("POST", ga_import_cancel(id, 1)),
        ("POST", ga_import_connect(id)),
        ("POST", ga_import_oauth_url(id)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ingest_query_is_url_encoded() {
        let query = TroubleshootingIngestQuery {
            since_ms: 5,
            hostname: Some("a b.example".to_string()),
        };
        assert_eq!(
            troubleshooting_ingest("7", &query),
            "/v2/session/projects/7/troubleshooting/ingest?since_ms=5&hostname=a+b.example"
        );
    }
}
