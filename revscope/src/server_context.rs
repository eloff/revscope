use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use dashmap::{DashMap, DashSet};
use parking_lot::{Condvar, Mutex};
use tokio::sync::{Notify, Semaphore};

use crate::acme::AcmeState;
use crate::connection_limit::{ConnectionLimit, ConnectionSlot, DEFAULT_MAX_CONNECTIONS};
use crate::db::main::MainDbCache;
use crate::db::project::{ProjectCache, ProjectCaches};
use crate::download_db::DownloadGrant;
use crate::download_snapshots::DownloadSnapshots;
use crate::ga_import::{GaEndpoints, GoogleOAuthClient};
use crate::http::CorsOrigins;
use crate::id_token::{APPLE, GOOGLE, Provider, Verifier};
use crate::ingest::IngestMap;
use crate::mcp::authorize::AuthorizationGrant;
use crate::mcp::token::{McpAccount, mcp_demo_token_from_env, parse_demo_token};
use crate::mcp::transport::{McpHttpService, build_mcp_service};
use crate::overview::{DEMO_OVERVIEW_CACHE_ENTRIES, DemoOverviewResponseCache};
use crate::query_slots::QuerySlots;
use crate::rate_limit::{AuthRateLimits, PublicWriteRateLimits};
use crate::request_slots::{PER_SESSION_SLOTS, REQUEST_SLOT_WAIT, RequestSlot, RequestSlots};
use crate::security_headers::HstsHosts;
use crate::session_cache::SessionCache;
use crate::tokens::{ExpiringTokens, SignedChallenges, TokenSecret};

/// Where per-project SQLite databases live for this server context.
#[derive(Clone, Debug)]
pub(crate) enum ProjectDbBackend {
    /// On-disk databases under `data_dir/{project_id}.db`.
    Filesystem { data_dir: PathBuf },
    /// Shared-cache in-memory databases keyed by `{namespace}-{project_id}`.
    /// Used by integration tests so each test gets an isolated DB without touching disk.
    InMemory { namespace: String },
}

/// Far more resets than real users start at once, since each finishes in about
/// a second; past it, new requests are dropped rather than queued.
const MAX_PASSWORD_RESET_SENDS: usize = 32;

pub struct ServerContext {
    project_db_backend: ProjectDbBackend,
    /// Per-project identity + SQLite pool, keyed by numeric project id.
    pub(crate) project_cache: ProjectCaches,
    /// Project ids whose SQLite databases have been explicitly created.
    /// Required for in-memory backends (shared-memory URIs cannot distinguish
    /// "missing" from "create empty"); also used to gate open vs create.
    pub(crate) created_project_dbs: Arc<DashSet<i32>>,
    /// Project ids deleted while this process ran. Ids are never reused, so
    /// this only grows by one per deletion. Code that still holds a deleted
    /// project's [`crate::db::ProjectCache`] (a running integration sync, a
    /// lookup that raced the delete) must not recreate its database or cache
    /// it again; see [`crate::db::project::delete_project_database`].
    pub(crate) deleted_project_ids: Arc<DashSet<i32>>,
    pub(crate) main_db_cache: Arc<MainDbCache>,
    pub(crate) cache_expiry_task_started: AtomicBool,
    /// Session users Postgres has confirmed, for [`crate::auth::session_user`].
    pub(crate) session_cache: SessionCache,
    /// Per-IP and per-email limits on login, signup and forgot password.
    pub(crate) auth_rate_limits: AuthRateLimits,
    /// Caps password resets being looked up and emailed in the background,
    /// which run after their request has released its request slot.
    pub(crate) password_reset_sends: Arc<Semaphore>,
    /// Live database download tokens, each granting one project key. Held in
    /// memory only: a restart just means the dashboard mints a new one.
    pub(crate) download_tokens: ExpiringTokens<DownloadGrant>,
    /// Consistent copies of project databases that downloads stream from.
    pub(crate) download_snapshots: DownloadSnapshots,
    /// Signs short-lived proofs that an anonymous demo visitor interacted with
    /// the Ask UI. Each is bound to the client IP it was minted for; a restart
    /// voids them and the browser simply mints another.
    pub(crate) demo_chat_challenges: SignedChallenges,
    /// Query tokens already checked against Postgres, each granting one project
    /// key. Purely a cache; the row in `api_tokens` is the source of truth.
    pub(crate) query_token_cache: ExpiringTokens<i32>,
    /// MCP access tokens already checked against Postgres, for the same reason
    /// as [`Self::query_token_cache`]. These grant an account rather than one
    /// project, so what is cached is the account they were minted for.
    pub(crate) mcp_token_cache: ExpiringTokens<McpAccount>,
    /// The MCP token that reads only the public demo. Set from
    /// `REVSCOPE_MCP_DEMO_TOKEN` in [`Self::new`]; `None` leaves the demo
    /// unreachable over MCP.
    pub(crate) mcp_demo_token: Option<TokenSecret>,
    /// OAuth authorization codes the consent page has minted but no client has
    /// redeemed yet. In memory only, and single use: they live for minutes, so a
    /// restart just sends the user through the consent page again.
    pub(crate) mcp_authorization_codes: ExpiringTokens<AuthorizationGrant>,
    /// The `rmcp` service behind `/v2/mcp`. Holds the tool schemas, so building
    /// it once keeps them out of the per-request path. It carries no reference
    /// back here: what a request may read travels in its http extensions.
    pub(crate) mcp_service: McpHttpService,
    /// Immutable project-name to [`ProjectCache`] mappings used by ingest.
    /// Entries intentionally never expire; they share Arcs with [`Self::project_cache`].
    pub(crate) project_name_cache: Arc<DashMap<String, Arc<ProjectCache>>>,
    /// Outbound HTTP to third-party APIs: payment providers, integrations,
    /// email and the chat gateway. Sharing it pools their connections, and its
    /// timeout keeps a stalled upstream from holding a sync or request open.
    pub(crate) http: reqwest::Client,
    #[cfg(feature = "billing")]
    pub(crate) billing: crate::billing::BillingState,
    /// Schema dump shared by every project. Initialized once: only a migration
    /// can change it, and that ships in a new binary (new process).
    pub(crate) schema_cache: OnceLock<Arc<str>>,
    /// Serialized demo overview JSON for the default (unfiltered) date ranges.
    pub(crate) demo_overview_cache: DemoOverviewResponseCache,
    /// Origins allowed to call the API with the session cookie from another
    /// site. Set from `REVSCOPE_CORS_ORIGINS` in [`Self::new`].
    pub(crate) cors_origins: CorsOrigins,
    /// Check Google and Apple sign-ins. `None` when `REVSCOPE_GOOGLE_CLIENT_ID`
    /// or `REVSCOPE_APPLE_CLIENT_ID` isn't set, which hides that button.
    pub(crate) google_sign_in: Option<Verifier>,
    pub(crate) apple_sign_in: Option<Verifier>,
    /// The web client that asks for Google Analytics access. `None` without
    /// `REVSCOPE_GOOGLE_CLIENT_ID` and `REVSCOPE_GOOGLE_CLIENT_SECRET`, which leaves
    /// an import only the service account route.
    pub(crate) google_oauth: Option<GoogleOAuthClient>,
    pub(crate) ga_endpoints: GaEndpoints,
    /// Skip the demo overview JSON cache. Set from `REVSCOPE_DISABLE_OVERVIEW_CACHE`
    /// (`1`/`true`/`yes`) so benchmarks measure query time instead of cache hits.
    pub(crate) overview_cache_disabled: bool,
    /// Answer plain HTTP only for Cloudflare and loopback peers (plus ACME
    /// HTTP-01 challenges), so the origin can't be reached around Cloudflare.
    /// Set from `REVSCOPE_REQUIRE_CLOUDFLARE` (`1`/`true`/`yes`).
    pub(crate) require_cloudflare: bool,
    /// Take the visitor IP from the last `X-Forwarded-For` value on requests
    /// from any peer but Cloudflare, for running behind a reverse proxy. Off by default since
    /// a client reaching the server directly could forge it. Set from
    /// `REVSCOPE_TRUST_X_FORWARDED_FOR` (`1`/`true`/`yes`).
    pub(crate) trust_x_forwarded_for: bool,
    /// Hosts that get `Strict-Transport-Security`. Set in [`Self::new`].
    pub(crate) hsts_hosts: HstsHosts,
    /// Per-project queues of pageviews/events/engagements awaiting a flush.
    pub(crate) ingest_map: IngestMap,
    /// Per-IP limits on tracker error reports and MCP client registration.
    pub(crate) public_write_rate_limits: PublicWriteRateLimits,
    /// In-flight HTTP-01 challenge responses and the certificates the TLS
    /// listener serves, keyed by SNI. Empty unless ACME is configured.
    pub(crate) acme: Arc<AcmeState>,
    /// Ensures the background ingest flush scheduler is spawned only once.
    pub(crate) flush_scheduler_started: AtomicBool,
    shutdown_requested: AtomicBool,
    shutdown_notify: Notify,
    shutdown_lock: Mutex<()>,
    shutdown_condvar: Condvar,
    active_tasks: AtomicUsize,
    /// Caps concurrent API and MCP work at `available_parallelism() * 4`, and
    /// one session's API work at [`PER_SESSION_SLOTS`].
    request_slots: RequestSlots,
    /// Caps concurrent ad-hoc SQL from query tokens, MCP and Ask AI, which the
    /// request slots alone would let one caller monopolize.
    pub(crate) ad_hoc_query_slots: QuerySlots,
    /// Caps open client connections, idle keep-alives included. Set from
    /// `REVSCOPE_MAX_CONNECTIONS` in [`Self::new`], default 100,000, and
    /// lowered to fit the open file limit.
    connection_limit: ConnectionLimit,
}

impl ServerContext {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        let mut context = Self::new_on_disk(data_dir);
        context.cors_origins = CorsOrigins::from_env();
        #[cfg(feature = "billing")]
        {
            context.billing = crate::billing::BillingState::from_env();
        }
        context.require_cloudflare = env_flag("REVSCOPE_REQUIRE_CLOUDFLARE");
        context.trust_x_forwarded_for = env_flag("REVSCOPE_TRUST_X_FORWARDED_FOR");
        context.hsts_hosts = HstsHosts::from_env();
        context.connection_limit = ConnectionLimit::from_env();
        context.overview_cache_disabled = env_flag("REVSCOPE_DISABLE_OVERVIEW_CACHE");
        context.google_sign_in = sign_in_verifier_from_env(&GOOGLE, "REVSCOPE_GOOGLE_CLIENT_ID");
        context.apple_sign_in = sign_in_verifier_from_env(&APPLE, "REVSCOPE_APPLE_CLIENT_ID");
        context.google_oauth = GoogleOAuthClient::from_env();
        context.mcp_demo_token = mcp_demo_token_from_env(|name| std::env::var(name).ok());
        if context.overview_cache_disabled {
            log::warn!("overview cache disabled via REVSCOPE_DISABLE_OVERVIEW_CACHE");
        }
        context
    }

    /// Project databases as files in `data_dir`, with the same fixed defaults as
    /// [`Self::new_in_memory`] rather than the `REVSCOPE_*` environment that
    /// [`Self::new`] reads, so tests don't depend on the developer's shell.
    pub fn new_on_disk(data_dir: impl Into<PathBuf>) -> Self {
        Self::with_backend(ProjectDbBackend::Filesystem {
            data_dir: data_dir.into(),
        })
    }

    /// Create a context whose project databases are shared-cache in-memory SQLite
    /// databases. `namespace` must be unique across concurrent tests in the same process.
    pub fn new_in_memory(namespace: impl Into<String>) -> Self {
        Self::with_backend(ProjectDbBackend::InMemory {
            namespace: namespace.into(),
        })
    }

    fn with_backend(project_db_backend: ProjectDbBackend) -> Self {
        Self {
            project_db_backend,
            project_cache: crate::db::project::new_project_caches(),
            created_project_dbs: Arc::new(DashSet::new()),
            deleted_project_ids: Arc::new(DashSet::new()),
            main_db_cache: Arc::new(crate::db::main::MainDbCache::new()),
            cache_expiry_task_started: AtomicBool::new(false),
            session_cache: SessionCache::new(),
            auth_rate_limits: AuthRateLimits::new(),
            password_reset_sends: Arc::new(Semaphore::new(MAX_PASSWORD_RESET_SENDS)),
            download_tokens: ExpiringTokens::new(),
            download_snapshots: DownloadSnapshots::new(),
            demo_chat_challenges: SignedChallenges::new()
                .expect("the OS random source is available at startup"),
            query_token_cache: ExpiringTokens::new(),
            mcp_token_cache: ExpiringTokens::new(),
            mcp_demo_token: None,
            mcp_authorization_codes: ExpiringTokens::new(),
            mcp_service: build_mcp_service(),
            project_name_cache: Arc::new(DashMap::new()),
            http: outbound_http_client(),
            #[cfg(feature = "billing")]
            billing: crate::billing::BillingState::new(),
            schema_cache: OnceLock::new(),
            demo_overview_cache: DemoOverviewResponseCache::new(DEMO_OVERVIEW_CACHE_ENTRIES),
            cors_origins: CorsOrigins::build_default(),
            google_sign_in: None,
            apple_sign_in: None,
            google_oauth: None,
            ga_endpoints: GaEndpoints::default(),
            overview_cache_disabled: false,
            require_cloudflare: false,
            trust_x_forwarded_for: false,
            hsts_hosts: HstsHosts::default(),
            ingest_map: IngestMap::new(),
            public_write_rate_limits: PublicWriteRateLimits::new(),
            acme: Arc::new(AcmeState::new()),
            flush_scheduler_started: AtomicBool::new(false),
            shutdown_requested: AtomicBool::new(false),
            shutdown_notify: Notify::new(),
            shutdown_lock: Mutex::new(()),
            shutdown_condvar: Condvar::new(),
            active_tasks: AtomicUsize::new(0),
            request_slots: RequestSlots::new(
                default_request_slot_count(),
                PER_SESSION_SLOTS,
                REQUEST_SLOT_WAIT,
            ),
            ad_hoc_query_slots: QuerySlots::with_default_limits(),
            connection_limit: ConnectionLimit::new(DEFAULT_MAX_CONNECTIONS),
        }
    }

    /// In-memory context with a fixed request-slot budget. Tests use this to
    /// exhaust the limiter without depending on host `available_parallelism`.
    /// The wait is shortened so timeout assertions do not take five seconds.
    #[cfg(test)]
    pub(crate) fn new_in_memory_with_request_slots(
        namespace: impl Into<String>,
        slots: usize,
    ) -> Self {
        let mut context = Self::with_backend(ProjectDbBackend::InMemory {
            namespace: namespace.into(),
        });
        context.request_slots =
            RequestSlots::new(slots, PER_SESSION_SLOTS, Duration::from_millis(50));
        context
    }

    /// The verifier for `provider` (`google` or `apple`), if it's configured.
    pub(crate) fn sign_in_verifier(&self, provider: &str) -> Option<&Verifier> {
        match provider {
            "google" => self.google_sign_in.as_ref(),
            "apple" => self.apple_sign_in.as_ref(),
            _ => None,
        }
    }

    /// Accepts Google and Apple ID tokens for `client_id` signed by
    /// `public_key_pem` under key id `kid`, without fetching the providers'
    /// keys. Public only so the integration tests can sign their own tokens.
    #[doc(hidden)]
    pub fn trust_sign_in_test_key(&mut self, client_id: &str, kid: &str, public_key_pem: &str) {
        let key = || {
            jsonwebtoken::DecodingKey::from_rsa_pem(public_key_pem.as_bytes())
                .expect("test key is an RSA public key in PEM")
        };
        self.google_sign_in = Some(Verifier::with_key(
            &GOOGLE,
            client_id.to_owned(),
            kid,
            key(),
        ));
        self.apple_sign_in = Some(Verifier::with_key(&APPLE, client_id.to_owned(), kid, key()));
    }

    /// Lets `token` read the public demo over MCP, when it is `rsm_` and 32
    /// hex characters; anything else leaves the demo unreachable.
    pub fn use_mcp_demo_token(&mut self, token: &str) {
        self.mcp_demo_token = parse_demo_token(token);
    }

    /// Sends Google Analytics import traffic to `base_url` instead of Google, and
    /// turns on the OAuth route. Public only so the integration tests can stub Google.
    #[doc(hidden)]
    pub fn use_ga_test_api(&mut self, base_url: &str) {
        self.google_oauth = Some(GoogleOAuthClient {
            client_id: "test-client".to_string(),
            client_secret: "test-secret".to_string(),
        });
        self.ga_endpoints = GaEndpoints {
            token_url: format!("{base_url}/token"),
            revoke_url: format!("{base_url}/revoke"),
            data_url: format!("{base_url}/data"),
            admin_url: format!("{base_url}/admin"),
        };
    }

    /// Wait up to [`REQUEST_SLOT_WAIT`] for an API/MCP concurrency slot, and
    /// first for one of the session's own slots when `session_key` is given.
    /// `None` means the caller should answer 503 with `Retry-After: 5`.
    pub(crate) async fn acquire_request_slot(
        &self,
        session_key: Option<u128>,
    ) -> Option<RequestSlot> {
        self.request_slots.acquire(session_key).await
    }

    /// Take a slot for a newly accepted connection, or `None` when the
    /// connection cap is reached and the connection should be closed.
    pub(crate) fn try_acquire_connection_slot(&self) -> Option<ConnectionSlot> {
        self.connection_limit.try_acquire()
    }

    /// An in-memory context that holds at most `max_connections` open.
    #[cfg(test)]
    pub(crate) fn new_in_memory_with_connection_limit(
        namespace: impl Into<String>,
        max_connections: usize,
    ) -> Self {
        let mut context = Self::new_in_memory(namespace);
        context.connection_limit = ConnectionLimit::new(max_connections);
        context
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn data_dir(&self) -> Option<&Path> {
        match &self.project_db_backend {
            ProjectDbBackend::Filesystem { data_dir } => Some(data_dir.as_path()),
            ProjectDbBackend::InMemory { .. } => None,
        }
    }

    pub(crate) fn project_db_backend(&self) -> &ProjectDbBackend {
        &self.project_db_backend
    }

    /// Insert a fully identified [`ProjectCache`] without a database lookup.
    /// Used when creating projects and by isolated integration-test contexts.
    pub fn cache_project(
        &self,
        project_id: i32,
        project_name: impl Into<String>,
        account_id: &str,
    ) -> Option<Arc<ProjectCache>> {
        let project_name = project_name.into();
        if !crate::projects::is_valid_project_name(&project_name) || project_id <= 0 {
            return None;
        }
        let account_key = crate::db::project::parse_uuid_key(account_id)?;
        Some(crate::db::project::insert_project(
            self,
            project_id,
            project_name,
            account_key,
        ))
    }

    pub fn request_shutdown(&self) {
        let first_shutdown_request = {
            let _guard = self.shutdown_lock.lock();
            let first_shutdown_request = !self.shutdown_requested.swap(true, Ordering::AcqRel);
            self.shutdown_condvar.notify_all();
            first_shutdown_request
        };

        if first_shutdown_request {
            self.main_db_cache.close_idle_connections();
        }
        self.shutdown_notify.notify_waiters();
    }

    pub fn shutdown_requested(&self) -> bool {
        self.shutdown_requested.load(Ordering::Acquire)
    }

    pub async fn wait_for_shutdown(&self) {
        loop {
            if self.shutdown_requested() {
                return;
            }

            let notified = self.shutdown_notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.shutdown_requested() {
                return;
            }
            notified.await;
        }
    }

    pub(crate) fn wait_for_shutdown_or_timeout(&self, timeout: Duration) -> bool {
        if self.shutdown_requested() {
            return true;
        }

        let mut guard = self.shutdown_lock.lock();
        if self.shutdown_requested() {
            return true;
        }
        self.shutdown_condvar.wait_for(&mut guard, timeout);
        self.shutdown_requested()
    }

    /// Whether `project_id` has hits queued that no flush has written yet.
    /// Takes the batch lock, so it waits out a flush in progress, and lets a
    /// caller wait for a background flush without opening the project database.
    pub fn has_queued_ingest(&self, project_id: i32) -> bool {
        self.ingest_map
            .get_existing(project_id)
            .is_some_and(|batch| batch.has_rows())
    }

    pub fn active_task_count(&self) -> usize {
        self.active_tasks.load(Ordering::Acquire)
    }

    pub(crate) fn active_task_started(&self) {
        self.active_tasks.fetch_add(1, Ordering::AcqRel);
    }

    pub(crate) fn active_task_finished(&self) {
        let previous = self.active_tasks.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0, "active task count underflow");
    }
}

/// Bounds each outbound call, body included. Calls that legitimately take
/// longer, like a streamed chat completion or a large report query, set their
/// own per request.
pub(crate) const OUTBOUND_HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const OUTBOUND_HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

fn outbound_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(OUTBOUND_HTTP_TIMEOUT)
        .connect_timeout(OUTBOUND_HTTP_CONNECT_TIMEOUT)
        .build()
        .expect("the TLS backend initializes at startup")
}

fn default_request_slot_count() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get().saturating_mul(4))
        .unwrap_or(16)
}

fn sign_in_verifier_from_env(provider: &'static Provider, name: &str) -> Option<Verifier> {
    let client_id = std::env::var(name).unwrap_or_default();
    let client_id = client_id.trim();
    (!client_id.is_empty()).then(|| Verifier::new(provider, client_id.to_owned()))
}

fn env_flag(name: &str) -> bool {
    matches!(
        std::env::var(name).unwrap_or_default().trim(),
        "1" | "true" | "yes"
    )
}
