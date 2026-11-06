use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;
use serde_json::Value;

static PARTNER_REF_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^partner[0-9a-f]+$").expect("valid partner ref regex"));

static GOOGLE_REFERRER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^google\.(?:com|[a-z]{2,3}(?:\.[a-z]{2})?)(?:/|$)")
        .expect("google referrer regex must compile")
});

static SEARCH_ENGINE_REFERRER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^(?:google\.(?:com|[a-z]{2,3}(?:\.[a-z]{2})?)|(?:[\w-]+\.)?bing\.com|(?:[\w-]+\.)?yahoo\.(?:com|co\.[a-z]{2})|duckduckgo\.com|search\.brave\.com|yandex\.(?:com|ru|by|kz|uz)|baidu\.com|ecosia\.org|startpage\.com|ask\.com|aol\.com|naver\.com|qwant\.com|swisscows\.com|mojeek\.com|kagi\.com)(?:/|$)",
    )
    .expect("search engine referrer regex must compile")
});

/// Mirrors `tracker.js` referrer normalization: strip scheme, one mobile/www-like
/// prefix, and any path/query fragment; return lowercase hostname.
static NORMALIZE_REFERRER_HOST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:https?://)?(?:(?:m|l|w{2,3}(?:[0-9]+)?)\.)?([^/?#]+).*$")
        .expect("referrer host normalize regex must compile")
});

/// Normalize a referrer string to hostname only (lowercase).
pub fn normalize_referrer_hostname(referrer: &str) -> Option<String> {
    let referrer = referrer.trim();
    if referrer.is_empty() {
        return None;
    }
    let host = if let Some(captures) = NORMALIZE_REFERRER_HOST.captures(referrer) {
        captures.get(1)?.as_str()
    } else {
        referrer.split(['/', '?', '#']).next()?
    };
    let host = host.trim();
    if host.is_empty() {
        return None;
    }
    Some(host.to_ascii_lowercase())
}

/// Loopback hosts are local preview traffic, not an external source.
/// `host` is a normalized referrer hostname, optionally with a port.
pub fn referrer_host_is_loopback(host: &str) -> bool {
    let host = match host.rsplit_once(':') {
        Some((name, port))
            if !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            name
        }
        _ => host,
    };
    host == "localhost" || host == "127.0.0.1"
}

const SOCIAL_REFERRER_HOSTS: &[&str] = &[
    "facebook.com",
    "fb.com",
    "m.facebook.com",
    "instagram.com",
    "linkedin.com",
    "lnkd.in",
    "twitter.com",
    "t.co",
    "x.com",
    "tiktok.com",
    "pinterest.com",
    "reddit.com",
    "youtube.com",
    "youtu.be",
    "snapchat.com",
    "threads.net",
];

const AI_PROVIDER_UTM_SOURCE_HOSTS: &[(&str, &str)] = &[
    ("anthropic", "claude.ai"),
    ("bard", "gemini.google.com"),
    ("chat.openai.com", "chat.openai.com"),
    ("chatgpt", "chatgpt.com"),
    ("chatgpt.com", "chatgpt.com"),
    ("claude", "claude.ai"),
    ("claude.ai", "claude.ai"),
    ("copilot", "copilot.microsoft.com"),
    ("copilot.microsoft.com", "copilot.microsoft.com"),
    ("gemini", "gemini.google.com"),
    ("gemini.google.com", "gemini.google.com"),
    ("grok", "grok.com"),
    ("grok.com", "grok.com"),
    ("openai", "chatgpt.com"),
    ("openai.com", "openai.com"),
    ("perplexity", "perplexity.ai"),
    ("perplexity.ai", "perplexity.ai"),
    ("phind", "phind.com"),
    ("phind.com", "phind.com"),
    ("poe", "poe.com"),
    ("poe.com", "poe.com"),
    ("you.com", "you.com"),
];

const AI_PROVIDER_HOSTS: &[&str] = &[
    "chat.openai.com",
    "chatgpt.com",
    "claude.ai",
    "copilot.microsoft.com",
    "gemini.google.com",
    "grok.com",
    "openai.com",
    "perplexity.ai",
    "phind.com",
    "poe.com",
    "you.com",
];

const EMAIL_UTM_SOURCES: &[&str] = &["newsletter", "email", "mailchimp"];

/// Whether a normalized referrer string points at Google Search.
pub fn referrer_is_google(referrer: &str) -> bool {
    GOOGLE_REFERRER.is_match(referrer)
}

/// Whether a normalized referrer string points at a known search engine.
pub fn referrer_is_search(referrer: &str) -> bool {
    SEARCH_ENGINE_REFERRER.is_match(referrer)
}

pub fn referrer_is_ai(referrer: &str) -> bool {
    referrer_host_matches(referrer, AI_PROVIDER_HOSTS)
}

pub fn ai_provider_referrer_from_source(source: &str) -> Option<&'static str> {
    let source = source.trim();
    if source.is_empty() {
        return None;
    }
    AI_PROVIDER_UTM_SOURCE_HOSTS
        .iter()
        .find(|(candidate, _)| strings_eq_ignore_case(source, candidate))
        .map(|(_, host)| *host)
}

fn referrer_is_social(referrer: &str) -> bool {
    referrer_host_matches(referrer, SOCIAL_REFERRER_HOSTS)
}

fn referrer_host_matches(referrer: &str, hosts: &[&str]) -> bool {
    let lower = referrer.to_ascii_lowercase();
    hosts.iter().any(|host| {
        lower == *host
            || lower.starts_with(&format!("{host}/"))
            || lower == format!("www.{host}")
            || lower.starts_with(&format!("www.{host}/"))
    })
}

/// Stored on `landings.referrer_type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferrerType {
    Ai,
    Search,
    Social,
    Email,
    Campaign,
    Affiliate,
}

impl ReferrerType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ai => "ai",
            Self::Search => "search",
            Self::Social => "social",
            Self::Email => "email",
            Self::Campaign => "campaign",
            Self::Affiliate => "affiliate",
        }
    }

    /// Parse a value from `landings.referrer_type`.
    pub fn from_stored(value: &str) -> Option<Self> {
        if strings_eq_ignore_case(value, "ai") {
            Some(Self::Ai)
        } else if strings_eq_ignore_case(value, "search") {
            Some(Self::Search)
        } else if strings_eq_ignore_case(value, "social") {
            Some(Self::Social)
        } else if strings_eq_ignore_case(value, "email") {
            Some(Self::Email)
        } else if strings_eq_ignore_case(value, "campaign") {
            Some(Self::Campaign)
        } else if strings_eq_ignore_case(value, "affiliate") {
            Some(Self::Affiliate)
        } else {
            None
        }
    }
}

impl std::fmt::Display for ReferrerType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Classify a landing into `landings.referrer_type` from the normalized referrer URL
/// and optional decoded landing query params.
pub fn classify_referrer_type(
    referrer: Option<&str>,
    params: Option<&serde_json::Map<String, Value>>,
) -> Option<ReferrerType> {
    if let Some(referrer) = referrer.filter(|value| !value.is_empty()) {
        if referrer_is_ai(referrer) {
            return Some(ReferrerType::Ai);
        }
        if referrer_is_search(referrer) {
            return Some(ReferrerType::Search);
        }
        if referrer_is_social(referrer) {
            return Some(ReferrerType::Social);
        }
    }
    params.and_then(classify_params_from_map)
}

fn classify_params_from_map(obj: &serde_json::Map<String, Value>) -> Option<ReferrerType> {
    let source = get_param(obj, "source");
    let campaign = get_param(obj, "campaign");
    let medium = get_param(obj, "medium");
    let ref_param = get_param(obj, "ref");

    if medium.is_some_and(|value| strings_eq_ignore_case(value, "email")) {
        return Some(ReferrerType::Email);
    }
    if source.is_some_and(|value| {
        EMAIL_UTM_SOURCES
            .iter()
            .any(|candidate| strings_eq_ignore_case(value, candidate))
    }) {
        return Some(ReferrerType::Email);
    }
    if ref_param.is_some_and(|value| PARTNER_REF_PATTERN.is_match(value)) {
        return Some(ReferrerType::Affiliate);
    }
    if campaign.is_some() {
        return Some(ReferrerType::Campaign);
    }
    None
}

fn get_param<'a>(obj: &'a serde_json::Map<String, Value>, key: &str) -> Option<&'a str> {
    obj.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

fn strings_eq_ignore_case(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}

/// Marketing-channel bucket for revenue-sources UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceGroup {
    Search,
    Ai,
    Social,
    Campaigns,
    Referrers,
    Direct,
}

impl SourceGroup {
    pub const ALL: [Self; 6] = [
        Self::Search,
        Self::Ai,
        Self::Social,
        Self::Campaigns,
        Self::Referrers,
        Self::Direct,
    ];

    /// Matches the serde name, so it round-trips through the `channel` segment filter.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Search => "search",
            Self::Ai => "ai",
            Self::Social => "social",
            Self::Campaigns => "campaigns",
            Self::Referrers => "referrers",
            Self::Direct => "direct",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|group| group.as_str() == value)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Search => "Search",
            Self::Ai => "AI",
            Self::Social => "Social",
            Self::Campaigns => "Campaigns",
            Self::Referrers => "Referrers",
            Self::Direct => "Direct",
        }
    }
}

/// Channel bucket for a referrer row with no campaign.
pub fn group_for_referrer_type(referrer_type: Option<ReferrerType>) -> SourceGroup {
    match referrer_type {
        None => SourceGroup::Referrers,
        Some(ReferrerType::Search) => SourceGroup::Search,
        Some(ReferrerType::Social) => SourceGroup::Social,
        Some(ReferrerType::Email | ReferrerType::Campaign | ReferrerType::Affiliate) => {
            SourceGroup::Campaigns
        }
        Some(ReferrerType::Ai) => SourceGroup::Ai,
    }
}

/// Channel bucket for a landing row. Campaign landings are always counted in the
/// campaigns channel, no-referrer landings are direct, and referrer landings
/// inherit their channel from `referrer_type`.
pub fn source_group_for_landing(
    has_campaign: bool,
    has_referrer: bool,
    referrer_type: Option<ReferrerType>,
) -> SourceGroup {
    if has_campaign {
        SourceGroup::Campaigns
    } else if !has_referrer {
        SourceGroup::Direct
    } else {
        group_for_referrer_type(referrer_type)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::{
        ReferrerType, ai_provider_referrer_from_source, classify_params_from_map,
        classify_referrer_type, normalize_referrer_hostname, referrer_host_is_loopback,
        referrer_is_ai, referrer_is_google, referrer_is_search, referrer_is_social,
    };

    fn params_map(value: serde_json::Value) -> serde_json::Map<String, Value> {
        value.as_object().cloned().expect("object params")
    }

    #[test]
    fn normalize_referrer_hostname_strips_scheme_path_and_www() {
        assert_eq!(
            normalize_referrer_hostname("https://www.google.com/search?q=foo").as_deref(),
            Some("google.com")
        );
        assert_eq!(
            normalize_referrer_hostname("google.com/search").as_deref(),
            Some("google.com")
        );
        assert_eq!(
            normalize_referrer_hostname("https://m.facebook.com/photo").as_deref(),
            Some("facebook.com")
        );
        assert_eq!(
            normalize_referrer_hostname("localhost:3445/landing.html").as_deref(),
            Some("localhost:3445")
        );
    }

    #[test]
    fn referrer_host_is_loopback_matches_localhost_and_ipv4_loopback() {
        assert!(referrer_host_is_loopback("localhost"));
        assert!(referrer_host_is_loopback("localhost:3445"));
        assert!(referrer_host_is_loopback("127.0.0.1"));
        assert!(referrer_host_is_loopback("127.0.0.1:8080"));
        assert!(!referrer_host_is_loopback("localhost.example.com"));
        assert!(!referrer_host_is_loopback("127.0.0.10"));
        assert!(!referrer_host_is_loopback("example.com"));
    }

    #[test]
    fn classifies_search_from_referrer() {
        assert_eq!(
            classify_referrer_type(Some("google.com/search"), None),
            Some(ReferrerType::Search)
        );
        assert_eq!(
            classify_referrer_type(Some("duckduckgo.com/"), None),
            Some(ReferrerType::Search)
        );
        assert!(referrer_is_search("bing.com/search"));
        assert!(referrer_is_google("google.com/search"));
        assert!(referrer_is_google("google.co.uk"));
        assert!(!referrer_is_google("bing.com/search"));
    }

    #[test]
    fn classifies_ai_from_referrer_hostname() {
        assert_eq!(
            classify_referrer_type(Some("chatgpt.com"), None),
            Some(ReferrerType::Ai)
        );
        assert!(referrer_is_ai("www.perplexity.ai"));
    }

    #[test]
    fn maps_known_ai_provider_utm_sources_to_hostnames() {
        assert_eq!(
            ai_provider_referrer_from_source("ChatGPT"),
            Some("chatgpt.com")
        );
        assert_eq!(
            ai_provider_referrer_from_source("perplexity"),
            Some("perplexity.ai")
        );
        assert_eq!(ai_provider_referrer_from_source("newsletter"), None);
    }

    #[test]
    fn classifies_social_from_referrer_hostname() {
        assert_eq!(
            classify_referrer_type(Some("linkedin.com/feed/update"), None),
            Some(ReferrerType::Social)
        );
        assert!(referrer_is_social("www.youtube.com/watch?v=abc"));
        assert_eq!(
            classify_referrer_type(Some("news.ycombinator.com/item"), None),
            None
        );
    }

    #[test]
    fn search_beats_campaign_params_on_same_visit() {
        assert_eq!(
            classify_referrer_type(
                Some("google.com/search"),
                Some(&params_map(json!({"campaign": "brand"}))),
            ),
            Some(ReferrerType::Search)
        );
    }

    #[test]
    fn classifies_email_from_medium() {
        assert_eq!(
            classify_referrer_type(
                None,
                Some(&params_map(json!({"medium": "email", "source": "other"}))),
            ),
            Some(ReferrerType::Email)
        );
    }

    #[test]
    fn classifies_campaign_from_campaign_or_ref() {
        assert_eq!(
            classify_params_from_map(&params_map(json!({"campaign": "brand"}))),
            Some(ReferrerType::Campaign)
        );
        assert_eq!(
            classify_params_from_map(&params_map(json!({"ref": "partner1a2b3c"}))),
            Some(ReferrerType::Affiliate)
        );
        assert_eq!(
            classify_params_from_map(&params_map(json!({"ref": "acme"}))),
            None
        );
    }

    #[test]
    fn returns_none_for_unrecognized_params() {
        assert_eq!(
            classify_referrer_type(None, Some(&params_map(json!({"source": "billboard"}))),),
            None
        );
    }

    #[test]
    fn from_stored_parses_stored_values() {
        assert_eq!(
            ReferrerType::from_stored("search"),
            Some(ReferrerType::Search)
        );
        assert_eq!(
            ReferrerType::from_stored("campaign"),
            Some(ReferrerType::Campaign)
        );
        assert_eq!(ReferrerType::from_stored("other"), None);
    }
}
