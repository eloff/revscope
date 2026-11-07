use std::net::IpAddr;

use compact_str::CompactString;
use serde_json::Value;

use super::attribution::{
    click_id_network_from_map, infer_referrer_from_query_source,
    normalize_landing_attribution_fields, parse_landing_metadata_map, prepare_landing_attribution,
    split_referrer,
};
use super::batch::BatchData;
use super::constraints::{validate_landing_constraints, validate_pageview_constraints};
use super::enqueue::{PersistOutcome, adopt_orphan_events, record_crawler_view};
use super::error::PersistError;
use super::failure::{IngestRowFailure, constraint_failure};
use super::lookup::{resolve_enqueue_pageview_ref, resolve_enqueue_visitor_timestamp};
use super::map::IngestBatch;
use super::normalize::{
    normalized_duration_seconds, normalized_screen_dimensions, required_non_empty, resolve_language,
};
use super::rows::{LandingPageView, PageView};
use super::session::{
    build_user_hash, is_storage_allowed, resolve_session_expires_after_days,
    session_cookie_for_payload,
};
use super::timestamp::assign_pageview_timestamp;
use super::user_agent::parse_user_agent_context;
use super::wire::{JsonField, TrackRequest};
use crate::db::{ClientPageId, ProjectConnection};
use crate::geoip;
use crate::http::ParsedRequest;
use crate::referrer::{ReferrerType, classify_referrer_type, referrer_is_google};
use crate::server_context::ServerContext;

#[derive(Debug)]
struct PageviewRecord {
    /// Per-page id from the tracker (`pg`, 12 bytes).
    client_page_id: ClientPageId,
    path: CompactString,
    external_referrer: Option<CompactString>,
    hostname: CompactString,
    metadata: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn enqueue_pageview(
    server_context: &ServerContext,
    project_id: &str,
    batch: &mut IngestBatch,
    read_conn: &mut Option<ProjectConnection>,
    parsed: &ParsedRequest<'_>,
    client_ip: Option<IpAddr>,
    cf_country: Option<&str>,
    wire: &mut TrackRequest,
    client_age_ms: i64,
    outcome: &mut PersistOutcome,
    constraint_failures: &mut Vec<IngestRowFailure>,
) -> Result<(), PersistError> {
    let record = parse_pageview_record(wire)?;
    let ua_context = parse_user_agent_context(parsed.user_agent, wire);
    if ua_context.is_bot {
        if let Some(useragent) = ua_context.useragent.as_deref() {
            record_crawler_view(
                batch,
                client_age_ms,
                useragent,
                &record.hostname,
                &record.path,
                client_ip,
            );
        }
        return Ok(());
    }

    // Resolved for every pageview: the tracker refreshes the session cookie from
    // `storage_allowed`, including cookieless returns matched only by user hash.
    let geo = geoip::resolve(client_ip, cf_country);
    let storage_allowed =
        is_storage_allowed(wire.consent, geo.country.as_deref(), geo.region.as_deref());

    // Same `pg` already queued or flushed: reuse that page/session instead of
    // minting a second timestamp that would fail UNIQUE on client_page_ids_map.
    if let Some((_, visitor_timestamp_id)) =
        resolve_enqueue_pageview_ref(server_context, project_id, batch, read_conn, wire)?
    {
        outcome.visitor_timestamp_id = Some(visitor_timestamp_id);
        outcome.storage_allowed = storage_allowed;
        outcome.session_cookie =
            session_cookie_for_payload(wire.consent, storage_allowed, visitor_timestamp_id);
        return Ok(());
    }

    let client_ip_text = client_ip.map(|ip| ip.to_string());
    let user_hash = build_user_hash(parsed.user_agent, client_ip_text.as_deref());

    let timestamp_id =
        assign_pageview_timestamp(server_context, project_id, batch, read_conn, client_age_ms)?;
    let visitor_timestamp_id = resolve_enqueue_visitor_timestamp(
        server_context,
        project_id,
        batch,
        read_conn,
        wire,
        user_hash,
        timestamp_id,
    )?;
    let is_landing = visitor_timestamp_id == timestamp_id;

    let mut page = PageView {
        batch_order: batch.total_events,
        timestamp_id,
        visitor_timestamp_id,
        path: record.path,
        hostname: record.hostname,
        metadata_json: record.metadata.clone(),
        duration_seconds: normalized_duration_seconds(wire.duration),
        scrolled_percent: wire.scrolled.map(|value| value.clamp(0, 100)),
        client_page_id: record.client_page_id,
        user_hash,
        expires_after_days: resolve_session_expires_after_days(wire),
    };

    let mut landing = None;
    if is_landing {
        // Landing attribution is parsed here; the campaigns upsert happens in flush with other writes.
        // We need access to specific keys for classification, and we remove utm/marketing params later,
        // so we need this as a map object.
        let mut landing_metadata_map = parse_landing_metadata_map(page.metadata_json.as_deref());
        if let Some(map) = landing_metadata_map.as_mut() {
            normalize_landing_attribution_fields(map);
        }
        let (referrer_type, is_google) = classify_landing_referrer(
            record.external_referrer.as_deref(),
            landing_metadata_map.as_ref(),
        );
        let (landing_campaign, landing_click_id) = match landing_metadata_map.as_mut() {
            Some(map) => {
                let (tuple, remaining) = prepare_landing_attribution(map);
                page.metadata_json = remaining;
                let click_id = tuple.as_ref().and_then(|tuple| tuple.click_id.clone());
                (tuple, click_id)
            }
            None => (None, None),
        };

        landing = Some(LandingPageView {
            timestamp_id,
            consent: wire.consent,
            external_referrer: record.external_referrer,
            is_google,
            referrer_type,
            landing_campaign,
            landing_click_id,
            ua_context,
            screen_dimensions: normalized_screen_dimensions(wire),
            timezone: wire.timezone.clone(),
            language: resolve_language(wire, parsed),
            country: geo.country.map(|value| value.to_ascii_uppercase()),
            region: geo.region.map(|value| value.to_ascii_uppercase()),
            city: geo.city,
            tracker_version: wire.version.unwrap_or(0.0),
        });
    }

    if let Some(landing_row) = landing.as_ref()
        && let Err(error_text) = validate_landing_constraints(landing_row)
    {
        constraint_failures.push(constraint_failure(wire, error_text));
        return Ok(());
    }
    if let Err(error_text) = validate_pageview_constraints(&page) {
        constraint_failures.push(constraint_failure(wire, error_text));
        return Ok(());
    }

    let client_page_id = page.client_page_id.clone();
    let data: &mut BatchData = &mut *batch;
    let index = data.pageviews.len() as u32;
    if let Some(landing) = landing {
        data.landings.push(landing);
    }
    data.pageview_idx_by_client_id
        .insert(client_page_id.clone(), index);
    data.pageview_idx_by_ts.insert(timestamp_id, index);
    data.pageviews.push(page);

    data.visitor_by_user_hash
        .insert(user_hash, visitor_timestamp_id);
    data.total_events += 1;

    adopt_orphan_events(
        server_context,
        project_id,
        batch,
        read_conn,
        &client_page_id,
        timestamp_id,
    )?;

    outcome.visitor_timestamp_id = Some(visitor_timestamp_id);
    outcome.storage_allowed = storage_allowed;
    outcome.session_cookie =
        session_cookie_for_payload(wire.consent, storage_allowed, visitor_timestamp_id);
    Ok(())
}

/// The landing's referrer type, and whether it is organic Google Search. A
/// click id marks the visit as a campaign whatever the referrer, so paid Google
/// traffic never sets `is_google` (CHECK: is_google => referrer_type = search).
fn classify_landing_referrer(
    external_referrer: Option<&str>,
    landing_metadata: Option<&serde_json::Map<String, Value>>,
) -> (Option<ReferrerType>, bool) {
    let referrer_type = if landing_metadata
        .and_then(click_id_network_from_map)
        .is_some()
    {
        Some(ReferrerType::Campaign)
    } else {
        classify_referrer_type(external_referrer, landing_metadata)
    };
    let is_google = referrer_type == Some(ReferrerType::Search)
        && external_referrer.is_some_and(referrer_is_google);
    (referrer_type, is_google)
}

fn parse_pageview_record(payload: &TrackRequest) -> Result<PageviewRecord, PersistError> {
    let path = CompactString::new(required_non_empty(payload.path.as_deref())?);
    let client_page_id = ClientPageId::from_hex(required_non_empty(payload.page_id.as_deref())?)
        .map_err(|_| PersistError::InvalidPayload)?;
    let hostname = CompactString::new(required_non_empty(payload.hostname.as_deref())?);
    let external_referrer = split_referrer(payload.referrer.as_deref(), &hostname)
        .map(CompactString::new)
        .or_else(|| {
            let metadata = payload
                .metadata
                .as_ref()
                .and_then(JsonField::to_normalized_json);
            infer_referrer_from_query_source(metadata.as_deref(), &hostname).map(CompactString::new)
        });
    let metadata = payload
        .metadata
        .as_ref()
        .and_then(JsonField::to_normalized_json);

    Ok(PageviewRecord {
        client_page_id,
        path,
        external_referrer,
        hostname,
        metadata,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::attribution::ClickIdNetwork;
    use crate::ingest::test_support::{sample_pageview, sample_parsed_request, sample_payload};

    #[test]
    fn parse_pageview_record_extracts_fields_for_persistence() {
        let payload = sample_payload();
        let parsed = parse_pageview_record(&payload).expect("pageview should parse");
        assert_eq!(parsed.path, "/pricing");
        assert_eq!(parsed.metadata.as_deref(), Some("{\"a\":1}"));
        assert_eq!(parsed.client_page_id.as_slice().len(), 12);
        assert_eq!(parsed.external_referrer, None);
    }

    #[test]
    fn parse_pageview_record_removes_internal_referrer() {
        let mut payload = sample_payload();
        payload.referrer = Some(CompactString::new("example.com/docs"));
        let parsed = parse_pageview_record(&payload).expect("pageview should parse");
        assert_eq!(parsed.external_referrer, None);
    }

    #[test]
    fn parse_pageview_record_treats_loopback_referrer_as_direct() {
        for referrer in [
            "localhost",
            "localhost:3445",
            "http://localhost/landing.html",
            "127.0.0.1",
            "http://127.0.0.1:3447/",
            "https://127.0.0.1/foo",
        ] {
            let mut payload = sample_payload();
            payload.referrer = Some(CompactString::new(referrer));
            let parsed = parse_pageview_record(&payload).expect("pageview should parse");
            assert_eq!(parsed.external_referrer, None, "{referrer}");
        }
    }

    #[test]
    fn parse_pageview_record_keeps_external_referrer() {
        let mut payload = sample_payload();
        payload.referrer = Some(CompactString::new("google.com/search"));
        let parsed = parse_pageview_record(&payload).expect("pageview should parse");
        assert_eq!(parsed.external_referrer.as_deref(), Some("google.com"));
    }

    #[test]
    fn parse_pageview_record_normalizes_full_referrer_url() {
        let mut payload = sample_payload();
        payload.referrer = Some(CompactString::new("https://www.google.com/search?q=foo"));
        let parsed = parse_pageview_record(&payload).expect("pageview should parse");
        assert_eq!(parsed.external_referrer.as_deref(), Some("google.com"));
    }

    #[test]
    fn parse_pageview_record_uses_hostname_source_when_referrer_missing() {
        let mut payload = sample_payload();
        payload.referrer = None;
        payload.metadata = Some(JsonField::Text(
            r#"{"source":"https://www.perplexity.ai/search?q=foo"}"#.to_string(),
        ));
        let parsed = parse_pageview_record(&payload).expect("pageview should parse");
        assert_eq!(parsed.external_referrer.as_deref(), Some("perplexity.ai"));
    }

    #[test]
    fn parse_pageview_record_uses_known_ai_source_when_referrer_missing() {
        let mut payload = sample_payload();
        payload.referrer = None;
        payload.metadata = Some(JsonField::Text(r#"{"source":"chatgpt"}"#.to_string()));
        let parsed = parse_pageview_record(&payload).expect("pageview should parse");
        assert_eq!(parsed.external_referrer.as_deref(), Some("chatgpt.com"));
    }

    #[test]
    fn parse_pageview_record_keeps_explicit_referrer_over_source_fallback() {
        let mut payload = sample_payload();
        payload.referrer = Some(CompactString::new("google.com/search"));
        payload.metadata = Some(JsonField::Text(r#"{"source":"chatgpt"}"#.to_string()));
        let parsed = parse_pageview_record(&payload).expect("pageview should parse");
        assert_eq!(parsed.external_referrer.as_deref(), Some("google.com"));
    }

    #[test]
    fn landing_referrer_type_uses_campaign_when_click_id_present() {
        let params = serde_json::from_str::<Value>(r#"{"gclid":"abc","source":"google"}"#)
            .expect("json")
            .as_object()
            .cloned()
            .expect("object");
        assert_eq!(
            click_id_network_from_map(&params),
            Some(ClickIdNetwork::Google)
        );
        assert_eq!(
            classify_landing_referrer(Some("google.com/search"), Some(&params)),
            (Some(ReferrerType::Campaign), false)
        );
        assert_eq!(
            classify_landing_referrer(Some("google.com/search"), None),
            (Some(ReferrerType::Search), true)
        );
    }

    #[test]
    fn parse_pageview_record_requires_non_empty_path() {
        let mut payload = sample_payload();
        payload.path = Some(CompactString::new(""));
        assert!(matches!(
            parse_pageview_record(&payload),
            Err(PersistError::InvalidPayload)
        ));
    }

    #[test]
    fn enqueue_reuses_a_client_page_id_already_in_the_batch() {
        let server_context = crate::server_context::ServerContext::new_in_memory("reuse-pg-batch");
        let mut batch = IngestBatch::new(0, 0);
        let mut existing = sample_pageview(1_000, 1_000, 3);
        existing.client_page_id =
            ClientPageId::from_hex("00112233445566778899aabb").expect("page id");
        {
            let data: &mut BatchData = &mut batch;
            data.pageview_idx_by_client_id
                .insert(existing.client_page_id.clone(), 0);
            data.pageviews.push(existing);
            data.total_events = 1;
        }

        let parsed = sample_parsed_request();
        let mut wire = sample_payload();
        let mut outcome = PersistOutcome::default();
        let mut constraint_failures = Vec::new();
        let mut read_conn = None;
        enqueue_pageview(
            &server_context,
            "1",
            &mut batch,
            &mut read_conn,
            &parsed,
            None,
            None,
            &mut wire,
            0,
            &mut outcome,
            &mut constraint_failures,
        )
        .expect("enqueue");

        assert!(constraint_failures.is_empty());
        assert_eq!(batch.pageviews.len(), 1);
        assert_eq!(batch.landings.len(), 0);
        assert_eq!(batch.total_events, 1);
        assert_eq!(outcome.visitor_timestamp_id, Some(1_000));
    }
}
