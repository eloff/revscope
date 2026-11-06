//! Read-only SQL against a project's analytics database.

use serde::{Deserialize, Serialize};

/// Rows in a page when the request doesn't say.
pub const DEFAULT_LIMIT: u32 = 100;
/// The server clamps a larger `limit` to this.
pub const MAX_LIMIT: u32 = 500;
/// The server clamps a larger `offset` to this, which would repeat rows, so a
/// client should refuse to ask for more.
pub const MAX_OFFSET: u32 = 5_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryRequest {
    pub sql: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResponse {
    pub columns: Vec<String>,
    /// One entry per row, in `columns` order. Blobs are base64 encoded.
    pub rows: Vec<Vec<serde_json::Value>>,
    /// The clamped values actually applied, which may differ from the request.
    pub limit: u32,
    pub offset: u32,
    /// More rows were available past this page (hit `limit`, size, or time cap),
    /// so the next page starts `rows.len()` further on, not `limit`.
    pub has_more: bool,
}
