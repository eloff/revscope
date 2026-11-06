//! The OAuth documents a client reads and sends to get a token.

use serde::{Deserialize, Serialize};

/// Read-only access to every project on the account, through `/v2/mcp`.
pub const MCP_SCOPE: &str = "mcp";
/// Everything the approving user can do, for the CLI. Not advertised in discovery,
/// since an MCP client asks for what it finds there, and only a loopback redirect
/// may ask for it.
pub const CLI_SCOPE: &str = "cli";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizationServerMetadata {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub registration_endpoint: String,
    pub response_types_supported: Vec<String>,
    pub grant_types_supported: Vec<String>,
    pub code_challenge_methods_supported: Vec<String>,
    pub token_endpoint_auth_methods_supported: Vec<String>,
    pub scopes_supported: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterRequest {
    #[serde(default)]
    pub client_name: Option<String>,
    #[serde(default)]
    pub redirect_uris: Vec<String>,
    /// Accepted and echoed back, but not honoured: we only implement the
    /// authorization code grant for public clients.
    #[serde(default)]
    pub grant_types: Option<Vec<String>>,
    #[serde(default)]
    pub token_endpoint_auth_method: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterResponse {
    pub client_id: String,
    /// Seconds since the unix epoch, as RFC 7591 section 3.2.1 specifies.
    pub client_id_issued_at: i64,
    pub client_name: String,
    pub redirect_uris: Vec<String>,
    pub grant_types: Vec<String>,
    pub response_types: Vec<String>,
    pub token_endpoint_auth_method: String,
    pub scope: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    /// Seconds from now, as RFC 6749 section 5.1 specifies.
    pub expires_in: i64,
    pub scope: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TokenRequest {
    pub grant_type: String,
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub redirect_uri: Option<String>,
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub code_verifier: String,
}
