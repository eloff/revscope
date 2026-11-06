use super::error::PersistError;
use super::normalize::normalized_language;
use super::wire::{HitKind, TrackBatchRequest, TrackRequest, parse_hit_kind};
use crate::util::milliseconds_since_epoch;

const MAX_TRACK_BATCH_ITEMS: usize = 20;

/// Largest client-reported hit age we trust. Ages far beyond the tracker's
/// normal flush window are more likely a clock adjustment than a real delay.
const MAX_CLIENT_AGE_MS: i64 = 30 * 60 * 1000;

#[derive(Debug)]
pub(super) enum ParsedPayloadOutcome {
    Ignore,
    Valid(Box<ValidatedPayload>),
    Batch(ValidatedBatch),
}

#[derive(Debug)]
pub(super) struct ValidatedPayload {
    pub(super) project_name: String,
    pub(super) hit_kind: HitKind,
    #[cfg_attr(not(test), allow(dead_code))]
    created_at: i64,
    /// How long the hit sat in the tracker's queue before it was sent, used to
    /// backdate the assigned timestamp id towards when the hit happened.
    pub(super) client_age_ms: i64,
    pub(super) wire: TrackRequest,
}

#[derive(Debug)]
pub(super) struct ValidatedBatch {
    pub(super) project_name: String,
    pub(super) items: Vec<ValidatedPayload>,
}

pub(super) fn parse_and_validate_payload(
    target: &str,
    body: &[u8],
) -> Result<ParsedPayloadOutcome, PersistError> {
    let project_name = crate::routes::ingest_project_name(target)
        .ok_or(PersistError::InvalidPayload)?
        .to_string();

    if let Some(batch) = parse_and_validate_batch_payload(&project_name, target, body)? {
        return Ok(batch);
    }

    let wire = parse_payload(target, body)?;
    parse_and_validate_wire(project_name, wire).map(|validated| {
        validated
            .map(|validated| ParsedPayloadOutcome::Valid(Box::new(validated)))
            .unwrap_or(ParsedPayloadOutcome::Ignore)
    })
}

fn parse_and_validate_wire(
    project_name: String,
    mut wire: TrackRequest,
) -> Result<Option<ValidatedPayload>, PersistError> {
    let Some(hit_kind) = parse_hit_kind(wire.hit_type) else {
        return Ok(None);
    };
    let created_at = wire.time.unwrap_or_else(milliseconds_since_epoch);
    // Both readings come from the same client clock, so their difference is how
    // long the hit waited in the tracker's queue, independent of clock skew.
    let client_age_ms = wire
        .send_time
        .map(|send_time| (send_time - created_at).clamp(0, MAX_CLIENT_AGE_MS))
        .unwrap_or(0);
    wire.language = normalized_language(wire.language);

    Ok(Some(ValidatedPayload {
        project_name,
        hit_kind,
        created_at,
        client_age_ms,
        wire,
    }))
}

fn parse_and_validate_batch_payload(
    project_name: &str,
    target: &str,
    body: &[u8],
) -> Result<Option<ParsedPayloadOutcome>, PersistError> {
    let query = std::str::from_utf8(extract_payload(target, body))
        .map_err(|_| PersistError::InvalidPayload)?;
    let envelope = serde_urlencoded::from_str::<TrackBatchRequest>(query)
        .map_err(|_| PersistError::InvalidPayload)?;
    let Some(items_json) = envelope.items.as_deref() else {
        return Ok(None);
    };

    let mut items = serde_json::from_str::<Vec<TrackRequest>>(items_json)
        .map_err(|_| PersistError::InvalidPayload)?;
    if items.is_empty() || items.len() > MAX_TRACK_BATCH_ITEMS {
        return Err(PersistError::InvalidPayload);
    }

    let mut validated_items = Vec::with_capacity(items.len());
    for mut item in items.drain(..) {
        apply_batch_defaults(&mut item, &envelope);
        let Some(validated) = parse_and_validate_wire(project_name.to_string(), item)? else {
            return Err(PersistError::InvalidPayload);
        };
        if validated.hit_kind == HitKind::Error {
            return Err(PersistError::InvalidPayload);
        }
        validated_items.push(validated);
    }

    Ok(Some(ParsedPayloadOutcome::Batch(ValidatedBatch {
        project_name: project_name.to_string(),
        items: validated_items,
    })))
}

fn apply_batch_defaults(item: &mut TrackRequest, envelope: &TrackBatchRequest) {
    if item.send_time.is_none() {
        item.send_time = envelope.send_time;
    }
    if item.version.is_none() {
        item.version = envelope.version;
    }
    if item.hostname.is_none() {
        item.hostname = envelope.hostname.clone();
    }
    if item.session_id.is_none() {
        item.session_id = envelope.session_id;
    }
    if item.consent.is_none() {
        item.consent = envelope.consent;
    }
}

fn parse_payload(target: &str, body: &[u8]) -> Result<TrackRequest, PersistError> {
    let query = std::str::from_utf8(extract_payload(target, body))
        .map_err(|_| PersistError::InvalidPayload)?;
    serde_urlencoded::from_str::<TrackRequest>(query).map_err(|_| PersistError::InvalidPayload)
}

pub(crate) fn extract_payload<'a>(target: &'a str, body: &'a [u8]) -> &'a [u8] {
    if !body.is_empty() {
        return body;
    }
    target
        .split_once('?')
        .map(|(_, query)| query.as_bytes())
        .unwrap_or(&[])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extract_payload_returns_query_when_present() {
        assert_eq!(extract_payload("/ga.gif?a=1&b=2", b""), b"a=1&b=2");
    }

    #[test]
    fn extract_payload_prefers_body_when_present() {
        assert_eq!(
            extract_payload("/demo.gif?a=1&b=2", b"ty=pageview"),
            b"ty=pageview"
        );
    }

    #[test]
    fn extract_payload_returns_empty_when_query_missing() {
        assert_eq!(extract_payload("/demo.gif", b""), b"");
        assert_eq!(extract_payload("/demo", b""), b"");
    }

    #[test]
    fn parse_and_validate_payload_accepts_query_payload() {
        let target = "/demo.gif?ty=pageview&t=123";
        let outcome = parse_and_validate_payload(target, b"").expect("payload should parse");
        let ParsedPayloadOutcome::Valid(validated) = outcome else {
            panic!("expected validated payload");
        };

        assert_eq!(validated.project_name, "demo");
        assert_eq!(validated.hit_kind, HitKind::Pageview);
        assert_eq!(validated.created_at, 123);
    }

    #[test]
    fn parse_and_validate_payload_accepts_engagement_query() {
        let target = "/demo?ty=engagement&pg=ffeeddccbbaa998877665544";
        let outcome = parse_and_validate_payload(target, b"").expect("payload should parse");
        let ParsedPayloadOutcome::Valid(validated) = outcome else {
            panic!("expected validated payload");
        };

        assert_eq!(validated.hit_kind, HitKind::Append);
    }

    #[test]
    fn parse_and_validate_payload_accepts_forget_query() {
        let target = "/demo?ty=forget&pg=ffeeddccbbaa998877665544&s=000000000000007b";
        let outcome = parse_and_validate_payload(target, b"").expect("payload should parse");
        let ParsedPayloadOutcome::Valid(validated) = outcome else {
            panic!("expected validated payload");
        };

        assert_eq!(validated.hit_kind, HitKind::Forget);
        assert_eq!(validated.wire.session_id, Some(123));
    }

    #[test]
    fn parse_and_validate_payload_rejects_invalid_project_name() {
        let target = "/invalid-name.gif?ty=pageview";
        let result = parse_and_validate_payload(target, b"");
        assert!(matches!(result, Err(PersistError::InvalidPayload)));
    }

    #[test]
    fn parse_and_validate_payload_ignores_non_tracker_payloads() {
        let unknown_type = parse_and_validate_payload("/demo.gif?ty=unknown", b"")
            .expect("unknown type should be ignored");
        assert!(matches!(unknown_type, ParsedPayloadOutcome::Ignore));
    }

    #[test]
    fn parse_payload_decodes_form_fields_with_serde() {
        let wire = parse_payload("/demo.gif", b"p=%2Fdocs%2Fintro&s=000000000000007b&sc=80")
            .expect("query payload should parse");
        assert_eq!(wire.path.as_deref(), Some("/docs/intro"));
        assert_eq!(wire.session_id, Some(123));
        assert_eq!(wire.scrolled, Some(80));
        assert_eq!(wire.consent, None);
    }

    #[test]
    fn parse_payload_rejects_decimal_session_id() {
        let result = parse_payload("/demo.gif", b"s=123");
        assert!(matches!(result, Err(PersistError::InvalidPayload)));
    }

    #[test]
    fn parse_and_validate_payload_accepts_batch_payload() {
        let items = serde_json::to_string(&json!([
            {
                "ty": "pageview",
                "t": 123,
                "pg": "00112233445566778899aabb",
                "p": "/pricing"
            },
            {
                "ty": "event",
                "t": 124,
                "pg": "00112233445566778899aabb",
                "ev": "cta-click"
            },
            {
                "ty": "engagement",
                "t": 125,
                "pg": "00112233445566778899aabb",
                "d": 10,
                "sc": 80
            }
        ]))
        .expect("serialize items");
        let body = serde_urlencoded::to_string([
            ("v", "1.0"),
            ("h", "example.com"),
            ("s", "00000000000001c8"),
            ("c", "true"),
            ("items", items.as_str()),
        ])
        .expect("encode form body");

        let outcome =
            parse_and_validate_payload("/demo", body.as_bytes()).expect("payload should parse");
        let ParsedPayloadOutcome::Batch(batch) = outcome else {
            panic!("expected validated batch");
        };

        assert_eq!(batch.project_name, "demo");
        assert_eq!(batch.items.len(), 3);
        assert_eq!(batch.items[0].hit_kind, HitKind::Pageview);
        assert_eq!(batch.items[0].wire.hostname.as_deref(), Some("example.com"));
        assert_eq!(batch.items[1].hit_kind, HitKind::Event);
        assert_eq!(batch.items[1].wire.session_id, Some(456));
        assert_eq!(batch.items[2].hit_kind, HitKind::Append);
        assert_eq!(batch.items[2].wire.consent, Some(true));
    }

    #[test]
    fn batch_items_take_their_queue_age_from_the_envelope_send_time() {
        let items = serde_json::to_string(&json!([
            {
                "ty": "pageview",
                "t": 1_000,
                "pg": "00112233445566778899aabb",
                "p": "/pricing"
            },
            {
                "ty": "event",
                "t": 9_500,
                "pg": "00112233445566778899aabb",
                "ev": "cta-click"
            }
        ]))
        .expect("serialize items");
        let body = serde_urlencoded::to_string([
            ("h", "example.com"),
            ("st", "10000"),
            ("items", items.as_str()),
        ])
        .expect("encode form body");

        let outcome =
            parse_and_validate_payload("/demo", body.as_bytes()).expect("payload should parse");
        let ParsedPayloadOutcome::Batch(batch) = outcome else {
            panic!("expected validated batch");
        };

        assert_eq!(batch.items[0].client_age_ms, 9_000);
        assert_eq!(batch.items[1].client_age_ms, 500);
    }

    #[test]
    fn client_age_ignores_send_times_that_precede_the_hit() {
        let outcome = parse_and_validate_payload("/demo.gif?ty=pageview&t=5000&st=4000", b"")
            .expect("payload should parse");
        let ParsedPayloadOutcome::Valid(validated) = outcome else {
            panic!("expected validated payload");
        };

        assert_eq!(validated.client_age_ms, 0);
    }

    #[test]
    fn client_age_is_capped_for_implausible_clock_jumps() {
        let outcome = parse_and_validate_payload("/demo.gif?ty=pageview&t=0&st=99999999999", b"")
            .expect("payload should parse");
        let ParsedPayloadOutcome::Valid(validated) = outcome else {
            panic!("expected validated payload");
        };

        assert_eq!(validated.client_age_ms, MAX_CLIENT_AGE_MS);
    }

    #[test]
    fn parse_and_validate_payload_rejects_oversized_batch() {
        let items = serde_json::to_string(&vec![
            json!({
                "ty": "event",
                "ev": "cta-click"
            });
            MAX_TRACK_BATCH_ITEMS + 1
        ])
        .expect("serialize items");
        let body =
            serde_urlencoded::to_string([("items", items.as_str())]).expect("encode form body");

        let result = parse_and_validate_payload("/demo", body.as_bytes());
        assert!(matches!(result, Err(PersistError::InvalidPayload)));
    }
}
