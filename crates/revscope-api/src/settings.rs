//! A project's settings, including its integrations' credentials.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsResponse {
    pub timezone: String,
    pub payment_success_path: Option<String>,
    /// First-party CNAME hostname for the tracker, or `None` when unset.
    pub custom_domain: Option<String>,
    pub integrations: Vec<IntegrationView>,
}

/// A stored integration with credentials masked for display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrationView {
    pub integration_type: String,
    pub external_account_id: Option<String>,
    /// Masked preview of the API key, or `None` when no key is stored.
    pub api_key_masked: Option<String>,
    /// Masked preview of the secret, or `None` when no secret is stored.
    pub secret_masked: Option<String>,
    /// `started_at` of the last successful sync with the current credentials and account,
    /// or `None` if there hasn't been one.
    pub last_synced_at: Option<String>,
    /// `error_text` from the most recent sync attempt with the current credentials and
    /// account, or `None` on success / none yet.
    pub last_sync_error: Option<String>,
    /// `started_at` of the most recent such attempt, finished or not. The same format
    /// as `last_synced_at`, so the two are equal once that attempt has succeeded.
    #[serde(default)]
    pub last_sync_attempted_at: Option<String>,
}

impl IntegrationView {
    /// Clearing an integration's credentials leaves its row behind, so a row is
    /// only a connection while it still holds one.
    pub fn is_connected(&self) -> bool {
        self.api_key_masked.is_some() || self.secret_masked.is_some()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SaveSettingsRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payment_success_path: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_domain: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub integrations: Vec<IntegrationInput>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct IntegrationInput {
    pub integration_type: String,
    /// Always written: absent or null stores NULL, unlike the credentials below.
    pub external_account_id: Option<String>,
    /// `null`/absent keeps the stored key, `""` clears it, otherwise replaces it.
    pub api_key: Option<String>,
    /// Same semantics as [`IntegrationInput::api_key`].
    pub secret: Option<String>,
}

impl std::fmt::Debug for IntegrationInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IntegrationInput")
            .field("integration_type", &self.integration_type)
            .field("external_account_id", &self.external_account_id)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("secret", &self.secret.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}
