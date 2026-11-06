use xxhash_rust::xxh3::xxh3_128;

use super::user_agent::normalized_user_agent;
use super::wire::TrackRequest;
use crate::http::cookie_value;
use crate::util::{encode_visitor_timestamp_id, parse_visitor_timestamp_id_hex};

const USER_SESSION_SHORT_TTL_DAYS: i64 = 1;
const USER_SESSION_LONG_TTL_DAYS: i64 = 90;
const SESSION_COOKIE_MAX_AGE_SECONDS: i64 = USER_SESSION_LONG_TTL_DAYS * 24 * 60 * 60;

#[derive(Debug)]
pub(super) enum SessionCookie {
    Set(i64),
    Clear,
}

/// Cookieless visitor identity: the `User-Agent` header (as we store it) hashed
/// together with the visitor's IP.
pub(super) fn build_user_hash(user_agent: Option<&str>, client_ip: Option<&str>) -> u128 {
    fn append_optional_string(buffer: &mut Vec<u8>, value: Option<&str>) {
        match value {
            Some(value) => {
                buffer.push(1);
                buffer.extend_from_slice(&(value.len() as u64).to_le_bytes());
                buffer.extend_from_slice(value.as_bytes());
            }
            None => buffer.push(0),
        }
    }

    let mut bytes = Vec::with_capacity(256);
    append_optional_string(&mut bytes, normalized_user_agent(user_agent).as_deref());
    append_optional_string(&mut bytes, client_ip);
    xxh3_128(&bytes)
}

pub(super) fn resolve_session_expires_after_days(payload: &TrackRequest) -> i64 {
    if payload.session_id.is_none() || payload.consent == Some(false) {
        USER_SESSION_SHORT_TTL_DAYS
    } else {
        USER_SESSION_LONG_TTL_DAYS
    }
}

/// Whether cookies and terminal-device reads are lawful for this visitor.
/// Explicit consent decides on its own; otherwise the visitor's jurisdiction
/// does. An existing session id does not change the answer — the tracker
/// refreshes the cookie whenever this is true.
pub(super) fn is_storage_allowed(
    consent: Option<bool>,
    country: Option<&str>,
    region: Option<&str>,
) -> bool {
    match consent {
        Some(consent) => consent,
        None => !crate::consent::location_requires_consent(country, region),
    }
}

fn session_cookie_name(project_name: &str) -> String {
    format!("rs{project_name}")
}

pub(super) fn apply_session_from_cookie(
    payload: &mut TrackRequest,
    cookie_header: Option<&str>,
    project_name: &str,
) {
    if payload.session_id.is_some() || payload.consent == Some(false) {
        return;
    }
    payload.session_id = parse_session_cookie(cookie_header, project_name);
}

pub(super) fn session_cookie_for_payload(
    consent: Option<bool>,
    storage_allowed: bool,
    visitor_timestamp_id: i64,
) -> Option<SessionCookie> {
    if consent == Some(false) {
        Some(SessionCookie::Clear)
    } else if storage_allowed {
        Some(SessionCookie::Set(visitor_timestamp_id))
    } else {
        None
    }
}

pub(super) fn session_cookie_headers(
    project_name: &str,
    cookie: Option<&SessionCookie>,
) -> Vec<(&'static str, String)> {
    let name = session_cookie_name(project_name);
    match cookie {
        Some(SessionCookie::Set(visitor_timestamp_id)) => vec![(
            "Set-Cookie",
            format!(
                "{name}={}; Path=/; Max-Age={SESSION_COOKIE_MAX_AGE_SECONDS}; SameSite=None; Secure",
                encode_visitor_timestamp_id(*visitor_timestamp_id)
            ),
        )],
        Some(SessionCookie::Clear) => vec![(
            "Set-Cookie",
            format!("{name}=; Path=/; Max-Age=0; SameSite=None; Secure"),
        )],
        None => Vec::new(),
    }
}

fn parse_session_cookie(cookie_header: Option<&str>, project_name: &str) -> Option<i64> {
    let value = cookie_value(cookie_header?, &session_cookie_name(project_name))?;
    parse_visitor_timestamp_id_hex(value).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::test_support::sample_payload;

    #[test]
    fn resolves_session_expires_after_days_from_payload() {
        let mut payload = sample_payload();
        payload.session_id = None;
        payload.consent = Some(false);
        assert_eq!(
            resolve_session_expires_after_days(&payload),
            USER_SESSION_SHORT_TTL_DAYS
        );

        payload.consent = Some(true);
        assert_eq!(
            resolve_session_expires_after_days(&payload),
            USER_SESSION_SHORT_TTL_DAYS
        );

        payload.consent = Some(false);
        payload.session_id = Some(1_000);
        assert_eq!(
            resolve_session_expires_after_days(&payload),
            USER_SESSION_SHORT_TTL_DAYS
        );

        payload.consent = None;
        assert_eq!(
            resolve_session_expires_after_days(&payload),
            USER_SESSION_LONG_TTL_DAYS
        );

        payload.consent = Some(true);
        assert_eq!(
            resolve_session_expires_after_days(&payload),
            USER_SESSION_LONG_TTL_DAYS
        );
    }

    #[test]
    fn parse_session_cookie_reads_project_session_cookie_from_header() {
        assert_eq!(
            parse_session_cookie(Some("rsdemo=00000000499602d2; other=value"), "demo"),
            Some(1_234_567_890)
        );
        assert_eq!(
            parse_session_cookie(Some("other=value; rsdemo=000000000000002a"), "demo"),
            Some(42)
        );
        assert_eq!(parse_session_cookie(Some("rsdemo="), "demo"), None);
        assert_eq!(
            parse_session_cookie(Some("rsdemo=0000000000000000"), "demo"),
            None
        );
        assert_eq!(parse_session_cookie(Some("rsdemo=42"), "demo"), None);
        assert_eq!(
            parse_session_cookie(Some("rsother=00000000499602d2"), "demo"),
            None
        );
        assert_eq!(parse_session_cookie(None, "demo"), None);
    }

    #[test]
    fn apply_session_from_cookie_fills_missing_session_id() {
        let mut payload = sample_payload();
        payload.session_id = None;
        payload.consent = None;
        apply_session_from_cookie(&mut payload, Some("rsdemo=00000000000003e7"), "demo");
        assert_eq!(payload.session_id, Some(999));

        payload.session_id = Some(123);
        apply_session_from_cookie(&mut payload, Some("rsdemo=00000000000003e7"), "demo");
        assert_eq!(payload.session_id, Some(123));

        payload.session_id = None;
        payload.consent = Some(false);
        apply_session_from_cookie(&mut payload, Some("rsdemo=00000000000003e7"), "demo");
        assert_eq!(payload.session_id, None);
    }

    #[test]
    fn storage_allowed_follows_consent_then_jurisdiction() {
        // Explicit consent decides on its own, wherever the visitor is.
        assert!(is_storage_allowed(Some(true), Some("DE"), Some("BE")));
        assert!(!is_storage_allowed(Some(false), Some("US"), None));
        assert!(is_storage_allowed(Some(true), Some("BR"), None));
        assert!(!is_storage_allowed(Some(false), Some("BR"), None));
        assert!(is_storage_allowed(Some(true), Some("CA"), Some("QC")));
        // Without a decision, the visitor's jurisdiction decides.
        assert!(is_storage_allowed(None, Some("US"), None));
        assert!(!is_storage_allowed(None, Some("DE"), None));
        // Brazil is not consent-required for cookies.
        assert!(is_storage_allowed(None, Some("BR"), None));
        // Quebec Law 25: consent required only for CA+QC; fail open otherwise.
        assert!(!is_storage_allowed(None, Some("CA"), Some("QC")));
        assert!(!is_storage_allowed(None, Some("ca"), Some("qc")));
        assert!(is_storage_allowed(None, Some("CA"), Some("ON")));
        assert!(is_storage_allowed(None, Some("CA"), None));
        // Unknown country fails open.
        assert!(is_storage_allowed(None, None, None));
    }
}
