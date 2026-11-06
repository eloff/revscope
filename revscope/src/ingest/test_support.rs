//! Fixtures shared by the ingest submodules' tests.

use compact_str::CompactString;

use super::rows::{LandingPageView, OrphanEvent, PageView};
use super::user_agent::parse_user_agent_context;
use super::wire::{JsonField, TrackRequest};
use crate::db::ClientPageId;
use crate::http::ParsedRequest;

pub(super) fn sample_orphan_event(received_nanos: i64) -> OrphanEvent {
    OrphanEvent {
        client_page_id: ClientPageId::from_hex("00112233445566778899aabb").expect("page id"),
        event_name: CompactString::new("signup"),
        metadata: None,
        received_nanos,
        client_age_ms: 0,
        wire_json: format!(r#"{{"received":{received_nanos}}}"#),
        client_ip: None,
    }
}

pub(super) fn sample_parsed_request() -> ParsedRequest<'static> {
    ParsedRequest {
        method: "POST",
        target: "/demo",
        head: "POST /demo HTTP/1.1\r\nUser-Agent: Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/123.0.0.0 Safari/537.36",
        cookie: None,
        authorization: None,
        origin: None,
        host: None,
        access_control_request_headers: None,
        accept_encoding: None,
        user_agent: Some(
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/123.0.0.0 Safari/537.36",
        ),
        accept_language: None,
        range: None,
        if_range: None,
        client_ip: None,
        x_forwarded_for_last: None,
        cf_ipcountry: None,
        webhook_id: None,
        webhook_timestamp: None,
        webhook_signature: None,
        stripe_signature: None,
        connection_close: false,
    }
}

pub(super) fn sample_landing_pageview(timestamp_id: i64) -> LandingPageView {
    LandingPageView {
        timestamp_id,
        consent: None,
        external_referrer: None,
        is_google: false,
        referrer_type: None,
        landing_campaign: None,
        landing_click_id: None,
        ua_context: parse_user_agent_context(None, &sample_payload()),
        screen_dimensions: None,
        timezone: None,
        language: None,
        country: None,
        region: None,
        city: None,
        tracker_version: 0.0,
    }
}

pub(super) fn sample_payload() -> TrackRequest {
    TrackRequest {
        hit_type: None,
        time: None,
        send_time: None,
        version: None,
        hostname: Some(CompactString::new("example.com")),
        timezone: None,
        language: None,
        mobile: Some(false),
        touch: None,
        brave: None,
        duck: None,
        page_id: Some(CompactString::new("00112233445566778899aabb")),
        path: Some(CompactString::new("/pricing")),
        referrer: Some(CompactString::new("example.com/blog")),
        viewport_width: Some(1200),
        viewport_height: Some(800),
        screen_width: Some(1920),
        screen_height: Some(1080),
        session_id: None,
        consent: Some(false),
        metadata: Some(JsonField::Json(serde_json::json!({"a":1}))),
        event: None,
        error: None,
        duration: None,
        scrolled: None,
    }
}

/// A pageview of `/pricing` with a client page id derived from `timestamp_id`.
pub(super) fn sample_pageview(
    timestamp_id: i64,
    visitor_timestamp_id: i64,
    duration_seconds: i64,
) -> PageView {
    let mut page_id_bytes = [0_u8; 12];
    page_id_bytes[..8].copy_from_slice(&timestamp_id.to_be_bytes());
    PageView {
        batch_order: 0,
        timestamp_id,
        visitor_timestamp_id,
        path: CompactString::new("/pricing"),
        hostname: CompactString::new("example.com"),
        metadata_json: None,
        duration_seconds: Some(duration_seconds),
        scrolled_percent: None,
        client_page_id: ClientPageId::from_bytes(page_id_bytes).expect("page id"),
        user_hash: visitor_timestamp_id as u128,
        expires_after_days: 365,
    }
}
