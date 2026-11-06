//! Environment configuration for the Ask tab's AI gateway.
//!
//! The gateway base URL is configurable rather than derived from an account id
//! so tests can point the agent at a local stub that replays canned responses.
//! When the configuration is absent the chat routes report 503 and the
//! dashboard hides the tab, which keeps local development working without keys.

use std::env;
use std::fmt;

/// Cloudflare's OpenAI-compatible endpoint lives under this path.
const COMPLETIONS_PATH: &str = "/chat/completions";
const DEFAULT_MODEL: &str = "grok/grok-4.7";
/// How long the gateway has to produce a whole turn, tool calls included.
pub(crate) const REQUEST_TIMEOUT_SECONDS: u64 = 180;

#[derive(Debug, Clone)]
pub(crate) struct ChatConfig {
    /// Fully qualified chat completions URL.
    pub(crate) completions_url: String,
    /// xAI key, forwarded by the gateway to the provider.
    pub(crate) api_key: String,
    /// Set only when the gateway itself is authenticated.
    pub(crate) gateway_token: Option<String>,
    pub(crate) model: String,
}

#[derive(Debug)]
pub(crate) struct MissingChatConfig(&'static str);

impl fmt::Display for MissingChatConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "chat is not configured: {} is unset", self.0)
    }
}

impl ChatConfig {
    pub(crate) fn from_env() -> Result<Self, MissingChatConfig> {
        let base =
            optional_env("CF_AI_GATEWAY_URL").ok_or(MissingChatConfig("CF_AI_GATEWAY_URL"))?;
        let api_key = optional_env("XAI_API_KEY").ok_or(MissingChatConfig("XAI_API_KEY"))?;
        Ok(Self {
            completions_url: completions_url(&base),
            api_key,
            gateway_token: optional_env("CF_AIG_TOKEN"),
            model: optional_env("REVSCOPE_CHAT_MODEL").unwrap_or_else(|| DEFAULT_MODEL.to_string()),
        })
    }

    /// True when the deployment can serve chat at all. The dashboard asks so it
    /// can hide the tab instead of offering a button that always fails.
    pub(crate) fn is_configured() -> bool {
        Self::from_env().is_ok()
    }
}

/// Accepts either the gateway base (`.../compat`) or a full completions URL, so
/// operators can paste whichever one Cloudflare showed them.
fn completions_url(base: &str) -> String {
    let trimmed = base.trim_end_matches('/');
    if trimmed.ends_with(COMPLETIONS_PATH) {
        return trimmed.to_string();
    }
    format!("{trimmed}{COMPLETIONS_PATH}")
}

fn optional_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_the_completions_path_to_a_gateway_base() {
        assert_eq!(
            completions_url("https://gateway.ai.cloudflare.com/v1/acct/gw/compat"),
            "https://gateway.ai.cloudflare.com/v1/acct/gw/compat/chat/completions"
        );
        assert_eq!(
            completions_url("https://gateway.ai.cloudflare.com/v1/acct/gw/compat/"),
            "https://gateway.ai.cloudflare.com/v1/acct/gw/compat/chat/completions"
        );
    }

    #[test]
    fn keeps_a_url_that_already_names_the_endpoint() {
        assert_eq!(
            completions_url("http://127.0.0.1:9999/chat/completions"),
            "http://127.0.0.1:9999/chat/completions"
        );
    }
}
