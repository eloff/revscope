//! Importing history from Google Analytics.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthUrlResponse {
    pub url: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ConnectRequest {
    ServiceAccount {
        service_account_json: String,
    },
    OAuth {
        oauth_code: String,
        oauth_state: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectResponse {
    pub connection_id: String,
    pub properties: Vec<GaProperty>,
    /// Hostnames the project already has pages on, to preselect in the import form.
    pub known_hostnames: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GaProperty {
    /// The digits of `properties/123`.
    pub id: String,
    pub name: String,
    pub account: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartImportRequest {
    pub connection_id: String,
    pub property_id: String,
    pub start_date: String,
    pub end_date: String,
    #[serde(default)]
    pub hostnames: Vec<String>,
    /// The admin saw the warning that this range overlaps our own tracking.
    #[serde(default)]
    pub allow_duplicates: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportView {
    pub id: i32,
    pub property_name: String,
    pub start_date: String,
    pub end_date: String,
    pub hostnames: Vec<String>,
    pub status: String,
    pub chunks_done: i32,
    pub chunks_total: i32,
    /// When a paused or retrying import tries again.
    pub resume_at: String,
    pub error: Option<String>,
    pub sessions_imported: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportsResponse {
    /// Whether the "Connect with Google" button can work on this server.
    pub oauth_available: bool,
    /// The oldest start the plan allows; absent when it keeps everything.
    pub oldest_allowed_date: Option<String>,
    /// Latest allowed end, since a day isn't final until it's over.
    pub latest_allowed_date: String,
    pub default_start_date: String,
    /// Earlier than the default start when tracking covers the whole allowed window,
    /// which leaves nothing to import without duplicates.
    pub default_end_date: String,
    /// The date of the project's first tracked pageview. An end date on or after it
    /// duplicates visits and pageviews.
    pub first_tracked_date: Option<String>,
    pub include_in_reports: bool,
    pub imports: Vec<ImportView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeletedImport {
    pub id: i32,
}
