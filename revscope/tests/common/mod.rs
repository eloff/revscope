// Shared by several test binaries; each one uses a different subset.
#![allow(dead_code)]

pub mod signup;

use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use fxhash::FxHashMap;
use revscope::server_context::ServerContext;
use revscope::{ServerHooks, bind_listener, flush_all_ingest, run_listener};
use rusqlite::Connection as RawConnection;
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{Notify, OnceCell};

static TEST_NAMESPACE_COUNTER: AtomicU64 = AtomicU64::new(0);
static MAIN_MIGRATIONS: OnceCell<()> = OnceCell::const_new();
async fn ensure_main_migrations(context: Arc<ServerContext>) -> io::Result<()> {
    MAIN_MIGRATIONS
        .get_or_try_init(|| async move {
            revscope::db::apply_main_migrations(context)
                .await
                .map_err(io::Error::other)
        })
        .await?;
    Ok(())
}

pub const DEMO_PROJECT_ID: &str = "1";
pub const DEMO_PROJECT_ID_B: &str = "2";
pub const DEMO_PROJECT_NAME: &str = "demo";
pub const DEMO_PROJECT_NAME_B: &str = "demo_b";
/// Placeholder account UUID for in-memory demo project caches (not a real account).
pub const DEMO_ACCOUNT_ID: &str = "00000000-0000-0000-0000-0000000000a1";
pub const DEMO_ACCOUNT_ID_B: &str = "00000000-0000-0000-0000-0000000000a2";
pub const CHROME_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
pub const FIREFOX_UA: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:121.0) Gecko/20100101 Firefox/121.0";
pub const ANDROID_UA: &str = "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Mobile Safari/537.36";

/// The client id every test server accepts Google and Apple ID tokens for,
/// when signed by [`SIGN_IN_PRIVATE_KEY`] as key [`SIGN_IN_KEY_ID`].
pub const SIGN_IN_CLIENT_ID: &str = "test-client";
pub const SIGN_IN_KEY_ID: &str = "test-key";
pub const SIGN_IN_PRIVATE_KEY: &str = include_str!("../../testdata/id-token-test-key.pem");
const SIGN_IN_PUBLIC_KEY: &str = include_str!("../../testdata/id-token-test-key.pub.pem");
pub const GOOGLE_ISSUER: &str = "https://accounts.google.com";
pub const APPLE_ISSUER: &str = "https://appleid.apple.com";

/// An ID token the test server accepts as coming from `issuer`.
pub fn id_token(issuer: &str, subject: &str, email: Option<&str>, verified: Value) -> String {
    let exp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_secs()
        + 600;
    let mut claims = serde_json::json!({
        "iss": issuer,
        "aud": SIGN_IN_CLIENT_ID,
        "sub": subject,
        "exp": exp,
        "email_verified": verified,
    });
    if let Some(email) = email {
        claims["email"] = serde_json::json!(email);
    }
    if issuer == GOOGLE_ISSUER {
        claims["name"] = serde_json::json!("Grace Hopper");
    }
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    header.kid = Some(SIGN_IN_KEY_ID.to_owned());
    let key =
        jsonwebtoken::EncodingKey::from_rsa_pem(SIGN_IN_PRIVATE_KEY.as_bytes()).expect("test key");
    jsonwebtoken::encode(&header, &claims, &key).expect("sign test token")
}

pub struct TestServer {
    pub addr: SocketAddr,
    pub context: Arc<ServerContext>,
    shutdown: Arc<Notify>,
    /// Raw connections that keep shared-cache in-memory DBs alive. Kept outside
    /// the project DB pool so they cannot block `BEGIN IMMEDIATE` writes.
    anchors: FxHashMap<String, RawConnection>,
    data_dir: Option<PathBuf>,
}

impl TestServer {
    pub async fn spawn(label: &str) -> io::Result<Self> {
        Self::start(label, false).await
    }

    /// Like [`Self::spawn`], but project databases are files in a temporary
    /// data directory, for the routes that only work on disk (database
    /// downloads). Dropping the server removes the directory; a test process
    /// that is killed leaves it behind in the temp directory.
    pub async fn spawn_on_disk(label: &str) -> io::Result<Self> {
        Self::start(label, true).await
    }

    /// Like [`Self::spawn_on_disk`], with Google Analytics traffic sent to `base_url`.
    #[allow(dead_code)]
    pub async fn spawn_with_ga_api(label: &str, base_url: &str) -> io::Result<Self> {
        Self::start_configured(label, true, |context| context.use_ga_test_api(base_url)).await
    }

    /// Like [`Self::spawn`], with `token` configured as the MCP demo token.
    #[allow(dead_code)]
    pub async fn spawn_with_mcp_demo_token(label: &str, token: &str) -> io::Result<Self> {
        Self::start_configured(label, false, |context| context.use_mcp_demo_token(token)).await
    }

    async fn start(label: &str, on_disk: bool) -> io::Result<Self> {
        Self::start_configured(label, on_disk, |_| {}).await
    }

    async fn start_configured(
        label: &str,
        on_disk: bool,
        configure: impl FnOnce(&mut ServerContext),
    ) -> io::Result<Self> {
        revscope::test_env::load_e2e_env();
        assert!(
            std::env::var_os("DATABASE_URL").is_some(),
            "DATABASE_URL must be set to run e2e tests (export it or add it to .e2e.env)"
        );

        let _ =
            env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("error"))
                .is_test(true)
                .try_init();
        revscope::db::migrations::initialize_available_migrations().map_err(io::Error::other)?;

        let unique = TEST_NAMESPACE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let namespace = format!(
            "e2e-{}-{}-{}",
            sanitize_label(label),
            std::process::id(),
            unique
        );
        let data_dir = on_disk.then(|| std::env::temp_dir().join(format!("revscope-{namespace}")));
        let mut context = match &data_dir {
            Some(dir) => {
                std::fs::create_dir_all(dir)?;
                ServerContext::new_on_disk(dir.clone())
            }
            None => ServerContext::new_in_memory(namespace),
        };
        context.trust_sign_in_test_key(SIGN_IN_CLIENT_ID, SIGN_IN_KEY_ID, SIGN_IN_PUBLIC_KEY);
        configure(&mut context);
        let context = Arc::new(context);
        assert!(
            context
                .cache_project(
                    DEMO_PROJECT_ID
                        .parse::<i32>()
                        .expect("valid demo project id"),
                    DEMO_PROJECT_NAME,
                    DEMO_ACCOUNT_ID,
                )
                .is_some()
        );
        assert!(
            context
                .cache_project(
                    DEMO_PROJECT_ID_B
                        .parse::<i32>()
                        .expect("valid demo project id"),
                    DEMO_PROJECT_NAME_B,
                    DEMO_ACCOUNT_ID_B,
                )
                .is_some()
        );
        ensure_main_migrations(context.clone()).await?;
        revscope::db::create_project_database(context.as_ref(), DEMO_PROJECT_ID)
            .map_err(io::Error::other)?;
        revscope::db::create_project_database(context.as_ref(), DEMO_PROJECT_ID_B)
            .map_err(io::Error::other)?;

        let listener = bind_listener("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let shutdown = Arc::new(Notify::new());
        let shutdown_for_task = shutdown.clone();
        let context_for_task = context.clone();
        tokio::spawn(async move {
            let _ = run_listener(
                listener,
                ServerHooks::default(),
                shutdown_for_task,
                context_for_task,
            )
            .await;
        });

        let mut server = Self {
            addr,
            context,
            shutdown,
            anchors: FxHashMap::default(),
            data_dir,
        };
        // Keep shared-memory DBs alive even if the pool cache expires.
        server.ensure_anchor(DEMO_PROJECT_ID);
        server.ensure_anchor(DEMO_PROJECT_ID_B);
        Ok(server)
    }

    pub async fn flush(&self) {
        flush_all_ingest(self.context.clone()).await;
    }

    /// Ensure the shared-memory project DB stays alive, then return a fresh
    /// pooled read connection for assertions.
    pub fn project_db(&mut self, project_id: &str) -> revscope::db::ProjectConnection {
        self.ensure_anchor(project_id);
        revscope::db::open_project_database_read(self.context.as_ref(), project_id)
            .expect("open project database for assertions")
    }

    fn ensure_anchor(&mut self, project_id: &str) {
        if self.data_dir.is_some() || self.anchors.contains_key(project_id) {
            return;
        }
        let uri = revscope::db::project_database_path(self.context.as_ref(), project_id);
        let conn = RawConnection::open(&uri).expect("open shared-memory anchor connection");
        let _ = conn.execute_batch("SELECT 1");
        self.anchors.insert(project_id.to_string(), conn);
    }

    pub async fn post_form(
        &self,
        path: &str,
        body: &str,
        headers: &[(&str, &str)],
    ) -> io::Result<HttpResponse> {
        self.post_body(path, "application/x-www-form-urlencoded", body, headers)
            .await
    }

    pub async fn post_json(
        &self,
        path: &str,
        body: &str,
        headers: &[(&str, &str)],
    ) -> io::Result<HttpResponse> {
        self.post_body(path, "application/json", body, headers)
            .await
    }

    async fn post_body(
        &self,
        path: &str,
        content_type: &str,
        body: &str,
        headers: &[(&str, &str)],
    ) -> io::Result<HttpResponse> {
        let mut request = format!(
            "POST {path} HTTP/1.1\r\nHost: ingest.example.test\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n",
            body.len()
        );
        push_request_headers(&mut request, headers);
        request.push_str("\r\n");
        request.push_str(body);
        send_raw(self.addr, &request).await
    }

    pub async fn get_query(
        &self,
        path_and_query: &str,
        headers: &[(&str, &str)],
    ) -> io::Result<HttpResponse> {
        let mut request = format!(
            "GET {path_and_query} HTTP/1.1\r\nHost: ingest.example.test\r\nConnection: close\r\n"
        );
        push_request_headers(&mut request, headers);
        request.push_str("\r\n");
        send_raw(self.addr, &request).await
    }

    pub async fn delete(
        &self,
        path_and_query: &str,
        headers: &[(&str, &str)],
    ) -> io::Result<HttpResponse> {
        let mut request = format!(
            "DELETE {path_and_query} HTTP/1.1\r\nHost: ingest.example.test\r\nConnection: close\r\nContent-Length: 0\r\n"
        );
        push_request_headers(&mut request, headers);
        request.push_str("\r\n");
        send_raw(self.addr, &request).await
    }

    pub async fn delete_json(
        &self,
        path: &str,
        body: &str,
        headers: &[(&str, &str)],
    ) -> io::Result<HttpResponse> {
        let mut request = format!(
            "DELETE {path} HTTP/1.1\r\nHost: ingest.example.test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
            body.len()
        );
        push_request_headers(&mut request, headers);
        request.push_str("\r\n");
        request.push_str(body);
        send_raw(self.addr, &request).await
    }

    pub async fn post_ingest(
        &self,
        body: &str,
        headers: &[(&str, &str)],
    ) -> io::Result<HttpResponse> {
        self.post_ingest_for(DEMO_PROJECT_NAME, body, headers).await
    }

    pub async fn post_ingest_for(
        &self,
        project_name: &str,
        body: &str,
        headers: &[(&str, &str)],
    ) -> io::Result<HttpResponse> {
        self.post_form(&format!("/{project_name}"), body, headers)
            .await
    }

    pub async fn get_gif(&self, query: &str, headers: &[(&str, &str)]) -> io::Result<HttpResponse> {
        self.get_query(&format!("/{DEMO_PROJECT_NAME}.gif?{query}"), headers)
            .await
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.shutdown.notify_waiters();
        if let Some(dir) = &self.data_dir {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// Ingest reads the visitor's browser from `User-Agent`, and every real client
/// sends one, so default it to Chrome unless the test names its own.
fn push_request_headers(request: &mut String, headers: &[(&str, &str)]) {
    for (name, value) in headers {
        request.push_str(name);
        request.push_str(": ");
        request.push_str(value);
        request.push_str("\r\n");
    }
    if !headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("user-agent"))
    {
        request.push_str("User-Agent: ");
        request.push_str(CHROME_UA);
        request.push_str("\r\n");
    }
}

#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status_line: String,
    pub headers: FxHashMap<String, String>,
    pub set_cookies: Vec<String>,
    pub body: Vec<u8>,
    pub raw: Vec<u8>,
}

impl HttpResponse {
    pub fn status_code(&self) -> u16 {
        self.status_line
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse().ok())
            .unwrap_or(0)
    }

    pub fn body_str(&self) -> &str {
        std::str::from_utf8(&self.body).unwrap_or("")
    }

    pub fn json<T: for<'de> Deserialize<'de>>(&self) -> T {
        serde_json::from_slice(&self.body).expect("response body must be valid JSON")
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }
}

#[derive(Debug, Deserialize)]
pub struct IngestJson {
    #[serde(default, deserialize_with = "deserialize_session_id")]
    pub visitor_timestamp_id: Option<i64>,
    pub storage_allowed: bool,
}

fn deserialize_session_id<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer)?
        .map(|value| {
            revscope::util::parse_visitor_timestamp_id_hex(&value).map_err(serde::de::Error::custom)
        })
        .transpose()
}

#[derive(Debug, Clone, Default)]
pub struct TrackHit {
    pub project_name: Option<String>,
    pub hit_type: Option<&'static str>,
    pub time: Option<i64>,
    pub version: Option<f64>,
    pub hostname: Option<String>,
    pub path: Option<String>,
    pub page_id: Option<String>,
    pub session_id: Option<i64>,
    pub referrer: Option<String>,
    pub duration: Option<i64>,
    pub scrolled: Option<i64>,
    pub viewport_width: Option<i64>,
    pub viewport_height: Option<i64>,
    pub screen_width: Option<i64>,
    pub screen_height: Option<i64>,
    pub timezone: Option<String>,
    pub language: Option<String>,
    pub mobile: Option<bool>,
    pub touch: Option<bool>,
    pub metadata: Option<Value>,
    pub event: Option<String>,
    pub error: Option<String>,
    pub consent: Option<bool>,
}

impl TrackHit {
    pub fn crawler(project_name: &str, path: &str) -> Self {
        Self {
            project_name: Some(project_name.to_string()),
            hit_type: Some("crawler"),
            time: Some(1_700_000_000_000),
            version: Some(1.0),
            hostname: Some("example.com".to_string()),
            path: Some(path.to_string()),
            ..Self::default()
        }
    }

    pub fn pageview(project_name: &str, path: &str, page_id: &str) -> Self {
        Self {
            project_name: Some(project_name.to_string()),
            hit_type: Some("pageview"),
            time: Some(1_700_000_000_000),
            version: Some(1.0),
            hostname: Some("example.com".to_string()),
            path: Some(path.to_string()),
            page_id: Some(page_id.to_string()),
            viewport_width: Some(1280),
            viewport_height: Some(720),
            screen_width: Some(1920),
            screen_height: Some(1080),
            timezone: Some("America/Denver".to_string()),
            // Tracker sends navigator.language (BCP 47); ingest normalizes to ISO 639-1.
            language: Some("en-US".to_string()),
            mobile: Some(false),
            touch: Some(false),
            consent: Some(true),
            ..Self::default()
        }
    }

    /// What the tracker sends for a landing now that nothing is read from the
    /// visitor's device before ingest has allowed it.
    pub fn pageview_without_device_context(project_name: &str, path: &str, page_id: &str) -> Self {
        Self {
            viewport_width: None,
            viewport_height: None,
            screen_width: None,
            screen_height: None,
            timezone: None,
            language: None,
            mobile: None,
            touch: None,
            ..Self::pageview(project_name, path, page_id)
        }
    }

    /// The follow-up hit carrying everything read from the visitor's device,
    /// sent only after ingest answered `storage_allowed`.
    pub fn context(project_name: &str, page_id: &str) -> Self {
        Self {
            project_name: Some(project_name.to_string()),
            hit_type: Some("context"),
            time: Some(1_700_000_000_400),
            version: Some(1.0),
            hostname: Some("example.com".to_string()),
            page_id: Some(page_id.to_string()),
            viewport_width: Some(1280),
            viewport_height: Some(720),
            screen_width: Some(1920),
            screen_height: Some(1080),
            timezone: Some("America/Denver".to_string()),
            language: Some("en-US".to_string()),
            touch: Some(false),
            ..Self::default()
        }
    }

    pub fn event(project_name: &str, name: &str, page_id: &str) -> Self {
        Self {
            project_name: Some(project_name.to_string()),
            hit_type: Some("event"),
            time: Some(1_700_000_000_100),
            version: Some(1.0),
            hostname: Some("example.com".to_string()),
            path: Some("/".to_string()),
            page_id: Some(page_id.to_string()),
            event: Some(name.to_string()),
            consent: Some(true),
            ..Self::default()
        }
    }

    pub fn engagement(project_name: &str, page_id: &str, duration: i64, scrolled: i64) -> Self {
        Self {
            project_name: Some(project_name.to_string()),
            hit_type: Some("engagement"),
            time: Some(1_700_000_000_200),
            version: Some(1.0),
            hostname: Some("example.com".to_string()),
            path: Some("/".to_string()),
            page_id: Some(page_id.to_string()),
            duration: Some(duration),
            scrolled: Some(scrolled),
            consent: Some(true),
            ..Self::default()
        }
    }

    pub fn metadata(project_name: &str, page_id: &str, metadata: Value) -> Self {
        Self {
            project_name: Some(project_name.to_string()),
            hit_type: Some("metadata"),
            time: Some(1_700_000_000_300),
            version: Some(1.0),
            hostname: Some("example.com".to_string()),
            path: Some("/".to_string()),
            page_id: Some(page_id.to_string()),
            metadata: Some(metadata),
            consent: Some(true),
            ..Self::default()
        }
    }

    pub fn with_referrer(mut self, referrer: &str) -> Self {
        self.referrer = Some(referrer.to_string());
        self
    }

    pub fn with_metadata(mut self, metadata: Value) -> Self {
        self.metadata = Some(metadata);
        self
    }

    pub fn with_session(mut self, session_id: i64) -> Self {
        self.session_id = Some(session_id);
        self
    }

    pub fn with_consent(mut self, consent: bool) -> Self {
        self.consent = Some(consent);
        self
    }

    pub fn with_touch(mut self, touch: bool) -> Self {
        self.touch = Some(touch);
        self
    }

    pub fn with_event_metadata(mut self, metadata: Value) -> Self {
        self.metadata = Some(metadata);
        self
    }

    pub fn to_form(&self) -> String {
        let mut pairs: Vec<(String, String)> = Vec::new();
        push_opt(&mut pairs, "ty", self.hit_type);
        if let Some(time) = self.time {
            pairs.push(("t".to_string(), time.to_string()));
        }
        if let Some(version) = self.version {
            pairs.push(("v".to_string(), version.to_string()));
        }
        push_opt(&mut pairs, "h", self.hostname.as_deref());
        push_opt(&mut pairs, "p", self.path.as_deref());
        push_opt(&mut pairs, "pg", self.page_id.as_deref());
        if let Some(session_id) = self.session_id {
            pairs.push((
                "s".to_string(),
                revscope::util::encode_visitor_timestamp_id(session_id),
            ));
        }
        push_opt(&mut pairs, "r", self.referrer.as_deref());
        if let Some(duration) = self.duration {
            pairs.push(("d".to_string(), duration.to_string()));
        }
        if let Some(scrolled) = self.scrolled {
            pairs.push(("sc".to_string(), scrolled.to_string()));
        }
        if let Some(value) = self.viewport_width {
            pairs.push(("vw".to_string(), value.to_string()));
        }
        if let Some(value) = self.viewport_height {
            pairs.push(("vh".to_string(), value.to_string()));
        }
        if let Some(value) = self.screen_width {
            pairs.push(("sw".to_string(), value.to_string()));
        }
        if let Some(value) = self.screen_height {
            pairs.push(("sh".to_string(), value.to_string()));
        }
        push_opt(&mut pairs, "tz", self.timezone.as_deref());
        push_opt(&mut pairs, "l", self.language.as_deref());
        if let Some(mobile) = self.mobile {
            pairs.push(("m".to_string(), bool_str(mobile).to_string()));
        }
        if let Some(touch) = self.touch {
            pairs.push(("tch".to_string(), bool_str(touch).to_string()));
        }
        if let Some(metadata) = &self.metadata {
            pairs.push((
                "meta".to_string(),
                serde_json::to_string(metadata).expect("serialize metadata"),
            ));
        }
        push_opt(&mut pairs, "ev", self.event.as_deref());
        push_opt(&mut pairs, "e", self.error.as_deref());
        if let Some(consent) = self.consent {
            pairs.push(("c".to_string(), bool_str(consent).to_string()));
        }
        serde_urlencoded::to_string(pairs).expect("encode form body")
    }

    /// Item object for a batch envelope (shared fields stripped).
    pub fn to_batch_item(&self) -> Value {
        let mut map = serde_json::Map::new();
        if let Some(hit_type) = self.hit_type {
            map.insert("ty".to_string(), Value::String(hit_type.to_string()));
        }
        if let Some(time) = self.time {
            map.insert("t".to_string(), Value::Number(time.into()));
        }
        if let Some(path) = &self.path {
            map.insert("p".to_string(), Value::String(path.clone()));
        }
        if let Some(page_id) = &self.page_id {
            map.insert("pg".to_string(), Value::String(page_id.clone()));
        }
        if let Some(referrer) = &self.referrer {
            map.insert("r".to_string(), Value::String(referrer.clone()));
        }
        if let Some(duration) = self.duration {
            map.insert("d".to_string(), Value::Number(duration.into()));
        }
        if let Some(scrolled) = self.scrolled {
            map.insert("sc".to_string(), Value::Number(scrolled.into()));
        }
        if let Some(value) = self.viewport_width {
            map.insert("vw".to_string(), Value::Number(value.into()));
        }
        if let Some(value) = self.viewport_height {
            map.insert("vh".to_string(), Value::Number(value.into()));
        }
        if let Some(value) = self.screen_width {
            map.insert("sw".to_string(), Value::Number(value.into()));
        }
        if let Some(value) = self.screen_height {
            map.insert("sh".to_string(), Value::Number(value.into()));
        }
        if let Some(timezone) = &self.timezone {
            map.insert("tz".to_string(), Value::String(timezone.clone()));
        }
        if let Some(language) = &self.language {
            map.insert("l".to_string(), Value::String(language.clone()));
        }
        if let Some(mobile) = self.mobile {
            map.insert("m".to_string(), Value::Bool(mobile));
        }
        if let Some(touch) = self.touch {
            map.insert("tch".to_string(), Value::Bool(touch));
        }
        if let Some(metadata) = &self.metadata {
            map.insert("meta".to_string(), metadata.clone());
        }
        if let Some(event) = &self.event {
            map.insert("ev".to_string(), Value::String(event.clone()));
        }
        Value::Object(map)
    }
}

pub fn batch_form(_project_name: &str, items: &[TrackHit]) -> String {
    batch_envelope_form(items, None)
}

/// Batch envelope carrying the tracker's send time (`st`), which lets ingest
/// backdate each item by the age its own `t` reports.
pub fn batch_form_with_send_time(items: &[TrackHit], send_time_millis: i64) -> String {
    batch_envelope_form(items, Some(send_time_millis))
}

fn batch_envelope_form(items: &[TrackHit], send_time_millis: Option<i64>) -> String {
    let item_values: Vec<Value> = items.iter().map(TrackHit::to_batch_item).collect();
    let items_json = serde_json::to_string(&item_values).expect("serialize batch items");
    // No `l`: the tracker keeps language on the item, since only the gated
    // context hit is allowed to read `navigator.language`.
    let mut pairs = vec![
        ("v", "1".to_string()),
        ("h", "example.com".to_string()),
        ("c", "true".to_string()),
    ];
    if let Some(send_time) = send_time_millis {
        pairs.push(("st", send_time.to_string()));
    }
    pairs.push(("items", items_json));
    serde_urlencoded::to_string(pairs).expect("encode batch form")
}

pub fn page_id(n: u8) -> String {
    format!("{n:024x}")
}

pub fn count_rows(conn: &revscope::db::ProjectConnection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
        row.get::<_, i64>(0)
    })
    .expect("count rows")
}

pub fn query_i64(conn: &revscope::db::ProjectConnection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get::<_, i64>(0))
        .expect("query i64")
}

pub fn query_optional_i64(conn: &revscope::db::ProjectConnection, sql: &str) -> Option<i64> {
    conn.query_row(sql, [], |row| row.get::<_, Option<i64>>(0))
        .expect("query optional i64")
}

pub fn query_string(conn: &revscope::db::ProjectConnection, sql: &str) -> String {
    conn.query_row(sql, [], |row| row.get::<_, String>(0))
        .expect("query string")
}

pub fn query_optional_string(conn: &revscope::db::ProjectConnection, sql: &str) -> Option<String> {
    conn.query_row(sql, [], |row| row.get::<_, Option<String>>(0))
        .expect("query optional string")
}

async fn send_raw(addr: SocketAddr, request: &str) -> io::Result<HttpResponse> {
    let mut stream = TcpStream::connect(addr).await?;
    stream.write_all(request.as_bytes()).await?;
    stream.shutdown().await?;

    let mut raw = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut raw))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "response timed out"))??;

    Ok(parse_http_response(raw))
}

fn parse_http_response(raw: Vec<u8>) -> HttpResponse {
    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("response must contain header terminator");
    let header_bytes = &raw[..split];
    let body = raw[split + 4..].to_vec();
    let header_text = String::from_utf8_lossy(header_bytes);
    let mut lines = header_text.split("\r\n");
    let status_line = lines.next().unwrap_or("").to_string();
    let mut headers = FxHashMap::default();
    let mut set_cookies = Vec::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim().to_string();
            if name == "set-cookie" {
                set_cookies.push(value.clone());
            }
            headers.insert(name, value);
        }
    }
    HttpResponse {
        status_line,
        headers,
        set_cookies,
        body,
        raw,
    }
}

fn sanitize_label(label: &str) -> String {
    label
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect()
}

fn push_opt(pairs: &mut Vec<(String, String)>, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        pairs.push((key.to_string(), value.to_string()));
    }
}

fn bool_str(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}
