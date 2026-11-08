use std::sync::OnceLock;
use std::time::{Duration, Instant};

use fxhash::FxHashMap;
use parking_lot::Mutex;
use sentry::protocol::{Context, Map, Value};

use crate::ingest::INGEST_PERSIST_INTERNAL_ERROR;

/// JSON object keys replaced before a request body is attached to a Sentry event.
const REDACTED_JSON_KEYS: &[&str] = &[
    "password",
    "hashed_password",
    "token",
    "id_token",
    "session_id",
    "api_key",
    "secret",
    "service_account_json",
    "oauth_code",
];

/// At most one Sentry-bound `error!` per key within this window.
const SENTRY_DEBOUNCE_WINDOW: Duration = Duration::from_secs(60 * 60);

/// Opportunistically prune expired keys once the map exceeds this size.
const KEYED_DEBOUNCER_PRUNE_THRESHOLD: usize = 1024;

struct KeyedDebouncer {
    window: Duration,
    state: Mutex<FxHashMap<String, Instant>>,
}

impl KeyedDebouncer {
    fn new(window: Duration) -> Self {
        Self {
            window,
            state: Mutex::new(FxHashMap::default()),
        }
    }

    fn try_acquire(&self, key: &str) -> bool {
        let mut map = self.state.lock();
        let now = Instant::now();
        if map.len() > KEYED_DEBOUNCER_PRUNE_THRESHOLD {
            let window = self.window;
            map.retain(|_, at| now.duration_since(*at) < window);
        }
        match map.get(key) {
            Some(at) if now.duration_since(*at) < self.window => false,
            _ => {
                map.insert(key.to_owned(), now);
                true
            }
        }
    }
}

fn sentry_debouncer() -> &'static KeyedDebouncer {
    static DEBOUNCER: OnceLock<KeyedDebouncer> = OnceLock::new();
    DEBOUNCER.get_or_init(|| KeyedDebouncer::new(SENTRY_DEBOUNCE_WINDOW))
}

/// Whether an error with this key should still be logged at `error!` (and thus
/// forwarded to Sentry via `SentryLogger`). Callers should `warn!` instead when
/// this returns false so local logs stay complete without burning quota.
pub(crate) fn should_report(key: &str) -> bool {
    sentry_debouncer().try_acquire(key)
}

/// Log at `error!` at most once per key per hour; otherwise `warn!`.
macro_rules! report_error {
    ($key:expr, $($arg:tt)*) => {{
        if $crate::sentry_report::should_report($key) {
            ::log::error!($($arg)*);
        } else {
            ::log::warn!($($arg)*);
        }
    }};
}
pub(crate) use report_error;

/// Whether an internal-error detail should be logged at `error!` (and thus
/// forwarded to Sentry via `SentryLogger`). Ingest persist failures are
/// debounced so an outage cannot exhaust quota; callers should `warn!`
/// instead when this returns false.
pub(crate) fn should_error_log_internal_detail(detail: &str) -> bool {
    if detail.starts_with(INGEST_PERSIST_INTERNAL_ERROR) {
        return should_report("ingest.persist");
    }
    true
}

/// Runs `log` with HTTP request fields on a temporary Sentry scope so
/// `error!` → `SentryLogger` includes them on that event only.
pub(crate) fn with_http_context(
    method: &str,
    path: &str,
    status: u16,
    body: &[u8],
    log: impl FnOnce(),
) {
    sentry::with_scope(
        |scope| {
            scope.set_tag("http.method", method);
            scope.set_tag("http.status_code", status);
            scope.set_context(
                "http",
                Context::Other(http_context_map(method, path, status, body)),
            );
        },
        log,
    );
}

fn http_context_map(method: &str, path: &str, status: u16, body: &[u8]) -> Map<String, Value> {
    let mut map = Map::new();
    map.insert("method".into(), method.into());
    map.insert("path".into(), path.into());
    map.insert("status_code".into(), u64::from(status).into());
    if !method.eq_ignore_ascii_case("GET")
        && let Some(value) = json_body_for_sentry(body)
    {
        map.insert("body".into(), value);
    }
    map
}

fn json_body_for_sentry(body: &[u8]) -> Option<Value> {
    let mut value = serde_json::from_slice(body).ok()?;
    redact_json(&mut value);
    Some(value)
}

fn redact_json(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for (key, child) in object.iter_mut() {
                if REDACTED_JSON_KEYS
                    .iter()
                    .any(|name| key.eq_ignore_ascii_case(name))
                {
                    *child = Value::String("redacted".into());
                } else {
                    redact_json(child);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                redact_json(item);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use revscope_api::ga_import::ConnectRequest;
    use revscope_api::settings::{IntegrationInput, SaveSettingsRequest};

    use super::*;

    #[test]
    fn keyed_debouncer_allows_once_per_key_then_blocks() {
        let debouncer = KeyedDebouncer::new(Duration::from_secs(60));
        assert!(debouncer.try_acquire("a"));
        assert!(!debouncer.try_acquire("a"));
        assert!(debouncer.try_acquire("b"));
        assert!(!debouncer.try_acquire("b"));
    }

    #[test]
    fn keyed_debouncer_resets_after_window() {
        let debouncer = KeyedDebouncer::new(Duration::from_millis(20));
        assert!(debouncer.try_acquire("a"));
        assert!(!debouncer.try_acquire("a"));
        std::thread::sleep(Duration::from_millis(25));
        assert!(debouncer.try_acquire("a"));
    }

    #[test]
    fn keyed_debouncer_prunes_expired_when_large() {
        let debouncer = KeyedDebouncer::new(Duration::from_millis(20));
        for i in 0..(KEYED_DEBOUNCER_PRUNE_THRESHOLD + 1) {
            assert!(debouncer.try_acquire(&format!("old-{i}")));
        }
        std::thread::sleep(Duration::from_millis(25));
        assert!(debouncer.try_acquire("trigger-prune"));
        let map = debouncer.state.lock();
        assert!(map.len() < KEYED_DEBOUNCER_PRUNE_THRESHOLD);
        assert!(map.contains_key("trigger-prune"));
    }

    #[test]
    fn http_context_includes_parsed_json_body_for_post() {
        let map = http_context_map("POST", "/v2/session/auth/login", 500, br#"{"user":"a"}"#);
        assert_eq!(map["method"], "POST");
        assert_eq!(map["path"], "/v2/session/auth/login");
        assert_eq!(map["status_code"], 500);
        assert_eq!(map["body"]["user"], "a");
    }

    #[test]
    fn http_context_omits_body_for_get_and_non_json() {
        let get = http_context_map("GET", "/v2/read/projects/1/overview?country=US", 500, b"{}");
        assert!(!get.contains_key("body"));
        let not_json = http_context_map("POST", "/demo", 500, b"ty=pageview");
        assert!(!not_json.contains_key("body"));
    }

    #[test]
    fn http_context_redacts_password_and_nested_token() {
        let map = http_context_map(
            "POST",
            "/v2/session/auth/login",
            500,
            br#"{"password":"secret","nested":{"token":"abc"},"id_token":"eyJ","ok":1}"#,
        );
        assert_eq!(map["body"]["password"], "redacted");
        assert_eq!(map["body"]["nested"]["token"], "redacted");
        assert_eq!(map["body"]["id_token"], "redacted");
        assert_eq!(map["body"]["ok"], 1);
    }

    #[test]
    fn http_context_redacts_integration_credentials_from_saved_settings() {
        let body = serde_json::to_vec(&SaveSettingsRequest {
            integrations: vec![
                IntegrationInput {
                    integration_type: "stripe".into(),
                    external_account_id: Some("acct_1".into()),
                    api_key: Some("sk_live_customer".into()),
                    secret: None,
                },
                IntegrationInput {
                    integration_type: "google_search_console".into(),
                    external_account_id: None,
                    api_key: None,
                    secret: Some("-----BEGIN PRIVATE KEY-----".into()),
                },
            ],
            ..Default::default()
        })
        .expect("serialize settings");

        let map = http_context_map("POST", "/v2/session/projects/1/settings", 500, &body);

        let integrations = &map["body"]["integrations"];
        assert_eq!(integrations[0]["api_key"], "redacted");
        assert_eq!(integrations[0]["integration_type"], "stripe");
        assert_eq!(integrations[0]["external_account_id"], "acct_1");
        assert_eq!(integrations[1]["secret"], "redacted");
    }

    #[test]
    fn http_context_redacts_google_analytics_connect_credentials() {
        let service_account = serde_json::to_vec(&ConnectRequest::ServiceAccount {
            service_account_json: r#"{"private_key":"-----BEGIN PRIVATE KEY-----"}"#.into(),
        })
        .expect("serialize service account");
        let oauth = serde_json::to_vec(&ConnectRequest::OAuth {
            oauth_code: "4/0google".into(),
            oauth_state: "state".into(),
        })
        .expect("serialize oauth");
        let path = "/v2/session/projects/1/ga-imports/connect";

        let map = http_context_map("POST", path, 500, &service_account);
        assert_eq!(map["body"]["service_account_json"], "redacted");

        let map = http_context_map("POST", path, 500, &oauth);
        assert_eq!(map["body"]["oauth_code"], "redacted");
    }
}
