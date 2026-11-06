use compact_str::CompactString;
use serde_json::Value;

use super::error::PersistError;
use super::rows::ScreenDimensions;
use super::wire::{JsonField, TrackRequest};
use crate::http::ParsedRequest;
use crate::util::non_empty;

fn payload_dimension(value: Option<i64>) -> Option<i64> {
    value.filter(|inner| *inner > 0)
}

pub(super) fn normalized_screen_dimensions(payload: &TrackRequest) -> Option<ScreenDimensions> {
    let screen_pair = payload_dimension_pair(payload.screen_width, payload.screen_height);
    let viewport_pair = payload_dimension_pair(payload.viewport_width, payload.viewport_height);

    match (screen_pair, viewport_pair) {
        (Some((screen_width, screen_height)), Some((viewport_width, viewport_height))) => {
            Some(ScreenDimensions {
                screen_width,
                screen_height,
                viewport_width,
                viewport_height,
            })
        }
        (Some((screen_width, screen_height)), None) => Some(ScreenDimensions {
            screen_width,
            screen_height,
            viewport_width: screen_width,
            viewport_height: screen_height,
        }),
        (None, Some((viewport_width, viewport_height))) => Some(ScreenDimensions {
            screen_width: viewport_width,
            screen_height: viewport_height,
            viewport_width,
            viewport_height,
        }),
        (None, None) => None,
    }
}

fn payload_dimension_pair(width: Option<i64>, height: Option<i64>) -> Option<(i64, i64)> {
    match (payload_dimension(width), payload_dimension(height)) {
        (Some(width), Some(height)) => Some((width, height)),
        _ => None,
    }
}

pub(super) fn required_non_empty(value: Option<&str>) -> Result<&str, PersistError> {
    non_empty(value).ok_or(PersistError::InvalidPayload)
}

pub(super) fn normalized_duration_seconds(value: Option<i64>) -> Option<i64> {
    value.map(|duration| duration.max(0))
}

/// The hit's language, falling back to the `Accept-Language` header when the
/// tracker did not send `navigator.language` (which it only reads once device
/// context collection is allowed). Server-side callers may send `l` directly.
pub(super) fn resolve_language(
    wire: &TrackRequest,
    parsed: &ParsedRequest<'_>,
) -> Option<CompactString> {
    wire.language
        .clone()
        .or_else(|| language_from_accept_language(parsed.accept_language))
}

/// Highest-priority tag of an `Accept-Language` header, e.g. `fr-CH` from
/// `fr-CH, fr;q=0.9, en;q=0.8`. Quality values are not reordered: browsers list
/// the preferred language first.
fn language_from_accept_language(accept_language: Option<&str>) -> Option<CompactString> {
    let first_tag = non_empty(accept_language)?
        .split(',')
        .next()?
        .split(';')
        .next();
    normalized_language(non_empty(first_tag).map(CompactString::new))
}

/// Primary BCP 47 subtag, uppercased; dropped unless exactly two ASCII letters.
pub(super) fn normalized_language(value: Option<CompactString>) -> Option<CompactString> {
    let value = value?;
    let primary = value.split('-').next().unwrap_or(value.as_str());
    if primary.len() == 2 && primary.bytes().all(|b| b.is_ascii_alphabetic()) {
        Some(CompactString::new(primary.to_ascii_uppercase()))
    } else {
        None
    }
}

pub(super) fn positive_timestamp_id(value: Option<i64>) -> Result<Option<i64>, PersistError> {
    match value {
        Some(value) if value > 0 => Ok(Some(value)),
        Some(_) => Err(PersistError::InvalidPayload),
        None => Ok(None),
    }
}

pub(super) fn normalized_metadata_object(
    value: Option<&JsonField>,
) -> Result<String, PersistError> {
    let normalized = value
        .and_then(JsonField::to_normalized_json)
        .ok_or(PersistError::InvalidPayload)?;
    match serde_json::from_str::<Value>(&normalized) {
        Ok(Value::Object(_)) => Ok(normalized),
        _ => Err(PersistError::InvalidPayload),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::test_support::sample_payload;

    #[test]
    fn normalizes_language_to_uppercase_iso639_1() {
        assert_eq!(
            normalized_language(Some(CompactString::new("en-US"))).as_deref(),
            Some("EN")
        );
        assert_eq!(
            normalized_language(Some(CompactString::new("EN"))).as_deref(),
            Some("EN")
        );
        assert_eq!(
            normalized_language(Some(CompactString::new("zh-Hans"))).as_deref(),
            Some("ZH")
        );
        assert_eq!(normalized_language(Some(CompactString::new("e"))), None);
        assert_eq!(normalized_language(Some(CompactString::new("eng"))), None);
        assert_eq!(normalized_language(Some(CompactString::new("e1"))), None);
        assert_eq!(normalized_language(Some(CompactString::new(""))), None);
        assert_eq!(normalized_language(None), None);
    }

    #[test]
    fn takes_the_first_accept_language_tag() {
        assert_eq!(
            language_from_accept_language(Some("fr-CH, fr;q=0.9, en;q=0.8")).as_deref(),
            Some("FR")
        );
        assert_eq!(
            language_from_accept_language(Some("de;q=0.7")).as_deref(),
            Some("DE")
        );
        assert_eq!(language_from_accept_language(Some("*")), None);
        assert_eq!(language_from_accept_language(Some("  ")), None);
        assert_eq!(language_from_accept_language(None), None);
    }

    #[test]
    fn normalizes_screen_dimensions_from_single_pair() {
        let mut payload = sample_payload();
        payload.viewport_width = None;
        payload.viewport_height = None;
        let normalized = normalized_screen_dimensions(&payload).expect("screen pair");
        assert_eq!(normalized.screen_width, 1920);
        assert_eq!(normalized.screen_height, 1080);
        assert_eq!(normalized.viewport_width, 1920);
        assert_eq!(normalized.viewport_height, 1080);

        payload.screen_width = None;
        payload.screen_height = None;
        payload.viewport_width = Some(1200);
        payload.viewport_height = Some(800);
        let normalized = normalized_screen_dimensions(&payload).expect("viewport pair");
        assert_eq!(normalized.screen_width, 1200);
        assert_eq!(normalized.screen_height, 800);
        assert_eq!(normalized.viewport_width, 1200);
        assert_eq!(normalized.viewport_height, 800);
    }

    #[test]
    fn ignores_partial_or_missing_screen_dimensions() {
        let mut payload = sample_payload();
        payload.screen_height = None;
        let normalized = normalized_screen_dimensions(&payload).expect("falls back to viewport");
        assert_eq!(normalized.screen_width, 1200);
        assert_eq!(normalized.screen_height, 800);
        assert_eq!(normalized.viewport_width, 1200);
        assert_eq!(normalized.viewport_height, 800);

        payload.screen_width = None;
        payload.screen_height = None;
        payload.viewport_width = Some(100);
        payload.viewport_height = None;
        assert_eq!(normalized_screen_dimensions(&payload), None);
    }
}
