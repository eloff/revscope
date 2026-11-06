//! Checking that a site's tracker is installed and reporting.

use serde::{Deserialize, Serialize};

/// How an integration's latest sync went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncStatus {
    NeverRun,
    Ok,
    Error,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TroubleshootingStatusResponse {
    pub project_name: String,
    /// Suggested URL for the install check, from the account website name.
    pub default_url: String,
    /// Pageviews + events written to the database in the last ~30 days.
    pub events_last_month: i64,
    /// Pageviews + events received but not yet written. The server flushes each
    /// project's batch every five minutes, so a site that just started reporting has
    /// these and nothing above.
    #[serde(default)]
    pub events_queued: i64,
    pub integrations: Vec<IntegrationStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrationStatus {
    pub integration_type: String,
    pub last_synced_at: Option<String>,
    pub last_sync_error: Option<String>,
    pub status: SyncStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TroubleshootingCheckRequest {
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TroubleshootingCheckResponse {
    pub url: String,
    pub fetch: FetchResult,
    pub install: InstallResult,
    /// Server clock when the check finished; poll ingest for hits at or after this.
    pub wait_started_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchResult {
    pub ok: bool,
    pub status_code: Option<u16>,
    pub error: Option<String>,
    /// Set when the response body may be incomplete (SPA shell, truncated).
    pub note: Option<String>,
    /// Where any redirects ended, which is the host visits are recorded under.
    #[serde(default)]
    pub final_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallResult {
    pub ok: bool,
    pub findings: Vec<InstallFinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallFinding {
    pub severity: Severity,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Ok,
    Warning,
    Error,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TroubleshootingIngestQuery {
    /// Unix milliseconds; only hits with `timestamp_id` at or after this count.
    pub since_ms: i64,
    /// Only count hits on this host, or on the same host with or without `www.`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TroubleshootingIngestResponse {
    pub detected: bool,
    /// Unix milliseconds of the detected hit (`timestamp_id` / 1e6), when present.
    pub detected_at_ms: Option<i64>,
    pub path: Option<String>,
    pub hostname: Option<String>,
}
