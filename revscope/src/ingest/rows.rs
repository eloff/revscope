use std::net::IpAddr;

use compact_str::CompactString;
use serde::{Serialize, Serializer};

use super::attribution::LandingCampaignTuple;
use super::user_agent::UserAgentContext;
use crate::db::ClientPageId;
use crate::referrer::ReferrerType;
use crate::util::encode_lower_hex;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(super) struct ScreenDimensions {
    pub(super) screen_width: i64,
    pub(super) screen_height: i64,
    pub(super) viewport_width: i64,
    pub(super) viewport_height: i64,
}

/// A queued pageview inserted into the `pageviews` table on flush.
#[derive(Serialize)]
pub(crate) struct PageView {
    pub(crate) batch_order: usize,
    pub(crate) timestamp_id: i64,
    pub(crate) visitor_timestamp_id: i64,
    pub(crate) path: CompactString,
    pub(crate) hostname: CompactString,
    pub(crate) metadata_json: Option<String>,
    pub(crate) duration_seconds: Option<i64>,
    pub(crate) scrolled_percent: Option<i64>,
    #[serde(serialize_with = "serialize_client_page_id")]
    pub(crate) client_page_id: ClientPageId,
    #[serde(serialize_with = "serialize_user_hash")]
    pub(crate) user_hash: u128,
    pub(crate) expires_after_days: i64,
}

/// Attribution and device fields written to `landings` for a session's first
/// pageview. Stored separately from the shared `PageView` row; `timestamp_id`
/// matches that pageview's server id (and `visitor_timestamp_id` for the session).
#[derive(Serialize)]
pub(super) struct LandingPageView {
    pub(super) timestamp_id: i64,
    pub(super) consent: Option<bool>,
    pub(super) external_referrer: Option<CompactString>,
    pub(super) is_google: bool,
    pub(super) referrer_type: Option<ReferrerType>,
    pub(super) landing_campaign: Option<LandingCampaignTuple>,
    pub(super) landing_click_id: Option<String>,
    pub(super) ua_context: UserAgentContext,
    pub(super) screen_dimensions: Option<ScreenDimensions>,
    pub(super) timezone: Option<CompactString>,
    pub(super) language: Option<CompactString>,
    pub(super) country: Option<String>,
    pub(super) region: Option<String>,
    pub(super) city: Option<String>,
    pub(super) tracker_version: f64,
}

/// Terminal-device context for a session, sent by the tracker only after the
/// server confirmed it may read it. Applied to the session's existing `landings`
/// row (which may have been written by an earlier flush).
#[derive(Serialize)]
pub(super) struct LandingContextRow {
    pub(super) visitor_timestamp_id: i64,
    pub(super) timezone: Option<CompactString>,
    pub(super) language: Option<CompactString>,
    pub(super) screen_dimensions: Option<ScreenDimensions>,
    /// Only ever upgrades `desktop` to `desktop_touch`; a mobile or tablet
    /// classification from the user agent always wins, as it does in
    /// `user_agent::resolve_device_class`.
    pub(super) touch: bool,
}

/// Index into `BatchData::pageviews`.
pub(super) type PageviewRef = u32;

/// An event queued for the next flush.
#[derive(Serialize)]
pub(super) struct EventRow {
    pub(super) timestamp_id: i64,
    pub(super) event_name: CompactString,
    pub(super) page_timestamp_id: i64,
    pub(super) metadata: Option<String>,
}

/// An event whose `pg` matched no pageview when it arrived. It is attached to
/// that pageview if the pageview is enqueued before the next flush, and dropped
/// at flush otherwise.
#[derive(Serialize)]
pub(super) struct OrphanEvent {
    #[serde(serialize_with = "serialize_client_page_id")]
    pub(super) client_page_id: ClientPageId,
    pub(super) event_name: CompactString,
    pub(super) metadata: Option<String>,
    /// Server receive time and client queue age, so the event id is backdated
    /// from when the event arrived rather than from when it was attached.
    pub(super) received_nanos: i64,
    pub(super) client_age_ms: i64,
    /// The hit as received, stored on the `errors` row if the event is dropped.
    #[serde(skip)]
    pub(super) wire_json: String,
    /// Charged against the error report limit only if the event is dropped,
    /// since most orphans are adopted once their pageview arrives.
    #[serde(skip)]
    pub(super) client_ip: Option<IpAddr>,
}

/// An engagement hit queued for the next flush.
#[derive(Serialize)]
pub(super) struct EngagementRow {
    pub(super) page_timestamp_id: i64,
    pub(super) duration_increment: i64,
    pub(super) scrolled_percent: Option<i64>,
}

/// A page metadata update queued for the next flush.
#[derive(Serialize)]
pub(super) struct MetadataUpdateRow {
    pub(super) batch_order: usize,
    pub(super) page_timestamp_id: i64,
    pub(super) visitor_timestamp_id: i64,
    pub(super) metadata_json: String,
}

/// Identity of a buffered crawler view before flush collapses it by path_id.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct CrawlerViewKey {
    pub(super) hour: i64,
    pub(super) useragent: CompactString,
    /// Client IP for unverified crawlers; `None` when verified or unknown.
    pub(super) ip: Option<CompactString>,
    pub(super) hostname: CompactString,
    pub(super) path: CompactString,
    pub(super) verified: bool,
}

/// A visitor-erasure request resolved after all other rows in the batch have
/// been written and identity merges have completed.
#[derive(Serialize)]
pub(super) struct ForgetRow {
    #[serde(serialize_with = "serialize_user_hash")]
    pub(super) user_hash: u128,
    #[serde(serialize_with = "serialize_optional_client_page_id")]
    pub(super) client_page_id: Option<ClientPageId>,
    pub(super) session_id: Option<i64>,
}

/// Client page ids are 12-byte blobs; hex is how the tracker and the dashboard
/// refer to them.
fn serialize_client_page_id<S>(page_id: &ClientPageId, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&encode_lower_hex(page_id.as_slice()))
}

fn serialize_optional_client_page_id<S>(
    page_id: &Option<ClientPageId>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match page_id {
        Some(page_id) => serializer.serialize_some(&encode_lower_hex(page_id.as_slice())),
        None => serializer.serialize_none(),
    }
}

/// Hex, matching the big-endian blob stored in `user_sessions.user_hash` (and
/// avoiding a 128-bit JSON number).
fn serialize_user_hash<S>(user_hash: &u128, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&encode_lower_hex(&user_hash.to_be_bytes()))
}
