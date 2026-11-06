//! The Google calls an import makes: tokens, the properties a credential can
//! read, and report rows.

use std::time::Duration;

pub(crate) use revscope_api::ga_import::GaProperty;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::integration::REPORT_QUERY_TIMEOUT;
use crate::integration::google_auth::{self, GOOGLE_TOKEN_URL, GoogleToken, GoogleTokenError};
use crate::server_context::ServerContext;

use super::credential::GaCredential;

pub(super) const GA_SCOPE: &str = "https://www.googleapis.com/auth/analytics.readonly";
/// Below the API's 250k maximum, to keep any one response from nearing the
/// request timeout. A chunk's pages are still held in memory until it's written.
const REPORT_PAGE_SIZE: usize = 100_000;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Where the Google APIs are, so tests can answer in their place.
#[derive(Debug, Clone)]
pub(crate) struct GaEndpoints {
    pub(crate) token_url: String,
    pub(crate) revoke_url: String,
    pub(crate) data_url: String,
    pub(crate) admin_url: String,
}

impl Default for GaEndpoints {
    fn default() -> Self {
        Self {
            token_url: GOOGLE_TOKEN_URL.to_string(),
            revoke_url: "https://oauth2.googleapis.com/revoke".to_string(),
            data_url: "https://analyticsdata.googleapis.com/v1beta".to_string(),
            admin_url: "https://analyticsadmin.googleapis.com/v1beta".to_string(),
        }
    }
}

#[derive(Debug, PartialEq)]
pub(crate) enum GaApiError {
    /// The property's token quota ran out; it refills on the hour.
    Quota,
    /// Google won't let this credential in. Retrying can't help.
    Denied(String),
    /// Google didn't accept this access token, which can just mean it expired during
    /// a long report. Retried with a fresh token, like a transient failure.
    TokenRejected(String),
    /// This server's OAuth client settings are missing or wrong. An operator can fix
    /// that with a deploy, so an import waits without giving up its credential.
    ClientSettings(String),
    /// Worth trying again later: a network failure or a Google outage.
    Transient(String),
    /// Google says the request itself is wrong, so retrying can't help.
    Rejected(String),
}

impl From<reqwest::Error> for GaApiError {
    fn from(error: reqwest::Error) -> Self {
        Self::Transient(error.to_string())
    }
}

fn classify_failure(status: reqwest::StatusCode, body: &str) -> GaApiError {
    let message = format!("Google Analytics answered {status}: {body}");
    match status.as_u16() {
        429 => GaApiError::Quota,
        _ if body.contains("RESOURCE_EXHAUSTED") => GaApiError::Quota,
        401 => GaApiError::TokenRejected(message),
        403 => GaApiError::Denied(message),
        408 => GaApiError::Transient(message),
        500.. => GaApiError::Transient(message),
        _ => GaApiError::Rejected(message),
    }
}

fn oauth_not_configured() -> GaApiError {
    GaApiError::ClientSettings("Google OAuth isn't configured".to_string())
}

pub(super) async fn fetch_access_token(
    server_context: &ServerContext,
    credential: &GaCredential,
) -> Result<GoogleToken, GaApiError> {
    let client = &server_context.http;
    let token_url = server_context.ga_endpoints.token_url.as_str();
    let result = match credential {
        GaCredential::ServiceAccount {
            client_email,
            private_key,
        } => {
            google_auth::service_account_token(
                client,
                token_url,
                client_email,
                private_key,
                GA_SCOPE,
            )
            .await
        }
        GaCredential::OAuth { refresh_token } => {
            let oauth = server_context
                .google_oauth
                .as_ref()
                .ok_or_else(oauth_not_configured)?;
            google_auth::request_token(
                client,
                token_url,
                &[
                    ("grant_type", "refresh_token"),
                    ("refresh_token", refresh_token),
                    ("client_id", &oauth.client_id),
                    ("client_secret", &oauth.client_secret),
                ],
            )
            .await
        }
    };
    result.map_err(token_error)
}

/// The authorization code exchange, which is what yields a refresh token.
pub(super) async fn exchange_oauth_code(
    server_context: &ServerContext,
    code: &str,
    redirect_uri: &str,
) -> Result<GoogleToken, GaApiError> {
    let oauth = server_context
        .google_oauth
        .as_ref()
        .ok_or_else(oauth_not_configured)?;
    google_auth::request_token(
        &server_context.http,
        &server_context.ga_endpoints.token_url,
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("client_id", &oauth.client_id),
            ("client_secret", &oauth.client_secret),
        ],
    )
    .await
    .map_err(token_error)
}

fn token_error(error: GoogleTokenError) -> GaApiError {
    match error {
        GoogleTokenError::Http(error) => error.into(),
        GoogleTokenError::Unavailable(message) => GaApiError::Transient(message),
        GoogleTokenError::ClientRejected(message) => GaApiError::ClientSettings(message),
        GoogleTokenError::Rejected(message) | GoogleTokenError::InvalidCredential(message) => {
            GaApiError::Denied(message)
        }
    }
}

/// Tells Google to drop a refresh token we no longer need. A failure leaves it
/// valid but unknown to us, and the user can still remove the grant from their
/// Google account, so it's only logged. The same goes for a token we lost track of
/// because the main database failed while we held it.
pub(super) async fn revoke_refresh_token(server_context: &ServerContext, refresh_token: &str) {
    let result = server_context
        .http
        .post(&server_context.ga_endpoints.revoke_url)
        .timeout(REQUEST_TIMEOUT)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(serde_urlencoded::to_string([("token", refresh_token)]).unwrap_or_default())
        .send()
        .await;
    match result {
        Ok(response) if response.status().is_success() => {}
        Ok(response) => log::warn!(
            "revoking a Google Analytics refresh token answered {}",
            response.status()
        ),
        Err(error) => log::warn!("revoking a Google Analytics refresh token failed: {error}"),
    }
}

#[derive(Deserialize)]
struct AccountSummariesPage {
    #[serde(default, rename = "accountSummaries")]
    account_summaries: Vec<AccountSummary>,
    #[serde(default, rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
struct AccountSummary {
    #[serde(default, rename = "displayName")]
    display_name: String,
    #[serde(default, rename = "propertySummaries")]
    property_summaries: Vec<PropertySummary>,
}

#[derive(Deserialize)]
struct PropertySummary {
    property: String,
    #[serde(default, rename = "displayName")]
    display_name: String,
}

/// Every GA4 property the token can read, across all its accounts and pages.
pub(super) async fn list_properties(
    server_context: &ServerContext,
    access_token: &str,
) -> Result<Vec<GaProperty>, GaApiError> {
    let url = format!("{}/accountSummaries", server_context.ga_endpoints.admin_url);
    let mut properties = Vec::new();
    let mut page_token: Option<String> = None;
    loop {
        let mut query = vec![("pageSize", "200")];
        if let Some(token) = page_token.as_deref() {
            query.push(("pageToken", token));
        }
        let response = server_context
            .http
            .get(&url)
            .timeout(REQUEST_TIMEOUT)
            .bearer_auth(access_token)
            .query(&query)
            .send()
            .await?;
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            return Err(classify_failure(status, &text));
        }
        let page: AccountSummariesPage = serde_json::from_str(&text)
            .map_err(|error| GaApiError::Transient(format!("unreadable property list: {error}")))?;
        for account in page.account_summaries {
            for property in account.property_summaries {
                if let Some(id) = property.property.strip_prefix("properties/") {
                    properties.push(GaProperty {
                        id: id.to_string(),
                        name: property.display_name,
                        account: account.display_name.clone(),
                    });
                }
            }
        }
        match page.next_page_token.filter(|token| !token.is_empty()) {
            Some(token) => page_token = Some(token),
            None => return Ok(properties),
        }
    }
}

/// One report row: its dimension values, then its metrics as integers.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct ReportRow {
    pub(super) dimensions: Vec<String>,
    pub(super) metrics: Vec<i64>,
}

#[derive(Deserialize)]
struct ReportPage {
    #[serde(default)]
    rows: Vec<RawRow>,
    #[serde(default, rename = "rowCount")]
    row_count: usize,
}

#[derive(Deserialize)]
struct RawRow {
    #[serde(default, rename = "dimensionValues")]
    dimension_values: Vec<RawValue>,
    #[serde(default, rename = "metricValues")]
    metric_values: Vec<RawValue>,
}

#[derive(Deserialize)]
struct RawValue {
    #[serde(default)]
    value: String,
}

/// Every row of a report, following its pages.
pub(super) async fn run_report(
    server_context: &ServerContext,
    access_token: &str,
    property_id: &str,
    request: &Value,
) -> Result<Vec<ReportRow>, GaApiError> {
    let url = format!(
        "{}/properties/{property_id}:runReport",
        server_context.ga_endpoints.data_url
    );
    let mut rows = Vec::new();
    loop {
        let mut body = request.clone();
        body["limit"] = json!(REPORT_PAGE_SIZE);
        body["offset"] = json!(rows.len());
        let response = server_context
            .http
            .post(&url)
            .timeout(REPORT_QUERY_TIMEOUT)
            .bearer_auth(access_token)
            .json(&body)
            .send()
            .await?;
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            return Err(classify_failure(status, &text));
        }
        let page: ReportPage = serde_json::from_str(&text)
            .map_err(|error| GaApiError::Transient(format!("unreadable report: {error}")))?;
        let received = page.rows.len();
        rows.extend(page.rows.into_iter().map(into_report_row));
        if received == 0 || rows.len() >= page.row_count {
            return Ok(rows);
        }
    }
}

fn into_report_row(row: RawRow) -> ReportRow {
    ReportRow {
        dimensions: row
            .dimension_values
            .into_iter()
            .map(|value| value.value)
            .collect(),
        metrics: row
            .metric_values
            .iter()
            .map(|value| parse_metric(&value.value))
            .collect(),
    }
}

/// Metrics arrive as strings, and durations can have a fractional part.
fn parse_metric(value: &str) -> i64 {
    value
        .parse::<i64>()
        .ok()
        .or_else(|| {
            value
                .parse::<f64>()
                .ok()
                .map(|parsed| parsed.round() as i64)
        })
        .unwrap_or(0)
        .max(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::StatusCode;

    #[test]
    fn metrics_round_and_never_go_negative() {
        assert_eq!(parse_metric("12"), 12);
        assert_eq!(parse_metric("12.6"), 13);
        assert_eq!(parse_metric("-1"), 0);
        assert_eq!(parse_metric("nonsense"), 0);
    }

    #[test]
    fn failures_are_sorted_by_what_can_be_done_about_them() {
        let classify = |status, body| classify_failure(status, body);
        assert_eq!(
            classify(StatusCode::TOO_MANY_REQUESTS, ""),
            GaApiError::Quota
        );
        assert_eq!(
            classify(StatusCode::FORBIDDEN, "RESOURCE_EXHAUSTED"),
            GaApiError::Quota
        );
        assert!(matches!(
            classify(StatusCode::FORBIDDEN, "no access"),
            GaApiError::Denied(_)
        ));
        assert!(matches!(
            classify(StatusCode::UNAUTHORIZED, "expired"),
            GaApiError::TokenRejected(_)
        ));
        assert!(matches!(
            classify(StatusCode::SERVICE_UNAVAILABLE, ""),
            GaApiError::Transient(_)
        ));
        assert!(matches!(
            classify(StatusCode::BAD_REQUEST, "bad dimension"),
            GaApiError::Rejected(_)
        ));
    }
}
