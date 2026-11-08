//! Creating projects.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateProjectRequest {
    /// A website (`https://shop.example.com/`) or a bare project name.
    pub website: String,
    /// The project's name, used instead of the one derived from `website`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_name: Option<String>,
    /// IANA zone for the new project's dashboards; UTC when missing or unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateProjectResponse {
    pub id: String,
    pub name: String,
}
