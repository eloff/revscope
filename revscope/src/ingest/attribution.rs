use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;
use serde_json::Value;

use crate::util::non_empty;

static CLICK_ID_QUERY_PARAM_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(gclid|gbraid|wbraid|dclid|gclsrc|fbclid|msclkid|li_fat_id|rdt_cid|ttclid|twclid|sccid|epik|tblci|ob_click_id|obclid|dicbo|vmcid|yclid|click_id|clickid|cid|subid|sub_id|aff_click_id)$")
        .expect("click id query param regex must compile")
});

static SOURCE_HOSTNAME_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:https?://)?(?:(?:m|l|w{2,3}(?:[0-9]+)?)\.)?((?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,6})(?:[/?#].*)?$")
        .expect("source hostname regex must compile")
});

/// Attribution fields split out of the landing query params and stored in the
/// `campaigns` table. Empty strings stand in for missing UTM keys.
#[derive(Clone, Debug, Default, Serialize)]
pub struct LandingCampaignTuple {
    pub source: String,
    pub medium: String,
    pub campaign: String,
    pub click_id: Option<String>,
    pub network: Option<String>,
}

impl LandingCampaignTuple {
    /// When `campaign` is missing but `network` is set (e.g. from a click id param),
    /// use the network label as the campaign name while keeping `network` populated.
    pub fn promote_network_to_campaign(&mut self) {
        if self.campaign.is_empty()
            && let Some(network) = self.network.as_ref()
        {
            self.campaign = network.clone();
        }
    }

    pub fn has_campaign(&self) -> bool {
        !self.campaign.is_empty()
    }
}

/// Ad network inferred from a click-id query parameter name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClickIdNetwork {
    Google,
    Meta,
    Microsoft,
    Linkedin,
    Reddit,
    Tiktok,
    X,
    Snapchat,
    Pinterest,
    Taboola,
    Outbrain,
    Yahoo,
    Yandex,
    Generic,
}

impl ClickIdNetwork {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Google => "google",
            Self::Meta => "meta",
            Self::Microsoft => "microsoft",
            Self::Linkedin => "linkedin",
            Self::Reddit => "reddit",
            Self::Tiktok => "tiktok",
            Self::X => "x",
            Self::Snapchat => "snapchat",
            Self::Pinterest => "pinterest",
            Self::Taboola => "taboola",
            Self::Outbrain => "outbrain",
            Self::Yahoo => "yahoo",
            Self::Yandex => "yandex",
            Self::Generic => "generic",
        }
    }
}

impl std::fmt::Display for ClickIdNetwork {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

pub(super) fn parse_landing_metadata_map(
    metadata: Option<&str>,
) -> Option<serde_json::Map<String, Value>> {
    let metadata = non_empty(metadata)?;
    let parsed: Value = serde_json::from_str(metadata).ok()?;
    parsed.as_object().cloned()
}

pub fn merge_query_into_metadata(query: Option<&str>, metadata: Option<String>) -> Option<String> {
    let mut merged = serde_json::Map::new();
    if let Some(query) = non_empty(query)
        && let Ok(Value::Object(obj)) = serde_json::from_str(query)
    {
        merged.extend(obj);
    }
    if let Some(metadata) = metadata
        && let Ok(Value::Object(obj)) = serde_json::from_str(&metadata)
    {
        for (key, value) in obj {
            merged.insert(key, value);
        }
    }
    if merged.is_empty() {
        None
    } else {
        serde_json::to_string(&Value::Object(merged)).ok()
    }
}

/// Splits decoded landing query params into a campaign tuple and the remaining
/// JSON string (everything except fields stored on `campaigns`).
pub fn split_landing_query_params(
    obj: &mut serde_json::Map<String, Value>,
) -> LandingCampaignTuple {
    let source = take_string_field(obj, "source");
    let medium = take_string_field(obj, "medium");
    let campaign = take_string_field(obj, "campaign");
    let (click_id, network) = take_click_id_field(obj);
    LandingCampaignTuple {
        source,
        medium,
        campaign,
        click_id,
        network,
    }
}

/// Splits the decoded landing query JSON into a campaign tuple and the
/// remaining JSON string (everything except fields stored on `campaigns`).
/// Returns `None` if the input is not a JSON object.
pub fn split_landing_query_params_from_json(
    decoded_query: &str,
) -> Option<(LandingCampaignTuple, Option<String>)> {
    let parsed: Value = serde_json::from_str(decoded_query).ok()?;
    let Value::Object(mut obj) = parsed else {
        return None;
    };
    let tuple = split_landing_query_params(&mut obj);
    let remaining = if obj.is_empty() {
        None
    } else {
        serde_json::to_string(&Value::Object(obj)).ok()
    };
    Some((tuple, remaining))
}

fn normalize_attribution_value(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

/// Trim and lowercase `source` / `medium` / `campaign` on the landing metadata
/// map so classification and the campaigns upsert see the same normalized keys.
pub(super) fn normalize_landing_attribution_fields(obj: &mut serde_json::Map<String, Value>) {
    for key in ["source", "medium", "campaign"] {
        let normalized = match obj.get(key) {
            Some(Value::String(value)) => normalize_attribution_value(value),
            _ => continue,
        };
        if normalized.is_empty() {
            obj.remove(key);
        } else {
            obj.insert(key.to_string(), Value::String(normalized));
        }
    }
}

fn take_string_field(obj: &mut serde_json::Map<String, Value>, key: &str) -> String {
    if let Some(Value::String(_)) = obj.get(key)
        && let Some(Value::String(value)) = obj.remove(key)
    {
        return normalize_attribution_value(&value);
    }
    String::new()
}

pub fn click_id_network_from_map(obj: &serde_json::Map<String, Value>) -> Option<ClickIdNetwork> {
    let key = obj
        .keys()
        .find(|key| CLICK_ID_QUERY_PARAM_PATTERN.is_match(key))?;
    let Value::String(value) = obj.get(key)? else {
        return None;
    };
    if value.is_empty() {
        return None;
    }
    Some(click_id_network(key))
}

fn take_click_id_field(
    obj: &mut serde_json::Map<String, Value>,
) -> (Option<String>, Option<String>) {
    let Some(key) = obj
        .keys()
        .find(|key| CLICK_ID_QUERY_PARAM_PATTERN.is_match(key))
        .cloned()
    else {
        return (None, None);
    };
    let network = click_id_network(&key);
    if let Some(Value::String(value)) = obj.remove(&key)
        && !value.is_empty()
    {
        return (Some(value), Some(network.as_str().to_string()));
    }
    (None, None)
}

fn click_id_network(param_name: &str) -> ClickIdNetwork {
    match param_name.to_ascii_lowercase().as_str() {
        "gclid" | "gbraid" | "wbraid" | "dclid" | "gclsrc" => ClickIdNetwork::Google,
        "fbclid" => ClickIdNetwork::Meta,
        "msclkid" => ClickIdNetwork::Microsoft,
        "li_fat_id" => ClickIdNetwork::Linkedin,
        "rdt_cid" => ClickIdNetwork::Reddit,
        "ttclid" => ClickIdNetwork::Tiktok,
        "twclid" => ClickIdNetwork::X,
        "sccid" => ClickIdNetwork::Snapchat,
        "epik" => ClickIdNetwork::Pinterest,
        "tblci" => ClickIdNetwork::Taboola,
        "ob_click_id" | "obclid" | "dicbo" => ClickIdNetwork::Outbrain,
        "vmcid" => ClickIdNetwork::Yahoo,
        "yclid" => ClickIdNetwork::Yandex,
        _ => ClickIdNetwork::Generic,
    }
}

pub fn restore_landing_attribution_params_to_query(
    tuple: &LandingCampaignTuple,
    remaining: &mut Option<String>,
) {
    let mut obj = remaining
        .take()
        .and_then(|json| serde_json::from_str::<Value>(&json).ok())
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    if !tuple.source.is_empty() {
        obj.insert("source".to_string(), Value::String(tuple.source.clone()));
    }
    if !tuple.medium.is_empty() {
        obj.insert("medium".to_string(), Value::String(tuple.medium.clone()));
    }
    *remaining = if obj.is_empty() {
        None
    } else {
        serde_json::to_string(&Value::Object(obj)).ok()
    };
}

/// Splits landing query JSON into an optional campaigns row and remaining metadata.
/// Source/medium stay in `metadata` when there is no campaign (including after
/// promoting `network` from a click id).
pub fn prepare_landing_attribution(
    obj: &mut serde_json::Map<String, Value>,
) -> (Option<LandingCampaignTuple>, Option<String>) {
    let mut tuple = split_landing_query_params(obj);
    tuple.promote_network_to_campaign();
    if !tuple.has_campaign() {
        let mut remaining = serde_json::to_string(&Value::Object(obj.clone())).ok();
        restore_landing_attribution_params_to_query(&tuple, &mut remaining);
        return (None, remaining);
    }
    let remaining = if obj.is_empty() {
        None
    } else {
        serde_json::to_string(&Value::Object(obj.clone())).ok()
    };
    (Some(tuple), remaining)
}

pub fn prepare_landing_attribution_from_json(
    decoded_query: &str,
) -> Option<(Option<LandingCampaignTuple>, Option<String>)> {
    let parsed: Value = serde_json::from_str(decoded_query).ok()?;
    let Value::Object(mut obj) = parsed else {
        return None;
    };
    Some(prepare_landing_attribution(&mut obj))
}

pub(super) fn split_referrer(referrer: Option<&str>, hostname: &str) -> Option<String> {
    let referrer = non_empty(referrer)?;
    let normalized = crate::referrer::normalize_referrer_hostname(referrer)?;
    // 127.0.0.1 and localhost are local previews, not an external source.
    if crate::referrer::referrer_host_is_loopback(&normalized) {
        return None;
    }
    let page_host = crate::referrer::normalize_referrer_hostname(hostname)
        .unwrap_or_else(|| hostname.to_ascii_lowercase());
    if normalized == page_host {
        return None;
    }
    Some(normalized)
}

pub(super) fn infer_referrer_from_query_source(
    decoded_query: Option<&str>,
    hostname: &str,
) -> Option<String> {
    let source = query_source(decoded_query)?;
    let normalized = crate::referrer::ai_provider_referrer_from_source(&source)
        .map(str::to_string)
        .or_else(|| {
            SOURCE_HOSTNAME_PATTERN
                .captures(&source)
                .and_then(|captures| captures.get(1))
                .map(|host| host.as_str().to_ascii_lowercase())
        })?;
    let page_host = crate::referrer::normalize_referrer_hostname(hostname)
        .unwrap_or_else(|| hostname.to_ascii_lowercase());
    if normalized == page_host {
        return None;
    }
    Some(normalized)
}

fn query_source(decoded_query: Option<&str>) -> Option<String> {
    let decoded_query = non_empty(decoded_query)?;
    let parsed: Value = serde_json::from_str(decoded_query).ok()?;
    let obj = parsed.as_object()?;
    get_query_string_field(obj, "source")
        .or_else(|| get_query_string_field(obj, "utm_source"))
        .map(str::to_string)
}

fn get_query_string_field<'a>(
    obj: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Option<&'a str> {
    obj.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn split_landing_query_params_extracts_campaign_tuple_and_remainder() {
        let (tuple, remaining) = split_landing_query_params_from_json(
            r#"{"source":"google","medium":"cpc","campaign":"brand","gclid":"abc123","term":"foo"}"#,
        )
        .expect("decoded JSON object");
        assert_eq!(tuple.source, "google");
        assert_eq!(tuple.medium, "cpc");
        assert_eq!(tuple.campaign, "brand");
        assert_eq!(tuple.click_id.as_deref(), Some("abc123"));
        assert_eq!(tuple.network.as_deref(), Some("google"));
        assert_eq!(remaining.as_deref(), Some(r#"{"term":"foo"}"#));

        let (mut tuple, mut remaining) =
            split_landing_query_params_from_json(r#"{"source":"newsletter","medium":"email"}"#)
                .expect("decoded JSON object");
        assert_eq!(tuple.source, "newsletter");
        assert_eq!(tuple.medium, "email");
        assert_eq!(tuple.campaign, "");
        tuple.promote_network_to_campaign();
        assert!(!tuple.has_campaign());
        restore_landing_attribution_params_to_query(&tuple, &mut remaining);
        assert_eq!(
            remaining.and_then(|json| serde_json::from_str::<Value>(&json).ok()),
            Some(json!({"source":"newsletter","medium":"email"}))
        );

        let (mut tuple, remaining) =
            split_landing_query_params_from_json(r#"{"cid":"affiliate-123","term":"foo"}"#)
                .expect("decoded JSON object");
        assert_eq!(tuple.click_id.as_deref(), Some("affiliate-123"));
        assert_eq!(tuple.network.as_deref(), Some("generic"));
        tuple.promote_network_to_campaign();
        assert_eq!(tuple.campaign, "generic");
        assert!(tuple.has_campaign());
        assert_eq!(remaining.as_deref(), Some(r#"{"term":"foo"}"#));

        let (tuple, remaining) =
            split_landing_query_params_from_json(r#"{"term":"foo"}"#).expect("decoded JSON object");
        assert!(!tuple.has_campaign());
        assert_eq!(remaining.as_deref(), Some(r#"{"term":"foo"}"#));

        assert!(split_landing_query_params_from_json("not json").is_none());
        assert!(split_landing_query_params_from_json("[1,2,3]").is_none());
    }

    #[test]
    fn normalize_landing_attribution_fields_trims_lowercases_and_drops_blank() {
        let mut obj = serde_json::from_str::<Value>(
            r#"{"source":" Google ","medium":"  ","campaign":"Brand","term":"FOO"}"#,
        )
        .unwrap()
        .as_object()
        .cloned()
        .unwrap();
        normalize_landing_attribution_fields(&mut obj);
        assert_eq!(obj.get("source").and_then(Value::as_str), Some("google"));
        assert!(!obj.contains_key("medium"));
        assert_eq!(obj.get("campaign").and_then(Value::as_str), Some("brand"));
        assert_eq!(obj.get("term").and_then(Value::as_str), Some("FOO"));
    }

    #[test]
    fn split_landing_query_params_trims_and_lowercases_attribution() {
        let (tuple, remaining) = split_landing_query_params_from_json(
            r#"{"source":" Google ","medium":" CPC ","campaign":" Brand-Sale ","term":"FOO"}"#,
        )
        .expect("decoded JSON object");
        assert_eq!(tuple.source, "google");
        assert_eq!(tuple.medium, "cpc");
        assert_eq!(tuple.campaign, "brand-sale");
        assert_eq!(remaining.as_deref(), Some(r#"{"term":"FOO"}"#));

        let (tuple, remaining) = split_landing_query_params_from_json(
            r#"{"source":"  ","medium":"Email","campaign":"   "}"#,
        )
        .expect("decoded JSON object");
        assert_eq!(tuple.source, "");
        assert_eq!(tuple.medium, "email");
        assert_eq!(tuple.campaign, "");
        assert!(!tuple.has_campaign());
        assert!(remaining.is_none());
    }

    #[test]
    fn merge_query_into_metadata_prefers_metadata_on_conflict() {
        let merged = merge_query_into_metadata(
            Some(r#"{"ref":"ad","section":"hero"}"#),
            Some(r#"{"section":"pricing","a":1}"#.to_string()),
        );
        assert_eq!(
            merged.as_deref(),
            Some(r#"{"a":1,"ref":"ad","section":"pricing"}"#)
                .map(|value| value.to_string())
                .as_deref()
        );
        let parsed: Value = serde_json::from_str(merged.as_deref().unwrap()).unwrap();
        assert_eq!(parsed["section"], "pricing");
        assert_eq!(parsed["ref"], "ad");
        assert_eq!(parsed["a"], 1);
    }

    #[test]
    fn prepare_landing_attribution_requires_campaign_for_campaigns_table() {
        let (tuple, remaining) =
            prepare_landing_attribution_from_json(r#"{"source":"newsletter","medium":"email"}"#)
                .expect("decoded JSON object");
        assert!(tuple.is_none());
        assert_eq!(
            remaining.and_then(|json| serde_json::from_str::<Value>(&json).ok()),
            Some(json!({"source":"newsletter","medium":"email"}))
        );

        let (tuple, remaining) = prepare_landing_attribution_from_json(
            r#"{"source":"google","medium":"cpc","campaign":"brand","gclid":"abc"}"#,
        )
        .expect("decoded JSON object");
        let tuple = tuple.expect("campaign tuple");
        assert_eq!(tuple.campaign, "brand");
        assert_eq!(tuple.network.as_deref(), Some("google"));
        assert_eq!(tuple.click_id.as_deref(), Some("abc"));
        assert!(remaining.is_none());

        let (tuple, remaining) =
            prepare_landing_attribution_from_json(r#"{"gclid":"abc123","term":"foo"}"#)
                .expect("decoded JSON object");
        let tuple = tuple.expect("campaign tuple");
        assert_eq!(tuple.campaign, "google");
        assert_eq!(tuple.network.as_deref(), Some("google"));
        assert_eq!(remaining.as_deref(), Some(r#"{"term":"foo"}"#));

        let (tuple, remaining) = prepare_landing_attribution_from_json(
            r#"{"source":" Google ","medium":" CPC ","campaign":" Brand "}"#,
        )
        .expect("decoded JSON object");
        let tuple = tuple.expect("campaign tuple");
        assert_eq!(tuple.source, "google");
        assert_eq!(tuple.medium, "cpc");
        assert_eq!(tuple.campaign, "brand");
        assert!(remaining.is_none());

        let (tuple, remaining) = prepare_landing_attribution_from_json(
            r#"{"source":"Newsletter","medium":"Email","campaign":"  "}"#,
        )
        .expect("decoded JSON object");
        assert!(tuple.is_none());
        assert_eq!(
            remaining.and_then(|json| serde_json::from_str::<Value>(&json).ok()),
            Some(json!({"source":"newsletter","medium":"email"}))
        );
    }
}
