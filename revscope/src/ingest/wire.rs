use compact_str::CompactString;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::util::parse_visitor_timestamp_id_hex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HitKind {
    Pageview = 0,
    Append = 1,
    Event = 2,
    Error = 3,
    Metadata = 4,
    Forget = 5,
    Context = 6,
    Crawler = 7,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum RequestType {
    Pageview,
    Engagement,
    Event,
    Error,
    Metadata,
    Forget,
    /// Terminal-device context (timezone, language, screen, touch) the tracker
    /// only reads once the server has confirmed it may.
    Context,
    Crawler,
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub(super) enum JsonField {
    Json(Value),
    Text(String),
}

impl JsonField {
    pub(super) fn to_normalized_json(&self) -> Option<String> {
        match self {
            // Form-urlencoded always yields strings. Untagged deserialization may
            // pick `Json(Value::String(...))` before `Text`, so re-parse that
            // inner JSON string the same way we handle `Text`.
            Self::Json(Value::String(value)) => normalized_json(value),
            Self::Json(value) => serde_json::to_string(value).ok(),
            Self::Text(value) => normalized_json(value),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct TrackRequest {
    #[serde(rename = "ty")]
    pub(super) hit_type: Option<RequestType>,
    #[serde(rename = "t")]
    pub(super) time: Option<i64>,
    /// Client clock reading from when the request was sent. Compared against
    /// `time` (same clock) to recover how stale the hit is.
    #[serde(rename = "st", alias = "send_time")]
    pub(super) send_time: Option<i64>,
    #[serde(rename = "v", alias = "version")]
    pub(super) version: Option<f64>,
    #[serde(rename = "h", alias = "hostname")]
    pub(super) hostname: Option<CompactString>,
    #[serde(rename = "tz", alias = "timezone")]
    pub(super) timezone: Option<CompactString>,
    #[serde(rename = "l", alias = "language")]
    pub(super) language: Option<CompactString>,
    #[serde(rename = "m", alias = "mobile")]
    pub(super) mobile: Option<bool>,
    #[serde(rename = "tch", alias = "touch")]
    pub(super) touch: Option<bool>,
    pub(super) brave: Option<bool>,
    pub(super) duck: Option<bool>,
    #[serde(rename = "pg", alias = "page_id")]
    pub(super) page_id: Option<CompactString>,
    #[serde(rename = "p", alias = "path")]
    pub(super) path: Option<CompactString>,
    #[serde(rename = "r", alias = "referrer")]
    pub(super) referrer: Option<CompactString>,
    #[serde(rename = "vw", alias = "viewport_width")]
    pub(super) viewport_width: Option<i64>,
    #[serde(rename = "vh", alias = "viewport_height")]
    pub(super) viewport_height: Option<i64>,
    #[serde(rename = "sw", alias = "screen_width")]
    pub(super) screen_width: Option<i64>,
    #[serde(rename = "sh", alias = "screen_height")]
    pub(super) screen_height: Option<i64>,
    #[serde(
        rename = "s",
        alias = "session",
        default,
        deserialize_with = "deserialize_session_id"
    )]
    pub(super) session_id: Option<i64>,
    #[serde(rename = "c", alias = "consent", default)]
    pub(super) consent: Option<bool>,
    #[serde(rename = "meta", alias = "metadata")]
    pub(super) metadata: Option<JsonField>,
    #[serde(rename = "ev", alias = "event")]
    pub(super) event: Option<CompactString>,
    #[serde(rename = "e", alias = "error")]
    pub(super) error: Option<String>,
    #[serde(rename = "d", alias = "duration")]
    pub(super) duration: Option<i64>,
    #[serde(rename = "sc", alias = "scrolled")]
    pub(super) scrolled: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub(super) struct TrackBatchRequest {
    #[serde(rename = "st", alias = "send_time")]
    pub(super) send_time: Option<i64>,
    #[serde(rename = "v", alias = "version")]
    pub(super) version: Option<f64>,
    #[serde(rename = "h", alias = "hostname")]
    pub(super) hostname: Option<CompactString>,
    #[serde(
        rename = "s",
        alias = "session",
        default,
        deserialize_with = "deserialize_session_id"
    )]
    pub(super) session_id: Option<i64>,
    #[serde(rename = "c", alias = "consent", default)]
    pub(super) consent: Option<bool>,
    pub(super) items: Option<String>,
}

fn deserialize_session_id<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer)?
        .map(|value| parse_visitor_timestamp_id_hex(&value).map_err(serde::de::Error::custom))
        .transpose()
}

fn normalized_json(value: &str) -> Option<String> {
    let parsed: Value = serde_json::from_str(value).ok()?;
    serde_json::to_string(&parsed).ok()
}

pub(super) fn parse_hit_kind(value: Option<RequestType>) -> Option<HitKind> {
    match value {
        Some(RequestType::Pageview) => Some(HitKind::Pageview),
        Some(RequestType::Engagement) => Some(HitKind::Append),
        Some(RequestType::Event) => Some(HitKind::Event),
        Some(RequestType::Error) => Some(HitKind::Error),
        Some(RequestType::Metadata) => Some(HitKind::Metadata),
        Some(RequestType::Forget) => Some(HitKind::Forget),
        Some(RequestType::Context) => Some(HitKind::Context),
        Some(RequestType::Crawler) => Some(HitKind::Crawler),
        Some(RequestType::Unknown) | None => None,
    }
}
