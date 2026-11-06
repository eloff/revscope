use std::time::{SystemTime, UNIX_EPOCH};

/// Block on a future from inside a blocking thread. Handlers run under
/// `tokio::task::spawn_blocking`, which keeps us in the runtime context so we
/// can drive postgres async helpers without making the caller itself async.
pub(crate) fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Handle::current().block_on(future)
}

pub const NANOS_PER_SECOND: i64 = 1_000_000_000;

/// Return the current time in seconds since the Unix epoch.
pub fn seconds_since_epoch() -> u32 {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    u32::try_from(seconds).unwrap_or(u32::MAX)
}

/// Return the current time in nanoseconds since the Unix epoch.
pub fn nanoseconds_since_epoch() -> u64 {
    let now = SystemTime::now();
    let duration = now
        .duration_since(UNIX_EPOCH)
        .expect("SystemTime before UNIX EPOCH!");
    duration.as_secs() * 1_000_000_000 + duration.subsec_nanos() as u64
}

/// Return the current time in whole seconds since the Unix epoch.
pub(crate) fn unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Return the current time in milliseconds since the Unix epoch.
pub(crate) fn milliseconds_since_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

/// The smallest UUIDv7 minted `max_age_millis` before `now_millis`, as 32 hex
/// digits. UUIDv7 ids start with their creation time in millis and Postgres
/// compares uuids bytewise, so every row id minted before then sorts below it
/// and `WHERE id < cutoff` deletes by age through the primary key index.
pub(crate) fn uuid_v7_age_cutoff_at(now_millis: u64, max_age_millis: u64) -> String {
    let cutoff_millis = now_millis.saturating_sub(max_age_millis) & 0xFFFF_FFFF_FFFF;
    format!("{:032x}", u128::from(cutoff_millis) << 80)
}

/// [`uuid_v7_age_cutoff_at`] for the current time.
pub(crate) fn uuid_v7_age_cutoff(max_age_millis: u64) -> String {
    let now_millis = u64::try_from(milliseconds_since_epoch()).unwrap_or(0);
    uuid_v7_age_cutoff_at(now_millis, max_age_millis)
}

/// The longest prefix of `text` that is at most `max_chars` characters.
pub(crate) fn truncate_chars(text: &str, max_chars: usize) -> &str {
    match text.char_indices().nth(max_chars) {
        Some((end, _)) => &text[..end],
        None => text,
    }
}

/// Formats Unix seconds as an RFC 3339 UTC timestamp, or `None` outside the
/// years `time` can represent.
pub(crate) fn unix_to_rfc3339(unix: i64) -> Option<String> {
    time::OffsetDateTime::from_unix_timestamp(unix)
        .ok()?
        .format(&time::format_description::well_known::Rfc3339)
        .ok()
}

/// Parses an RFC 3339 timestamp to Unix seconds.
pub(crate) fn rfc3339_to_unix(value: &str) -> Result<i64, time::error::Parse> {
    time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
        .map(|datetime| datetime.unix_timestamp())
}

/// Public origin of the marketing site and dashboard, used to build links the
/// customer follows back to us (password resets, checkout success).
pub(crate) fn web_base_url() -> String {
    let base_url = std::env::var("REVSCOPE_WEB_BASE_URL")
        .or_else(|_| std::env::var("APP_BASE_URL"))
        .unwrap_or_else(|_| "https://www.revscope.co".to_string());
    base_url.trim().trim_end_matches('/').to_string()
}

/// Decodes one `application/x-www-form-urlencoded` query value, the encoding
/// `URLSearchParams` produces: `+` is a space and `%XX` is a byte. Returns
/// `None` for a malformed escape or bytes that aren't UTF-8.
pub(crate) fn decode_query_component(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                output.push(b' ');
                index += 1;
            }
            b'%' => {
                let high = hex_nibble(*bytes.get(index + 1)?)?;
                let low = hex_nibble(*bytes.get(index + 2)?)?;
                output.push((high << 4) | low);
                index += 3;
            }
            byte => {
                output.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(output).ok()
}

/// The value of one ASCII hex digit, either case.
pub(crate) fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Lowercase hex of `bytes`, two digits per byte.
pub(crate) fn encode_lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

pub const VISITOR_TIMESTAMP_ID_HEX_LEN: usize = 16;

/// Encode a visitor/session timestamp id as lowercase hex for JSON and query params.
pub fn encode_visitor_timestamp_id(value: i64) -> String {
    format!(
        "{:0width$x}",
        u64::try_from(value).unwrap_or(0),
        width = VISITOR_TIMESTAMP_ID_HEX_LEN
    )
}

pub fn is_valid_visitor_timestamp_id_hex(value: &str) -> bool {
    value.len() == VISITOR_TIMESTAMP_ID_HEX_LEN
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

pub fn parse_visitor_timestamp_id_hex(value: &str) -> Result<i64, &'static str> {
    if !is_valid_visitor_timestamp_id_hex(value) {
        return Err("invalid visitor_timestamp_id");
    }
    let parsed = u64::from_str_radix(value, 16).map_err(|_| "invalid visitor_timestamp_id")?;
    let id = i64::try_from(parsed).map_err(|_| "invalid visitor_timestamp_id")?;
    if id < 1 {
        return Err("visitor_timestamp_id must be at least 1");
    }
    Ok(id)
}

/// `value`, or `None` when it is missing or empty.
pub(crate) fn non_empty(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.is_empty())
}

/// Splits a comma-separated list, trimming entries and dropping empty ones.
pub(crate) fn split_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_v7_age_cutoff_orders_between_older_and_newer_ids() {
        let now = 1_800_000_000_000_u64;
        let max_age = 60_000;
        let cutoff = uuid_v7_age_cutoff_at(now, max_age);
        let v7_at = |millis: u64| format!("{:012x}7fff{:016x}", millis, u64::MAX);
        assert_eq!(cutoff.len(), 32);
        assert!(v7_at(now - max_age - 1) < cutoff);
        assert!(v7_at(now - max_age) > cutoff);
    }

    #[test]
    fn truncate_chars_cuts_on_a_character_boundary() {
        assert_eq!(truncate_chars("héllo", 2), "hé");
        assert_eq!(truncate_chars("héllo", 5), "héllo");
        assert_eq!(truncate_chars("héllo", 50), "héllo");
        assert_eq!(truncate_chars("", 0), "");
    }

    #[test]
    fn rfc3339_round_trips_unix_seconds() {
        let unix = 1_788_000_000;
        let formatted = unix_to_rfc3339(unix).expect("in range");
        assert_eq!(formatted, "2026-08-29T10:40:00Z");
        assert_eq!(rfc3339_to_unix(&formatted).ok(), Some(unix));
        assert_eq!(unix_to_rfc3339(i64::MAX), None);
    }

    #[test]
    fn encodes_bytes_as_lowercase_hex() {
        assert_eq!(encode_lower_hex(&[0x00, 0x0f, 0xa0, 0xff]), "000fa0ff");
        assert_eq!(encode_lower_hex(&[]), "");
    }

    #[test]
    fn decodes_form_encoded_query_values() {
        assert_eq!(
            decode_query_component("US-New+York").as_deref(),
            Some("US-New York")
        );
        assert_eq!(
            decode_query_component("FR-Saint-%C3%89tienne").as_deref(),
            Some("FR-Saint-Étienne")
        );
        assert_eq!(decode_query_component("a%2Bb%25").as_deref(), Some("a+b%"));
        assert_eq!(decode_query_component("trailing%4"), None);
        assert_eq!(decode_query_component("bad%zz"), None);
        assert_eq!(decode_query_component("%FF"), None);
    }

    #[test]
    fn splits_and_trims_comma_separated_lists() {
        assert_eq!(
            split_list(" api.revscope.co , revscope.co ,"),
            vec!["api.revscope.co".to_string(), "revscope.co".to_string()]
        );
        assert!(split_list("  ").is_empty());
    }

    #[test]
    fn encodes_and_parses_large_visitor_timestamp_ids() {
        let value = 1_750_612_345_678_901_234;
        let hex = encode_visitor_timestamp_id(value);
        assert_eq!(hex.len(), VISITOR_TIMESTAMP_ID_HEX_LEN);
        assert_eq!(parse_visitor_timestamp_id_hex(&hex).expect("parse"), value);
    }

    #[test]
    fn rejects_decimal_visitor_timestamp_ids() {
        assert!(parse_visitor_timestamp_id_hex("1").is_err());
        assert!(parse_visitor_timestamp_id_hex("0000000000000000").is_err());
        assert!(parse_visitor_timestamp_id_hex("ABCDEF0123456789").is_err());
    }
}
