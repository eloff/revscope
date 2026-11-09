use std::fmt;
use std::io;
use std::net::{IpAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tokio::io::AsyncWriteExt;
use tokio::sync::{Semaphore, watch};

use dashmap::DashMap;
use fxhash::FxHashMap;
use instant_acme::{
    Account, AccountCredentials, AuthorizationStatus, Challenge, ChallengeType, Identifier,
    LetsEncrypt, NewAccount, NewOrder, Order, OrderStatus, Problem, RetryPolicy,
};
use log::{error, info, warn};
use parking_lot::RwLock;
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;

use crate::db::open_main_database_read;
use crate::http::Response;
use crate::rate_limit::{
    CERTIFICATE_DNS_CHECKS_PER_ACCOUNT, CERTIFICATE_ORDERS_PER_ACCOUNT,
    CERTIFICATE_ORDERS_PER_DEPLOYMENT, RateLimiter,
};
use crate::server_context::ServerContext;
use crate::tls::{CertificateError, CertificateStore};
use crate::util::split_list;

/// Path prefix the ACME server fetches over plain HTTP to validate an HTTP-01
/// challenge, per RFC 8555 section 8.3.
const CHALLENGE_PATH_PREFIX: &str = "/.well-known/acme-challenge/";

/// Let's Encrypt hits origin directly, so this path is allowed from peers that
/// are not Cloudflare even with `REVSCOPE_REQUIRE_CLOUDFLARE`. Query strings are ignored, matching [`challenge_response`].
pub(crate) fn is_acme_challenge_path(target: &str) -> bool {
    acme_challenge_token(target).is_some()
}

fn acme_challenge_token(target: &str) -> Option<&str> {
    let path = target.split('?').next().unwrap_or(target);
    path.strip_prefix(CHALLENGE_PATH_PREFIX)
}

const ACCOUNT_CREDENTIALS_FILE: &str = "account.json";
const CERTIFICATE_FILE: &str = "certificate.pem";
const PRIVATE_KEY_FILE: &str = "private-key.pem";
const PLATFORM_CACHE_DIR: &str = "platform";
const CUSTOM_CACHE_DIR: &str = "custom";
/// Staging keeps its own account and certificates under this subdirectory, so
/// switching back to production never reuses a staging account or serves its
/// untrusted certificates.
const STAGING_CACHE_DIR: &str = "staging";
const PLATFORM_ORDER_KEY: &str = "platform";
const DEPLOYMENT_BUDGET_KEY: &str = "deployment";

/// Hostnames certified when `REVSCOPE_TLS_DOMAINS` is unset: the hosted
/// service's, where `eu1` is the CNAME target customers point at. A self-hosted
/// instance has no sensible default, so it must name its own.
const DEFAULT_PLATFORM_DOMAINS: &[&str] = if cfg!(feature = "self-hosted") {
    &[]
} else {
    &["eu1.revscope.co", "api.revscope.co"]
};

/// Order a replacement once the certificate has this little validity left.
/// Let's Encrypt issues 90 day certificates, so this leaves a month of
/// retries before an expiry would take HTTPS down.
const RENEW_BEFORE_EXPIRY: Duration = Duration::from_secs(30 * 24 * 60 * 60);
/// How long to wait between checks when nothing needs following up soon.
const RENEWAL_CHECK_INTERVAL: Duration = Duration::from_secs(12 * 60 * 60);
/// How long to wait after starting an order, and the cooldown before another
/// order after one fails, so a misconfigured domain cannot spend the ACME rate
/// limit in a loop.
const RENEWAL_RETRY_INTERVAL: Duration = Duration::from_secs(15 * 60);
/// How soon the renewal task retries a failed platform order. Every customer
/// depends on this certificate, so it doesn't wait as long as a custom domain.
/// Let's Encrypt pauses a hostname after 1,152 consecutive failed validations,
/// and this pace stays under that for the whole [`RENEW_BEFORE_EXPIRY`] window.
const PLATFORM_RENEWAL_RETRY_INTERVAL: Duration = Duration::from_secs(60 * 60);
/// How long a TLS handshake may wait for an in-flight HTTP-01 order.
/// Kept under typical browser TLS timeouts. A settings save starts issuance when
/// the domain already points here, and then this is only the remainder of the order.
const ISSUANCE_WAIT: Duration = Duration::from_secs(25);
/// How many handshakes may wait for certificates at once, across all orders.
/// Nothing watches a waiting client's socket, so without a limit a client could
/// send a ClientHello and hang up, over and over, and fill every connection slot
/// without keeping a socket open. This only protects a connection cap larger than
/// itself, as the default is.
const MAX_WAITING_HANDSHAKES: usize = 1024;
/// How long to poll Let's Encrypt for one order to become ready, and again for
/// the certificate after finalize. Handshake waiters use [`ISSUANCE_WAIT`] and
/// return sooner; this is the background order, not the first connection.
const ORDER_WAIT: Duration = Duration::from_secs(5 * 60);
/// Limit on a whole order. instant-acme sets no timeout on its requests, so a
/// stalled connection would otherwise keep the order in flight until a restart,
/// and every later check would join it instead of retrying.
const ORDER_DEADLINE: Duration = Duration::from_secs(15 * 60);
/// How often to reread which custom domains are active and start any orders that
/// makes due. Billing changes plans without telling this module, so this is how
/// soon a change takes effect.
const CUSTOM_DOMAIN_REFRESH_INTERVAL: Duration = Duration::from_secs(5 * 60);
/// How long to wait for DNS when checking that a custom domain points here.
const DNS_CHECK_TIMEOUT: Duration = Duration::from_secs(10);
/// Concurrent system DNS lookups for those checks. Each holds a thread of the
/// pool API handlers run on, so this stays far below that pool's 512. Checks wait
/// for a place rather than give up, or a renewal pass would lose most of its
/// domains for [`RENEWAL_CHECK_INTERVAL`].
const MAX_DNS_LOOKUPS: usize = 32;
/// How soon a save or handshake may retry a custom domain whose order was held
/// back. That costs nothing at Let's Encrypt, so someone who has just added their
/// CNAME shouldn't wait out [`RENEWAL_RETRY_INTERVAL`].
const HELD_BACK_RECHECK_INTERVAL: Duration = Duration::from_secs(60);

/// Everything needed to obtain certificates for this deployment. Absent when
/// the deployment terminates TLS elsewhere, which leaves the HTTPS listener off.
#[derive(Clone, Debug)]
pub struct AcmeConfig {
    /// Contacts registered with the ACME account, already `mailto:`-prefixed.
    contacts: Vec<String>,
    directory_url: String,
    /// Holds the account credentials and the issued certificates, so a restart
    /// reuses them instead of ordering again.
    cache_dir: PathBuf,
    /// Hostnames this instance always certifies, lowercase and never empty.
    /// The first is the CNAME target for custom domains and the certificate
    /// served when a handshake has no SNI.
    platform_domains: Vec<String>,
}

impl AcmeConfig {
    /// Reads the ACME settings from the environment. Used when
    /// `REVSCOPE_LISTEN_TLS` is set, which is what turns HTTPS on.
    ///
    /// - `REVSCOPE_ACME_CONTACT`: comma-separated contacts for the account,
    ///   which is how Let's Encrypt reports expiry problems. Optional.
    /// - `REVSCOPE_ACME_STAGING`: order from the Let's Encrypt staging
    ///   environment, whose certificates are untrusted but rate limits are
    ///   loose enough to test a deployment with. Its cache goes in a `staging`
    ///   subdirectory of the ACME dir.
    /// - `REVSCOPE_ACME_DIR`: where to keep the account credentials and the
    ///   issued certificates. Defaults to `data/acme`.
    /// - `REVSCOPE_TLS_DOMAINS`: comma-separated hostnames to certify, the
    ///   first being the default for handshakes without SNI. Defaults to the
    ///   hosted service's `eu1.revscope.co,api.revscope.co`, except in a
    ///   self-hosted build, which fails without it.
    pub fn from_env() -> Result<Self, String> {
        let contacts = std::env::var("REVSCOPE_ACME_CONTACT")
            .map(|value| split_list(&value).into_iter().map(contact_uri).collect())
            .unwrap_or_default();
        let staging = env_flag("REVSCOPE_ACME_STAGING");
        let directory_url = if staging {
            LetsEncrypt::Staging.url()
        } else {
            LetsEncrypt::Production.url()
        };
        let acme_dir = std::env::var("REVSCOPE_ACME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| Path::new(crate::db::DATA_DIR).join("acme"));
        let cache_dir = cache_dir_for(acme_dir, staging);
        let platform_domains =
            required_platform_domains(&std::env::var("REVSCOPE_TLS_DOMAINS").unwrap_or_default())?;

        Ok(Self {
            contacts,
            directory_url: directory_url.to_string(),
            cache_dir,
            platform_domains,
        })
    }

    /// The hostnames this instance certifies, the default SNI first.
    pub fn platform_domains(&self) -> &[String] {
        &self.platform_domains
    }
}

/// The hostnames to certify, or an error when there are none, as in a
/// self-hosted build that leaves `REVSCOPE_TLS_DOMAINS` unset.
fn required_platform_domains(value: &str) -> Result<Vec<String>, String> {
    let domains = platform_domains_from(value);
    if domains.is_empty() {
        return Err(
            "REVSCOPE_TLS_DOMAINS must list this instance's hostnames when REVSCOPE_LISTEN_TLS is set"
                .to_string(),
        );
    }
    Ok(domains)
}

/// Parses `REVSCOPE_TLS_DOMAINS`, falling back to `DEFAULT_PLATFORM_DOMAINS`
/// when it lists none.
fn platform_domains_from(value: &str) -> Vec<String> {
    let mut domains: Vec<String> = Vec::new();
    for domain in split_list(value) {
        let domain = normalize_sni(&domain);
        if !domain.is_empty() && !domains.contains(&domain) {
            domains.push(domain);
        }
    }
    if domains.is_empty() {
        domains = DEFAULT_PLATFORM_DOMAINS
            .iter()
            .map(|domain| (*domain).to_string())
            .collect();
    }
    domains
}

fn cache_dir_for(acme_dir: PathBuf, staging: bool) -> PathBuf {
    if staging {
        acme_dir.join(STAGING_CACHE_DIR)
    } else {
        acme_dir
    }
}

/// ACME contacts are URIs; a bare address is assumed to be an email one.
fn contact_uri(contact: String) -> String {
    if contact.contains(':') {
        contact
    } else {
        format!("mailto:{contact}")
    }
}

fn env_flag(name: &str) -> bool {
    matches!(
        std::env::var(name).unwrap_or_default().trim(),
        "1" | "true" | "yes"
    )
}

pub(crate) fn is_revscope_hostname(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == "revscope.co" || lower.ends_with(".revscope.co")
}

fn normalize_sni(name: &str) -> String {
    name.trim().trim_end_matches('.').to_ascii_lowercase()
}

/// State shared by the HTTP-01 challenge route, the TLS listener, and the
/// renewal task.
pub(crate) struct AcmeState {
    /// Challenge responses the ACME server may fetch right now, keyed by token.
    /// Only populated while an order is in flight.
    challenges: DashMap<String, String>,
    certificates: Arc<CertificateStore>,
    /// Customer CNAME hostnames currently stored in `project_settings`, each
    /// with the account that owns its project.
    custom_domains: DashMap<String, String>,
    /// Budgeted custom-domain orders per account, so one account can't use up
    /// [`Self::deployment_orders`].
    account_orders: RateLimiter,
    /// Budgeted custom-domain orders in total, because Let's Encrypt's new-order
    /// limit covers this whole deployment.
    deployment_orders: RateLimiter,
    /// In-flight ACME orders, keyed by [`PLATFORM_ORDER_KEY`] or the custom
    /// hostname, so concurrent handshakes wait on one order.
    in_flight: DashMap<String, Arc<IssuanceWaiter>>,
    /// Places for handshakes to wait on an order, [`MAX_WAITING_HANDSHAKES`] in all.
    handshake_waits: Semaphore,
    /// Last failed order per key, so a broken CNAME cannot retry on every
    /// handshake.
    last_failure: DashMap<String, Instant>,
    /// When each custom domain's order last stopped before reaching the ACME
    /// server. Separate from [`Self::last_failure`] because these cost nothing
    /// there, and a held-back renewal hasn't had its first attempt yet.
    held_back: DashMap<String, Instant>,
    /// Places for [`MAX_DNS_LOOKUPS`] lookups to run.
    dns_lookups: Arc<Semaphore>,
    /// DNS checks that an account can bring about at will, so that it can't keep
    /// every lookup place busy and delay other customers' orders.
    account_dns_checks: RateLimiter,
    config: RwLock<Option<AcmeConfig>>,
    #[cfg(test)]
    test_addresses: DashMap<String, Vec<IpAddr>>,
}

impl AcmeState {
    pub(crate) fn new() -> Self {
        Self {
            challenges: DashMap::new(),
            certificates: Arc::new(CertificateStore::new()),
            custom_domains: DashMap::new(),
            account_orders: RateLimiter::new(CERTIFICATE_ORDERS_PER_ACCOUNT),
            deployment_orders: RateLimiter::new(CERTIFICATE_ORDERS_PER_DEPLOYMENT),
            in_flight: DashMap::new(),
            handshake_waits: Semaphore::new(MAX_WAITING_HANDSHAKES),
            last_failure: DashMap::new(),
            held_back: DashMap::new(),
            dns_lookups: Arc::new(Semaphore::new(MAX_DNS_LOOKUPS)),
            account_dns_checks: RateLimiter::new(CERTIFICATE_DNS_CHECKS_PER_ACCOUNT),
            config: RwLock::new(None),
            #[cfg(test)]
            test_addresses: DashMap::new(),
        }
    }

    pub(crate) fn certificates(&self) -> &Arc<CertificateStore> {
        &self.certificates
    }

    pub(crate) fn set_config(&self, config: AcmeConfig) {
        *self.config.write() = Some(config);
    }

    pub(crate) fn replace_custom_domain(
        &self,
        previous: Option<&str>,
        next: Option<&str>,
        account_id: &str,
    ) {
        if let Some(previous) = previous {
            self.custom_domains.remove(previous);
            if next != Some(previous) {
                self.held_back.remove(previous);
            }
        }
        if let Some(next) = next {
            self.custom_domains
                .insert(next.to_string(), account_id.to_string());
        }
    }

    pub(crate) fn set_custom_domains(&self, domains: impl IntoIterator<Item = (String, String)>) {
        // Not cleared first, since a handshake in the gap would find its domain gone.
        let domains: FxHashMap<String, String> = domains.into_iter().collect();
        self.custom_domains
            .retain(|domain, _| domains.contains_key(domain));
        for (domain, account_id) in domains {
            self.custom_domains.insert(domain, account_id);
        }
    }

    /// The configured platform hostnames, or none before ACME is configured.
    fn platform_domains(&self) -> Vec<String> {
        self.config
            .read()
            .as_ref()
            .map(|config| config.platform_domains.clone())
            .unwrap_or_default()
    }

    fn is_platform_domain(&self, name: &str) -> bool {
        self.config.read().as_ref().is_some_and(|config| {
            config
                .platform_domains
                .iter()
                .any(|domain| domain.eq_ignore_ascii_case(name))
        })
    }

    fn is_allowed(&self, name: &str) -> bool {
        self.is_platform_domain(name) || self.custom_domains.contains_key(name)
    }

    fn order_key(&self, name: &str) -> String {
        if self.is_platform_domain(name) {
            PLATFORM_ORDER_KEY.to_string()
        } else {
            name.to_string()
        }
    }

    fn domains_for(&self, name: &str) -> Vec<String> {
        if self.is_platform_domain(name) {
            self.platform_domains()
        } else {
            vec![name.to_string()]
        }
    }

    fn cache_subdir(&self, name: &str) -> PathBuf {
        if self.is_platform_domain(name) {
            PathBuf::from(PLATFORM_CACHE_DIR)
        } else {
            Path::new(CUSTOM_CACHE_DIR).join(name)
        }
    }

    /// The hostname a handshake is for. Missing SNI uses the first platform
    /// domain.
    fn handshake_name(&self, server_name: Option<&str>) -> Option<String> {
        match server_name
            .map(normalize_sni)
            .filter(|name| !name.is_empty())
        {
            Some(name) => Some(name),
            None => self.platform_domains().into_iter().next(),
        }
    }

    /// Picks the certificate for this handshake. rustls calls this on a runtime
    /// worker, so it only looks one up; [`wait_for_handshake_certificate`] does
    /// any waiting first.
    pub(crate) fn resolve_sni(&self, server_name: Option<&str>) -> Option<Arc<CertifiedKey>> {
        self.certificates
            .certified_key(&self.handshake_name(server_name)?)
    }

    /// Only for a domain still saved, so that cycling through hostnames can't
    /// grow this map.
    fn hold_back(&self, domain: &str) {
        if self.custom_domains.contains_key(domain) {
            self.held_back.insert(domain.to_string(), Instant::now());
        }
    }

    fn held_back_within(&self, domain: &str, interval: Duration) -> bool {
        self.held_back
            .get(domain)
            .is_some_and(|held_back_at| held_back_at.elapsed() < interval)
    }

    /// Charges the account first, so an account over its limit can't drain the
    /// shared budget.
    fn spend_order_budget(&self, account_id: &str) -> Result<(), AcmeError> {
        self.account_orders
            .check(account_id)
            .map_err(|_| AcmeError::OrderLimit)?;
        self.deployment_orders
            .check(DEPLOYMENT_BUDGET_KEY)
            .map_err(|_| {
                self.account_orders.refund(account_id);
                AcmeError::OrderLimit
            })
    }

    async fn resolve(&self, name: &str) -> Result<Vec<IpAddr>, AcmeError> {
        let place = self
            .dns_lookups
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| AcmeError::DomainNotReady("DNS checks are shut down".to_string()))?;
        #[cfg(test)]
        if let Some(addresses) = self.test_addresses.get(name) {
            return Ok(addresses.clone());
        }
        let host = (name.to_string(), 80);
        // The system resolver can't be cancelled, so the thread keeps its place
        // until the lookup returns, even after the timeout below gives up on it.
        let lookup = tokio::task::spawn_blocking(move || {
            let _place = place;
            host.to_socket_addrs()
                .map(|addresses| addresses.collect::<Vec<_>>())
        });
        let addresses = tokio::time::timeout(DNS_CHECK_TIMEOUT, lookup)
            .await
            .map_err(|_| AcmeError::DomainNotReady(format!("timed out resolving {name}")))?
            .map_err(|error| {
                AcmeError::DomainNotReady(format!("unable to resolve {name}: {error}"))
            })?
            .map_err(|error| {
                AcmeError::DomainNotReady(format!("unable to resolve {name}: {error}"))
            })?;
        Ok(addresses.iter().map(|address| address.ip()).collect())
    }

    /// Compares addresses rather than reading the CNAME so that a chain of
    /// CNAMEs works, and because what HTTP-01 needs is to reach this server.
    async fn check_points_here(&self, domain: &str, target: &str) -> Result<(), AcmeError> {
        let (domain_addresses, target_addresses) =
            tokio::join!(self.resolve(domain), self.resolve(target));
        let (domain_addresses, target_addresses) = (domain_addresses?, target_addresses?);
        let points_here = domain_addresses.iter().any(|address| {
            target_addresses
                .iter()
                .any(|target| target.to_canonical() == address.to_canonical())
        });
        if points_here {
            Ok(())
        } else {
            Err(AcmeError::DomainNotReady(format!(
                "it does not resolve to the same address as {target}"
            )))
        }
    }

    /// Records a running order for `name`, without contacting an ACME server.
    #[cfg(test)]
    pub(crate) fn start_test_order(&self, name: &str) -> Arc<IssuanceWaiter> {
        self.custom_domains
            .insert(name.to_string(), "test-account".to_string());
        let waiter = Arc::new(IssuanceWaiter::new());
        self.in_flight.insert(name.to_string(), waiter.clone());
        waiter
    }
}

/// Lets handshakes wait while an ACME order for their name is running.
pub(crate) struct IssuanceWaiter {
    finished: watch::Sender<bool>,
}

impl IssuanceWaiter {
    fn new() -> Self {
        Self {
            finished: watch::Sender::new(false),
        }
    }

    async fn wait_timeout(&self, timeout: Duration) -> bool {
        let mut finished = self.finished.subscribe();
        matches!(
            tokio::time::timeout(timeout, finished.wait_for(|finished| *finished)).await,
            Ok(Ok(_))
        )
    }

    pub(crate) fn finish(&self) {
        self.finished.send_replace(true);
    }

    #[cfg(test)]
    pub(crate) fn has_waiters(&self) -> bool {
        self.finished.receiver_count() > 0
    }
}

struct SniCertificateResolver {
    server_context: Arc<ServerContext>,
}

impl fmt::Debug for SniCertificateResolver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SniCertificateResolver")
            .finish_non_exhaustive()
    }
}

impl ResolvesServerCert for SniCertificateResolver {
    fn resolve(&self, client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        self.server_context
            .acme
            .resolve_sni(client_hello.server_name())
    }
}

pub(crate) fn sni_resolver(server_context: Arc<ServerContext>) -> Arc<dyn ResolvesServerCert> {
    Arc::new(SniCertificateResolver { server_context })
}

/// Answers the plain-HTTP validation request the ACME server sends while an
/// order is in flight. `None` means the target is not a challenge path, so the
/// caller should carry on with normal routing.
pub(crate) fn challenge_response(server_context: &ServerContext, target: &str) -> Option<Response> {
    let token = acme_challenge_token(target)?;
    let Some(key_authorization) = server_context.acme.challenges.get(token) else {
        return Some(Response::not_found());
    };
    Some(Response::with_content_type(
        "200 OK",
        "text/plain; charset=utf-8",
        key_authorization.value().clone(),
        &[],
    ))
}

/// Loads cached certificates and the custom-domain allowlist so the first
/// handshake after a restart can succeed before the renewal task runs.
pub async fn prepare_acme(server_context: Arc<ServerContext>, config: &AcmeConfig) {
    server_context.acme.set_config(config.clone());
    let saved = match load_custom_domains(server_context.clone()).await {
        Ok(saved) => saved,
        Err(error) => {
            warn!("unable to load custom ACME domains: {error}");
            Vec::new()
        }
    };
    if !saved.is_empty() {
        info!(
            "loaded {} custom ACME domain(s) from project settings",
            saved.len()
        );
    }
    let names: Vec<String> = saved.iter().map(|domain| domain.name.clone()).collect();
    server_context
        .acme
        .set_custom_domains(active_domains(saved));
    load_cached_certificates(&server_context, config, &names);
}

/// Installs any cached certificate, then starts the task that orders one over
/// ACME HTTP-01 and renews it before it expires.
pub fn start_acme_task(server_context: Arc<ServerContext>, config: AcmeConfig) {
    server_context.acme.set_config(config);
    crate::task::spawn(
        server_context.clone(),
        custom_domain_refresh_loop(server_context.clone()),
    );
    crate::task::spawn(server_context.clone(), renewal_loop(server_context));
}

/// Starts an ACME order for `domain` if one is not already running. Used when
/// a project saves a custom domain, so that if its DNS is already in place the
/// first visitor does not have to wait out the whole issuance.
pub(crate) fn request_issuance(server_context: Arc<ServerContext>, domain: &str) {
    if server_context.acme.config.read().is_none() {
        return;
    }
    let _ = order_if_missing(&server_context, domain);
}

/// Makes a just-saved domain active, or leaves it inactive when the plan doesn't
/// include one.
pub(crate) fn apply_saved_custom_domain(
    server_context: &Arc<ServerContext>,
    previous: Option<&str>,
    next: Option<&str>,
    plan_allows: bool,
    account_id: &str,
) {
    let active = next.filter(|_| plan_allows);
    server_context
        .acme
        .replace_custom_domain(previous, active, account_id);
    if let Some(domain) = active {
        request_issuance(server_context.clone(), domain);
    }
}

/// If the handshake's name is allowed but has no certificate yet, starts or
/// joins its order and waits up to [`ISSUANCE_WAIT`] for HTTP-01 to complete,
/// unless [`MAX_WAITING_HANDSHAKES`] are already waiting.
/// This has to run before rustls takes over, because its certificate resolver
/// is synchronous and waiting there would stall a runtime worker.
pub(crate) async fn wait_for_handshake_certificate(
    server_context: &Arc<ServerContext>,
    server_name: Option<&str>,
) {
    let acme = &server_context.acme;
    let Some(name) = acme.handshake_name(server_name) else {
        return;
    };
    if acme.certificates.certified_key(&name).is_some() || !acme.is_allowed(&name) {
        return;
    }
    let order_key = acme.order_key(&name);
    if let Some(failed_at) = acme.last_failure.get(&order_key)
        && acme.in_flight.get(&order_key).is_none()
        && failed_at.elapsed() < RENEWAL_RETRY_INTERVAL
    {
        return;
    }
    let Some(waiter) = order_if_missing(server_context, &name) else {
        return;
    };
    let Ok(_place) = acme.handshake_waits.try_acquire() else {
        info!("too many handshakes are waiting for certificates; not waiting for {name}");
        return;
    };
    waiter.wait_timeout(ISSUANCE_WAIT).await;
}

/// For callers that only need a certificate to exist. Renewal must replace an
/// installed one, so it calls [`start_or_join_order`] directly.
fn order_if_missing(
    server_context: &Arc<ServerContext>,
    name: &str,
) -> Option<Arc<IssuanceWaiter>> {
    if server_context
        .acme
        .certificates
        .certified_key(name)
        .is_some()
    {
        return None;
    }
    start_or_join_order(server_context, name)
}

fn start_or_join_order(
    server_context: &Arc<ServerContext>,
    name: &str,
) -> Option<Arc<IssuanceWaiter>> {
    let order_key = server_context.acme.order_key(name);
    // Join an order already running, including tests that insert a waiter
    // without an ACME config.
    if let Some(waiter) = server_context.acme.in_flight.get(&order_key) {
        return Some(waiter.clone());
    }
    let config = server_context.acme.config.read().clone()?;
    if server_context
        .acme
        .held_back_within(&order_key, HELD_BACK_RECHECK_INTERVAL)
    {
        return None;
    }
    if let Some(failed_at) = server_context.acme.last_failure.get(&order_key)
        && failed_at.elapsed() < RENEWAL_RETRY_INTERVAL
    {
        return None;
    }
    match server_context.acme.in_flight.entry(order_key.clone()) {
        dashmap::mapref::entry::Entry::Occupied(entry) => Some(entry.get().clone()),
        dashmap::mapref::entry::Entry::Vacant(entry) => {
            let waiter = Arc::new(IssuanceWaiter::new());
            entry.insert(waiter.clone());
            let domains = server_context.acme.domains_for(name);
            let cache_subdir = server_context.acme.cache_subdir(name);
            let server_context = server_context.clone();
            crate::task::spawn(
                server_context.clone(),
                run_order(server_context, config, order_key, domains, cache_subdir),
            );
            Some(waiter)
        }
    }
}

async fn run_order(
    server_context: Arc<ServerContext>,
    config: AcmeConfig,
    order_key: String,
    domains: Vec<String>,
    cache_subdir: PathBuf,
) {
    let names = domains.join(", ");
    info!("ordering a certificate for {names}");
    let outcome = tokio::select! {
        _ = server_context.wait_for_shutdown() => {
            finish_order(&server_context, &order_key, false);
            return;
        }
        outcome = tokio::time::timeout(
            ORDER_DEADLINE,
            order_certificate(&server_context, &config, &order_key, &domains, &cache_subdir),
        ) => outcome.unwrap_or(Err(AcmeError::Timeout)),
    };
    match outcome {
        Ok(not_after) => {
            server_context.acme.last_failure.remove(&order_key);
            info!(
                "issued certificate for {names}, expiring in {}",
                format_remaining(not_after)
            );
            finish_order(&server_context, &order_key, true);
        }
        Err(AcmeError::Timeout) => {
            // Without a certificate, the next handshake may start a new order
            // rather than fail. A renewal has no one waiting, and each order that
            // finishes late counts toward Let's Encrypt's weekly duplicate limit.
            if has_installed_certificate(&server_context, &domains) {
                server_context
                    .acme
                    .last_failure
                    .insert(order_key.clone(), Instant::now());
            }
            warn!("ACME order for {names} timed out; will retry");
            finish_order(&server_context, &order_key, false);
        }
        Err(AcmeError::DomainNotReady(reason)) => {
            server_context.acme.hold_back(&order_key);
            info!("not ordering a certificate for {names}: {reason}");
            finish_order(&server_context, &order_key, false);
        }
        Err(AcmeError::OrderLimit) => {
            server_context.acme.hold_back(&order_key);
            warn!("not ordering a certificate for {names}: over the certificate order limit");
            finish_order(&server_context, &order_key, false);
        }
        Err(error) => {
            server_context
                .acme
                .last_failure
                .insert(order_key.clone(), Instant::now());
            error!("ACME order for {names} failed: {error}");
            finish_order(&server_context, &order_key, false);
        }
    }
}

fn has_installed_certificate(server_context: &ServerContext, domains: &[String]) -> bool {
    domains.iter().any(|domain| {
        server_context
            .acme
            .certificates
            .certified_key(domain)
            .is_some()
    })
}

async fn order_certificate(
    server_context: &ServerContext,
    config: &AcmeConfig,
    order_key: &str,
    domains: &[String],
    cache_subdir: &Path,
) -> Result<SystemTime, AcmeError> {
    if order_key != PLATFORM_ORDER_KEY {
        admit_custom_domain_order(server_context, config, order_key, domains).await?;
        server_context.acme.held_back.remove(order_key);
    }
    provision_certificate(server_context, config, domains, cache_subdir).await
}

/// Runs for every custom-domain order, because a handshake or the renewal task
/// can order a saved domain just as a settings save can. DNS is checked first
/// so that a domain not pointed here yet doesn't spend its account's budget.
async fn admit_custom_domain_order(
    server_context: &ServerContext,
    config: &AcmeConfig,
    domain: &str,
    domains: &[String],
) -> Result<(), AcmeError> {
    let acme = &server_context.acme;
    let target = config.platform_domains.first().ok_or_else(|| {
        AcmeError::DomainNotReady("no platform hostname is configured".to_string())
    })?;
    let no_longer_saved =
        || AcmeError::DomainNotReady("it is no longer a custom domain".to_string());
    let account_id = acme
        .custom_domains
        .get(domain)
        .map(|entry| entry.value().clone())
        .ok_or_else(no_longer_saved)?;
    // A renewal's first attempt comes up once per certificate until a restart, and
    // refusing it risks an outage. Its retries are charged, since a customer can
    // make every attempt fail on purpose, such as with a CAA record.
    let charged = !has_installed_certificate(server_context, domains)
        || acme.last_failure.contains_key(domain);
    if charged && acme.account_dns_checks.check(&account_id).is_err() {
        return Err(AcmeError::DomainNotReady(
            "its account has run too many DNS checks".to_string(),
        ));
    }
    acme.check_points_here(domain, target).await?;
    // The check can wait a while for a place, and the domain may have gone since.
    if !acme.custom_domains.contains_key(domain) {
        return Err(no_longer_saved());
    }
    if charged {
        acme.spend_order_budget(&account_id)?;
    }
    Ok(())
}

fn finish_order(server_context: &ServerContext, order_key: &str, _succeeded: bool) {
    if let Some((_, waiter)) = server_context.acme.in_flight.remove(order_key) {
        waiter.finish();
    }
}

async fn refresh_custom_domains(server_context: &Arc<ServerContext>) {
    match load_custom_domains(server_context.clone()).await {
        Ok(saved) => {
            let acme = &server_context.acme;
            acme.set_custom_domains(active_domains(saved));
            acme.held_back
                .retain(|domain, _| acme.custom_domains.contains_key(domain));
        }
        Err(error) => warn!("unable to refresh custom ACME domains: {error}"),
    }
}

async fn custom_domain_refresh_loop(server_context: Arc<ServerContext>) {
    loop {
        tokio::select! {
            _ = server_context.wait_for_shutdown() => return,
            _ = tokio::time::sleep(CUSTOM_DOMAIN_REFRESH_INTERVAL) => {}
        }
        refresh_custom_domains(&server_context).await;
        // A handshake only orders when no certificate is installed, so a domain
        // whose certificate expired while its plan didn't cover it needs this.
        start_due_orders(&server_context);
    }
}

async fn renewal_loop(server_context: Arc<ServerContext>) {
    loop {
        refresh_custom_domains(&server_context).await;

        let wait = if start_due_orders(&server_context) {
            RENEWAL_RETRY_INTERVAL
        } else {
            RENEWAL_CHECK_INTERVAL
        };

        tokio::select! {
            _ = server_context.wait_for_shutdown() => return,
            _ = tokio::time::sleep(wait) => {}
        }
    }
}

/// Starts or joins an order for every certificate that is due or missing, and
/// returns whether to check again after [`RENEWAL_RETRY_INTERVAL`].
fn start_due_orders(server_context: &Arc<ServerContext>) -> bool {
    let mut check_again_soon = false;
    for (names, not_after) in server_context.acme.certificates.unique_certificates() {
        let Some(reason) = renewal_reason(not_after) else {
            continue;
        };
        let Some(name) = names.first() else {
            continue;
        };
        // The store keeps a removed custom domain's certificate until restart,
        // but its hostname may no longer point at us or belong to the customer.
        if !server_context.acme.is_allowed(name) {
            continue;
        }
        if waiting_to_retry(server_context, name) {
            check_again_soon |= server_context.acme.is_platform_domain(name);
            continue;
        }
        if start_or_join_order(server_context, name).is_some() {
            info!("renewing certificate for {}: {reason}", names.join(", "));
            check_again_soon = true;
        }
    }
    for name in missing_allowed_names(server_context) {
        if waiting_to_retry(server_context, &name) {
            check_again_soon |= server_context.acme.is_platform_domain(&name);
            continue;
        }
        if start_or_join_order(server_context, &name).is_some() {
            info!("ordering a certificate for {name}: no certificate is installed");
            check_again_soon = true;
        }
    }
    check_again_soon
}

/// Whether the renewal task should leave a recently failed order alone. The
/// loop's sleep is shared, and a failing platform certificate keeps it short, so
/// the wait is enforced per order: many broken CNAMEs retrying every cooldown
/// could spend the account's order limit, platform orders included. Handshakes
/// can still retry sooner, under [`RENEWAL_RETRY_INTERVAL`].
fn waiting_to_retry(server_context: &ServerContext, name: &str) -> bool {
    let wait = if server_context.acme.is_platform_domain(name) {
        PLATFORM_RENEWAL_RETRY_INTERVAL
    } else {
        RENEWAL_CHECK_INTERVAL
    };
    let order_key = server_context.acme.order_key(name);
    server_context
        .acme
        .last_failure
        .get(&order_key)
        .is_some_and(|failed_at| failed_at.elapsed() < wait)
        || server_context.acme.held_back_within(&order_key, wait)
}

fn missing_allowed_names(server_context: &ServerContext) -> Vec<String> {
    let mut names = Vec::new();
    let platform_domains = server_context.acme.platform_domains();
    if platform_domains.iter().any(|name| {
        server_context
            .acme
            .certificates
            .certified_key(name)
            .is_none()
    }) && let Some(first) = platform_domains.first()
    {
        names.push(first.clone());
    }
    for domain in server_context.acme.custom_domains.iter() {
        if server_context
            .acme
            .certificates
            .certified_key(domain.key())
            .is_none()
        {
            names.push(domain.key().clone());
        }
    }
    names
}

fn renewal_reason(not_after: SystemTime) -> Option<&'static str> {
    let remaining = not_after
        .duration_since(SystemTime::now())
        .unwrap_or_default();
    (remaining <= RENEW_BEFORE_EXPIRY).then_some("the current one is close to expiring")
}

fn format_remaining(not_after: SystemTime) -> String {
    let remaining = not_after
        .duration_since(SystemTime::now())
        .unwrap_or_default();
    format!("{} days", remaining.as_secs() / (24 * 60 * 60))
}

/// Polls about once a second until [`ORDER_WAIT`]. Backoff is 1 so the default
/// exponential schedule cannot exhaust the budget in a few retries.
fn order_retry_policy() -> RetryPolicy {
    RetryPolicy::new()
        .initial_delay(Duration::from_secs(1))
        .backoff(1.0)
        .timeout(ORDER_WAIT)
}

/// Let's Encrypt stores the real HTTP-01 failure on each authorization, not
/// on the order. Refresh them after `Invalid` so the log names the hostname
/// and the problem document (404, unauthorized, connection, …).
async fn order_not_ready_error(order: &mut Order, status: OrderStatus) -> AcmeError {
    if status != OrderStatus::Invalid {
        return AcmeError::OrderStatus(format!("{status:?}"));
    }
    let mut details = Vec::new();
    let mut authorizations = order.authorizations();
    while let Some(next) = authorizations.next().await {
        match next {
            Err(error) => details.push(format!("unable to fetch authorization: {error}")),
            Ok(mut authorization) => match authorization.refresh().await {
                Err(error) => {
                    details.push(format!("unable to refresh authorization: {error}"));
                }
                Ok(_) => {
                    if let Some(detail) = format_failed_authorization(
                        authorization.identifier(),
                        authorization.status,
                        &authorization.challenges,
                    ) {
                        details.push(detail);
                    }
                }
            },
        }
    }
    if details.is_empty() {
        AcmeError::OrderStatus("Invalid".to_string())
    } else {
        AcmeError::OrderStatus(format!("Invalid: {}", details.join("; ")))
    }
}

fn format_failed_authorization(
    identifier: impl fmt::Display,
    status: AuthorizationStatus,
    challenges: &[Challenge],
) -> Option<String> {
    let identifier = identifier.to_string();
    let http01 = challenges
        .iter()
        .find(|challenge| challenge.r#type == ChallengeType::Http01);
    if let Some(error) = http01.and_then(|challenge| challenge.error.as_ref()) {
        return Some(format!("{identifier}: {}", format_problem(error)));
    }
    (status != AuthorizationStatus::Valid)
        .then(|| format!("{identifier}: authorization {status:?}"))
}

fn format_problem(problem: &Problem) -> String {
    match (&problem.detail, &problem.r#type) {
        (Some(detail), Some(kind)) => format!("{detail} ({kind})"),
        (Some(detail), None) => detail.clone(),
        (None, Some(kind)) => kind.clone(),
        (None, None) => "unknown challenge error".to_string(),
    }
}

/// Runs one HTTP-01 order end to end and installs the resulting certificate,
/// returning when it expires.
async fn provision_certificate(
    server_context: &ServerContext,
    config: &AcmeConfig,
    domains: &[String],
    cache_subdir: &Path,
) -> Result<SystemTime, AcmeError> {
    let account = load_or_create_account(config).await?;
    let identifiers = domains
        .iter()
        .map(|domain| Identifier::Dns(domain.clone()))
        .collect::<Vec<_>>();
    let mut order = account.new_order(&NewOrder::new(&identifiers)).await?;

    // Dropped once the order finishes so a token cannot be answered for longer
    // than the order it belongs to, including when the order fails partway.
    let mut published = PublishedChallenges::new(server_context.acme.clone());
    {
        let mut authorizations = order.authorizations();
        while let Some(authorization) = authorizations.next().await {
            let mut authorization = authorization?;
            match authorization.status {
                AuthorizationStatus::Pending => {}
                // Reusing a still-valid authorization needs no challenge.
                AuthorizationStatus::Valid => continue,
                status => return Err(AcmeError::Authorization(format!("{status:?}"))),
            }

            let mut challenge = authorization
                .challenge(ChallengeType::Http01)
                .ok_or(AcmeError::NoHttpChallenge)?;
            published.publish(
                challenge.token.clone(),
                challenge.key_authorization().as_str().to_string(),
            );
            challenge.set_ready().await?;
        }
    }

    let retries = order_retry_policy();
    let status = order.poll_ready(&retries).await?;
    if status != OrderStatus::Ready {
        return Err(order_not_ready_error(&mut order, status).await);
    }

    let private_key_pem = order.finalize().await?;
    let certificate_chain_pem = order.poll_certificate(&retries).await?;
    drop(published);

    let not_after = server_context.acme.certificates().install_pem(
        domains,
        &certificate_chain_pem,
        &private_key_pem,
    )?;
    // Written after installing so a certificate we cannot serve is never cached.
    let cache_dir = config.cache_dir.join(cache_subdir);
    write_private_file(
        &cache_dir.join(CERTIFICATE_FILE),
        certificate_chain_pem.as_bytes(),
    )
    .await?;
    write_private_file(
        &cache_dir.join(PRIVATE_KEY_FILE),
        private_key_pem.as_bytes(),
    )
    .await?;
    Ok(not_after)
}

/// HTTP-01 tokens currently answered by [`challenge_response`], removed as soon
/// as the order that needs them is done with them.
struct PublishedChallenges {
    state: Arc<AcmeState>,
    tokens: Vec<String>,
}

impl PublishedChallenges {
    fn new(state: Arc<AcmeState>) -> Self {
        Self {
            state,
            tokens: Vec::new(),
        }
    }

    fn publish(&mut self, token: String, key_authorization: String) {
        self.state
            .challenges
            .insert(token.clone(), key_authorization);
        self.tokens.push(token);
    }
}

impl Drop for PublishedChallenges {
    fn drop(&mut self) {
        for token in &self.tokens {
            self.state.challenges.remove(token);
        }
    }
}

/// Reuses the cached account so repeat orders are attributed to one account
/// (and one set of rate limits), creating it on first run.
async fn load_or_create_account(config: &AcmeConfig) -> Result<Account, AcmeError> {
    crate::tls::install_default_crypto_provider();
    let credentials_path = config.cache_dir.join(ACCOUNT_CREDENTIALS_FILE);
    if let Some(cached) = read_optional(&credentials_path)? {
        let credentials: AccountCredentials =
            serde_json::from_slice(&cached).map_err(AcmeError::Credentials)?;
        return Ok(Account::builder()?.from_credentials(credentials).await?);
    }

    let contacts = config
        .contacts
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let (account, credentials) = Account::builder()?
        .create(
            &NewAccount {
                contact: &contacts,
                terms_of_service_agreed: true,
                only_return_existing: false,
            },
            config.directory_url.clone(),
            None,
        )
        .await?;
    let serialized = serde_json::to_vec(&credentials).map_err(AcmeError::Credentials)?;
    write_private_file(&credentials_path, &serialized).await?;
    info!("registered a new ACME account at {}", config.directory_url);
    Ok(account)
}

/// `custom_domains` includes those whose plan no longer covers them, so that a
/// certificate already paid for in quota keeps working until it expires, whenever
/// the next restart happens to be.
fn load_cached_certificates(
    server_context: &ServerContext,
    config: &AcmeConfig,
    custom_domains: &[String],
) {
    let platform_dir = config.cache_dir.join(PLATFORM_CACHE_DIR);
    let platform_names = config.platform_domains.clone();
    let platform_label = platform_names.join(", ");
    match load_cached_certificate(
        &platform_dir,
        &platform_names,
        server_context.acme.certificates(),
    ) {
        Ok(Some(not_after)) => info!(
            "loaded cached certificate for {platform_label}, expiring in {}",
            format_remaining(not_after)
        ),
        Ok(None) => info!("no cached certificate for {platform_label}"),
        Err(error) => warn!("ignoring cached certificate for {platform_label}: {error}"),
    }

    for domain in custom_domains {
        let cache_dir = config.cache_dir.join(CUSTOM_CACHE_DIR).join(domain);
        match load_cached_certificate(
            &cache_dir,
            std::slice::from_ref(domain),
            server_context.acme.certificates(),
        ) {
            Ok(Some(not_after)) => info!(
                "loaded cached certificate for {domain}, expiring in {}",
                format_remaining(not_after)
            ),
            Ok(None) => {}
            Err(error) => warn!("ignoring cached certificate for {domain}: {error}"),
        }
    }
}

fn load_cached_certificate(
    cache_dir: &Path,
    names: &[String],
    certificates: &CertificateStore,
) -> Result<Option<SystemTime>, AcmeError> {
    let Some(certificate_chain_pem) = read_optional(&cache_dir.join(CERTIFICATE_FILE))? else {
        return Ok(None);
    };
    let Some(private_key_pem) = read_optional(&cache_dir.join(PRIVATE_KEY_FILE))? else {
        return Ok(None);
    };
    let certificate_chain_pem =
        String::from_utf8(certificate_chain_pem).map_err(|_| AcmeError::CachedPemNotUtf8)?;
    let private_key_pem =
        String::from_utf8(private_key_pem).map_err(|_| AcmeError::CachedPemNotUtf8)?;
    Ok(Some(certificates.install_pem(
        names,
        &certificate_chain_pem,
        &private_key_pem,
    )?))
}

struct SavedCustomDomain {
    name: String,
    account_id: String,
    /// Whether the account's subscription currently includes a custom domain.
    included: bool,
}

fn active_domains(saved: Vec<SavedCustomDomain>) -> impl Iterator<Item = (String, String)> {
    saved
        .into_iter()
        .filter(|domain| domain.included)
        .map(|domain| (domain.name, domain.account_id))
}

async fn load_custom_domains(
    server_context: Arc<ServerContext>,
) -> Result<Vec<SavedCustomDomain>, AcmeError> {
    let conn = open_main_database_read(server_context)
        .await
        .map_err(|error| AcmeError::Database(error.to_string()))?;
    let rows = conn
        .query(
            "SELECT s.custom_domain, p.account_id::text, sub.plan_type::text,
                    sub.subscription_status
             FROM project_settings s
             JOIN projects p ON p.id = s.project_id
             JOIN account_subscriptions sub
                ON sub.account_id = p.account_id AND sub.ended_at IS NULL
             WHERE s.custom_domain IS NOT NULL",
            &[],
        )
        .await
        .map_err(|error| AcmeError::Database(error.to_string()))?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            Some(SavedCustomDomain {
                name: row.get::<_, Option<String>>(0)?,
                account_id: row.get(1),
                included: crate::projects::plan_includes_paid_features(row.get(2), row.get(3)),
            })
        })
        .collect())
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, AcmeError> {
    match std::fs::read(path) {
        Ok(contents) => Ok(Some(contents)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(AcmeError::Io(error)),
    }
}

/// Writes through a temporary file so a process crash mid-write cannot leave a
/// half-written certificate behind, and keeps the file owner-only because it
/// holds private keys.
async fn write_private_file(path: &Path, contents: &[u8]) -> Result<(), AcmeError> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(AcmeError::Io)?;
    }
    // Unique because orders for different domains can create the shared account
    // file at once.
    let mut suffix = [0u8; 8];
    getrandom::getrandom(&mut suffix).map_err(|error| AcmeError::Io(io::Error::other(error)))?;
    let mut temporary = path.as_os_str().to_os_string();
    temporary.push(format!(".{:016x}.tmp", u64::from_ne_bytes(suffix)));
    let temporary = PathBuf::from(temporary);

    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create_new(true);
    // Set at creation so the key is never readable by others, even briefly.
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(&temporary).await.map_err(AcmeError::Io)?;
    file.write_all(contents).await.map_err(AcmeError::Io)?;
    file.flush().await.map_err(AcmeError::Io)?;
    drop(file);
    tokio::fs::rename(&temporary, path)
        .await
        .map_err(AcmeError::Io)
}

#[derive(Debug)]
pub(crate) enum AcmeError {
    Acme(instant_acme::Error),
    /// The account or the deployment has no custom-domain orders left for now.
    OrderLimit,
    Authorization(String),
    /// The cached certificate or key is not valid PEM text.
    CachedPemNotUtf8,
    Certificate(CertificateError),
    Credentials(serde_json::Error),
    Database(String),
    /// A custom domain isn't ready to order for, or that couldn't be established,
    /// so no request was sent to the ACME server.
    DomainNotReady(String),
    Io(io::Error),
    /// The ACME server offered no HTTP-01 challenge for an identifier, which
    /// happens for wildcard names (they require DNS-01).
    NoHttpChallenge,
    /// Order did not become ready. For `Invalid`, this includes each
    /// hostname's HTTP-01 problem document.
    OrderStatus(String),
    /// The ACME server had not finished the order, or stopped responding, within
    /// the time allowed. Not a validation failure; the order may still complete.
    Timeout,
}

impl fmt::Display for AcmeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Acme(error) => write!(formatter, "{error}"),
            Self::OrderLimit => write!(formatter, "over the certificate order limit"),
            Self::Authorization(status) => {
                write!(formatter, "unusable authorization status: {status}")
            }
            Self::CachedPemNotUtf8 => write!(formatter, "cached certificate is not valid PEM"),
            Self::Certificate(error) => write!(formatter, "{error}"),
            Self::Credentials(error) => write!(formatter, "invalid account credentials: {error}"),
            Self::Database(error) => write!(formatter, "{error}"),
            Self::DomainNotReady(reason) => write!(formatter, "{reason}"),
            Self::Io(error) => write!(formatter, "{error}"),
            Self::NoHttpChallenge => {
                write!(formatter, "no http-01 challenge offered for an identifier")
            }
            Self::OrderStatus(status) => write!(formatter, "unexpected order status: {status}"),
            Self::Timeout => write!(formatter, "timed out waiting for the ACME server"),
        }
    }
}

impl From<instant_acme::Error> for AcmeError {
    fn from(error: instant_acme::Error) -> Self {
        match error {
            instant_acme::Error::Timeout(_) => Self::Timeout,
            error => Self::Acme(error),
        }
    }
}

impl From<CertificateError> for AcmeError {
    fn from(error: CertificateError) -> Self {
        Self::Certificate(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY_SECS: u64 = 24 * 60 * 60;
    const TEST_ACCOUNT: &str = "test-account";
    const SERVER_ADDRESS: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(203, 0, 113, 1));
    const OTHER_ADDRESS: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(198, 51, 100, 9));

    fn set_addresses(server_context: &ServerContext, name: &str, addresses: &[IpAddr]) {
        server_context
            .acme
            .test_addresses
            .insert(name.to_string(), addresses.to_vec());
    }

    /// Makes `domain` a saved custom domain that resolves where the platform does.
    fn point_at_platform(server_context: &ServerContext, domain: &str) {
        server_context
            .acme
            .custom_domains
            .insert(domain.to_string(), TEST_ACCOUNT.to_string());
        set_addresses(server_context, "eu1.revscope.co", &[SERVER_ADDRESS]);
        set_addresses(server_context, domain, &[SERVER_ADDRESS]);
    }

    fn remaining_orders(server_context: &ServerContext, account: &str) -> u32 {
        let mut remaining = 0;
        while server_context.acme.account_orders.check(account).is_ok() {
            remaining += 1;
        }
        remaining
    }

    /// A fake ACME server that only records whether anything connected.
    async fn configure_listening_acme_server(
        server_context: &ServerContext,
    ) -> tokio::net::TcpListener {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fake ACME server");
        let mut config = test_config(&["eu1.revscope.co"]);
        config.directory_url = format!(
            "https://{}/directory",
            listener.local_addr().expect("listener address")
        );
        server_context.acme.set_config(config);
        listener
    }

    async fn wait_for_orders_to_finish(server_context: &ServerContext) {
        tokio::time::timeout(Duration::from_secs(30), async {
            while !server_context.acme.in_flight.is_empty() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("order should finish");
    }

    /// Moves a held-back order past the wait before a handshake may retry it.
    fn backdate_hold(server_context: &ServerContext, domain: &str) {
        let mut held_back_at = server_context
            .acme
            .held_back
            .get_mut(domain)
            .expect("order was held back");
        *held_back_at = held_back_at
            .checked_sub(HELD_BACK_RECHECK_INTERVAL + Duration::from_secs(1))
            .expect("system has been up longer than the recheck interval");
    }

    async fn assert_no_connection(listener: &tokio::net::TcpListener) {
        assert!(
            tokio::time::timeout(Duration::from_millis(100), listener.accept())
                .await
                .is_err(),
            "the ACME server must not be contacted"
        );
    }

    #[tokio::test]
    async fn a_domain_not_pointed_here_never_reaches_the_acme_server() {
        let server_context = test_context();
        let listener = configure_listening_acme_server(&server_context).await;
        point_at_platform(&server_context, "assets.example.com");
        set_addresses(&server_context, "assets.example.com", &[OTHER_ADDRESS]);
        let fresh_budget = remaining_orders(&server_context, "another-account");

        request_issuance(server_context.clone(), "assets.example.com");
        wait_for_orders_to_finish(&server_context).await;

        assert_no_connection(&listener).await;
        assert_eq!(
            remaining_orders(&server_context, TEST_ACCOUNT),
            fresh_budget,
            "a failed DNS check must not spend the account's orders"
        );
    }

    #[tokio::test]
    async fn a_domain_with_no_addresses_never_reaches_the_acme_server() {
        let server_context = test_context();
        let listener = configure_listening_acme_server(&server_context).await;
        point_at_platform(&server_context, "assets.example.com");
        set_addresses(&server_context, "assets.example.com", &[]);

        request_issuance(server_context.clone(), "assets.example.com");
        wait_for_orders_to_finish(&server_context).await;

        assert_no_connection(&listener).await;
    }

    #[tokio::test]
    async fn a_domain_not_pointed_here_is_rechecked_soon() {
        let server_context = test_context();
        let listener = configure_listening_acme_server(&server_context).await;
        point_at_platform(&server_context, "assets.example.com");
        set_addresses(&server_context, "assets.example.com", &[OTHER_ADDRESS]);

        request_issuance(server_context.clone(), "assets.example.com");
        wait_for_orders_to_finish(&server_context).await;

        assert!(start_or_join_order(&server_context, "assets.example.com").is_none());
        assert!(waiting_to_retry(&server_context, "assets.example.com"));

        backdate_hold(&server_context, "assets.example.com");
        set_addresses(&server_context, "assets.example.com", &[SERVER_ADDRESS]);

        assert!(start_or_join_order(&server_context, "assets.example.com").is_some());
        tokio::time::timeout(Duration::from_secs(30), listener.accept())
            .await
            .expect("the recheck should let the order reach the ACME server")
            .expect("accept ACME connection");
    }

    #[tokio::test]
    async fn a_domain_pointed_here_orders_and_spends_one_of_its_accounts_orders() {
        let server_context = test_context();
        let listener = configure_listening_acme_server(&server_context).await;
        point_at_platform(&server_context, "assets.example.com");
        let fresh_budget = remaining_orders(&server_context, "another-account");

        request_issuance(server_context.clone(), "assets.example.com");
        tokio::time::timeout(Duration::from_secs(30), listener.accept())
            .await
            .expect("the order should contact the ACME server")
            .expect("accept ACME connection");

        assert_eq!(
            remaining_orders(&server_context, TEST_ACCOUNT),
            fresh_budget - 1
        );
    }

    #[tokio::test]
    async fn a_domain_sharing_one_of_several_addresses_counts_as_pointed_here() {
        let server_context = test_context();
        let listener = configure_listening_acme_server(&server_context).await;
        point_at_platform(&server_context, "assets.example.com");
        let mapped = IpAddr::V6(std::net::Ipv4Addr::new(203, 0, 113, 1).to_ipv6_mapped());
        set_addresses(
            &server_context,
            "assets.example.com",
            &[OTHER_ADDRESS, mapped],
        );

        request_issuance(server_context.clone(), "assets.example.com");

        tokio::time::timeout(Duration::from_secs(30), listener.accept())
            .await
            .expect("the order should contact the ACME server")
            .expect("accept ACME connection");
    }

    #[tokio::test]
    async fn an_account_over_its_limit_cannot_order_but_others_can() {
        let server_context = test_context();
        let listener = configure_listening_acme_server(&server_context).await;
        point_at_platform(&server_context, "assets.example.com");
        remaining_orders(&server_context, TEST_ACCOUNT);

        request_issuance(server_context.clone(), "assets.example.com");
        wait_for_orders_to_finish(&server_context).await;

        assert_no_connection(&listener).await;
        assert!(start_or_join_order(&server_context, "assets.example.com").is_none());

        point_at_platform(&server_context, "other.example.org");
        server_context.acme.custom_domains.insert(
            "other.example.org".to_string(),
            "another-account".to_string(),
        );
        request_issuance(server_context.clone(), "other.example.org");
        tokio::time::timeout(Duration::from_secs(30), listener.accept())
            .await
            .expect("another account's order should contact the ACME server")
            .expect("accept ACME connection");
    }

    #[tokio::test]
    async fn a_renewal_ignores_an_exhausted_account_limit() {
        let server_context = test_context();
        let listener = configure_listening_acme_server(&server_context).await;
        point_at_platform(&server_context, "assets.example.com");
        install_expiring_in(&server_context, &["eu1.revscope.co"], 60);
        install_expiring_in(&server_context, &["assets.example.com"], 7);
        remaining_orders(&server_context, TEST_ACCOUNT);

        assert!(start_due_orders(&server_context));

        tokio::time::timeout(Duration::from_secs(30), listener.accept())
            .await
            .expect("the renewal should contact the ACME server")
            .expect("accept ACME connection");
    }

    #[tokio::test]
    async fn a_renewal_is_held_back_when_the_domain_no_longer_points_here() {
        let server_context = test_context();
        let listener = configure_listening_acme_server(&server_context).await;
        point_at_platform(&server_context, "assets.example.com");
        set_addresses(&server_context, "assets.example.com", &[OTHER_ADDRESS]);
        install_expiring_in(&server_context, &["eu1.revscope.co"], 60);
        install_expiring_in(&server_context, &["assets.example.com"], 7);

        assert!(start_due_orders(&server_context));
        wait_for_orders_to_finish(&server_context).await;

        assert_no_connection(&listener).await;
    }

    #[tokio::test]
    async fn the_deployment_limit_stops_orders_and_does_not_charge_the_account() {
        let server_context = test_context();
        let listener = configure_listening_acme_server(&server_context).await;
        point_at_platform(&server_context, "assets.example.com");
        let fresh_budget = remaining_orders(&server_context, "another-account");
        while server_context
            .acme
            .deployment_orders
            .check(DEPLOYMENT_BUDGET_KEY)
            .is_ok()
        {}

        request_issuance(server_context.clone(), "assets.example.com");
        wait_for_orders_to_finish(&server_context).await;

        assert_no_connection(&listener).await;
        assert_eq!(
            remaining_orders(&server_context, TEST_ACCOUNT),
            fresh_budget
        );
    }

    #[tokio::test]
    async fn an_account_over_its_limit_does_not_spend_the_deployment_limit() {
        let server_context = test_context();
        let _listener = configure_listening_acme_server(&server_context).await;
        point_at_platform(&server_context, "assets.example.com");
        remaining_orders(&server_context, TEST_ACCOUNT);

        request_issuance(server_context.clone(), "assets.example.com");
        wait_for_orders_to_finish(&server_context).await;

        let mut remaining = 0;
        while server_context
            .acme
            .deployment_orders
            .check(DEPLOYMENT_BUDGET_KEY)
            .is_ok()
        {
            remaining += 1;
        }
        let fresh = RateLimiter::new(CERTIFICATE_ORDERS_PER_DEPLOYMENT);
        let mut fresh_remaining = 0;
        while fresh.check(DEPLOYMENT_BUDGET_KEY).is_ok() {
            fresh_remaining += 1;
        }
        assert_eq!(remaining, fresh_remaining);
    }

    #[tokio::test]
    async fn a_renewal_retry_spends_the_account_limit() {
        let server_context = test_context();
        let listener = configure_listening_acme_server(&server_context).await;
        point_at_platform(&server_context, "assets.example.com");
        install_expiring_in(&server_context, &["assets.example.com"], 7);
        remaining_orders(&server_context, TEST_ACCOUNT);
        let past_the_short_cooldown = Instant::now()
            .checked_sub(RENEWAL_RETRY_INTERVAL + Duration::from_secs(1))
            .expect("system has been up longer than the cooldown");
        server_context
            .acme
            .last_failure
            .insert("assets.example.com".to_string(), past_the_short_cooldown);

        assert!(start_or_join_order(&server_context, "assets.example.com").is_some());
        wait_for_orders_to_finish(&server_context).await;

        assert_no_connection(&listener).await;
    }

    #[tokio::test]
    async fn a_held_back_renewal_still_gets_its_uncharged_first_attempt() {
        let server_context = test_context();
        let listener = configure_listening_acme_server(&server_context).await;
        point_at_platform(&server_context, "assets.example.com");
        set_addresses(&server_context, "assets.example.com", &[OTHER_ADDRESS]);
        install_expiring_in(&server_context, &["eu1.revscope.co"], 60);
        install_expiring_in(&server_context, &["assets.example.com"], 7);
        remaining_orders(&server_context, TEST_ACCOUNT);
        assert!(start_due_orders(&server_context));
        wait_for_orders_to_finish(&server_context).await;
        assert_no_connection(&listener).await;

        backdate_hold(&server_context, "assets.example.com");
        set_addresses(&server_context, "assets.example.com", &[SERVER_ADDRESS]);
        assert!(start_or_join_order(&server_context, "assets.example.com").is_some());

        tokio::time::timeout(Duration::from_secs(30), listener.accept())
            .await
            .expect("the renewal should contact the ACME server")
            .expect("accept ACME connection");
    }

    #[tokio::test]
    async fn replacing_a_domain_forgets_that_it_was_held_back() {
        let server_context = test_context();
        let _listener = configure_listening_acme_server(&server_context).await;
        point_at_platform(&server_context, "assets.example.com");
        set_addresses(&server_context, "assets.example.com", &[OTHER_ADDRESS]);
        request_issuance(server_context.clone(), "assets.example.com");
        wait_for_orders_to_finish(&server_context).await;
        assert!(
            server_context
                .acme
                .held_back
                .contains_key("assets.example.com")
        );

        server_context.acme.replace_custom_domain(
            Some("assets.example.com"),
            Some("next.example.com"),
            TEST_ACCOUNT,
        );

        assert!(server_context.acme.held_back.is_empty());
    }

    #[test]
    fn a_domain_no_longer_saved_is_not_held_back() {
        let server_context = test_context();
        server_context.acme.hold_back("removed.example.com");
        assert!(server_context.acme.held_back.is_empty());
    }

    #[tokio::test]
    async fn a_dns_check_waits_for_a_lookup_place_and_then_orders() {
        let server_context = test_context();
        let listener = configure_listening_acme_server(&server_context).await;
        point_at_platform(&server_context, "assets.example.com");
        let all_places = server_context
            .acme
            .dns_lookups
            .clone()
            .try_acquire_many_owned(u32::try_from(MAX_DNS_LOOKUPS).expect("limit fits"))
            .expect("places are free");

        request_issuance(server_context.clone(), "assets.example.com");

        assert_no_connection(&listener).await;
        assert!(server_context.acme.held_back.is_empty());
        assert!(!server_context.acme.in_flight.is_empty());

        drop(all_places);
        tokio::time::timeout(Duration::from_secs(30), listener.accept())
            .await
            .expect("the order should continue once a place is free")
            .expect("accept ACME connection");
    }

    fn drain_dns_checks(server_context: &ServerContext, account: &str) {
        while server_context
            .acme
            .account_dns_checks
            .check(account)
            .is_ok()
        {}
    }

    #[tokio::test]
    async fn an_account_over_its_dns_check_limit_is_held_back_without_spending_orders() {
        let server_context = test_context();
        let listener = configure_listening_acme_server(&server_context).await;
        point_at_platform(&server_context, "assets.example.com");
        drain_dns_checks(&server_context, TEST_ACCOUNT);
        let fresh_budget = remaining_orders(&server_context, "another-account");

        request_issuance(server_context.clone(), "assets.example.com");
        wait_for_orders_to_finish(&server_context).await;

        assert_no_connection(&listener).await;
        assert!(
            server_context
                .acme
                .held_back
                .contains_key("assets.example.com")
        );
        assert_eq!(
            remaining_orders(&server_context, TEST_ACCOUNT),
            fresh_budget
        );
    }

    #[tokio::test]
    async fn a_first_renewal_attempt_ignores_the_dns_check_limit() {
        let server_context = test_context();
        let listener = configure_listening_acme_server(&server_context).await;
        point_at_platform(&server_context, "assets.example.com");
        install_expiring_in(&server_context, &["eu1.revscope.co"], 60);
        install_expiring_in(&server_context, &["assets.example.com"], 7);
        drain_dns_checks(&server_context, TEST_ACCOUNT);

        assert!(start_due_orders(&server_context));

        tokio::time::timeout(Duration::from_secs(30), listener.accept())
            .await
            .expect("the renewal should contact the ACME server")
            .expect("accept ACME connection");
    }

    #[tokio::test]
    async fn a_finished_lookup_gives_back_its_place() {
        let state = AcmeState::new();
        state
            .resolve("localhost")
            .await
            .expect("localhost resolves");
        tokio::time::timeout(Duration::from_secs(5), async {
            while state.dns_lookups.available_permits() < MAX_DNS_LOOKUPS {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the place should be released");
    }

    #[tokio::test]
    async fn saving_the_same_domain_again_keeps_it_held_back() {
        let server_context = test_context();
        let _listener = configure_listening_acme_server(&server_context).await;
        point_at_platform(&server_context, "assets.example.com");
        set_addresses(&server_context, "assets.example.com", &[OTHER_ADDRESS]);
        request_issuance(server_context.clone(), "assets.example.com");
        wait_for_orders_to_finish(&server_context).await;

        server_context.acme.replace_custom_domain(
            Some("assets.example.com"),
            Some("assets.example.com"),
            TEST_ACCOUNT,
        );

        assert!(start_or_join_order(&server_context, "assets.example.com").is_none());
    }

    #[tokio::test]
    async fn a_resolver_error_holds_the_order_back() {
        let server_context = test_context();
        let listener = configure_listening_acme_server(&server_context).await;
        // A NUL makes the system lookup fail at once, without touching the network.
        let unresolvable = "assets\0.example.com";
        point_at_platform(&server_context, unresolvable);
        server_context.acme.test_addresses.remove(unresolvable);

        request_issuance(server_context.clone(), unresolvable);
        wait_for_orders_to_finish(&server_context).await;

        assert_no_connection(&listener).await;
        assert!(server_context.acme.held_back.contains_key(unresolvable));
    }

    #[tokio::test]
    async fn saving_a_domain_the_plan_excludes_neither_activates_nor_orders_it() {
        let server_context = test_context();
        let _listener = configure_listening_acme_server(&server_context).await;

        apply_saved_custom_domain(
            &server_context,
            None,
            Some("assets.example.com"),
            false,
            TEST_ACCOUNT,
        );

        assert!(server_context.acme.custom_domains.is_empty());
        assert!(server_context.acme.in_flight.is_empty());
    }

    #[tokio::test]
    async fn saving_a_domain_the_plan_includes_activates_and_orders_it() {
        let server_context = test_context();
        let listener = configure_listening_acme_server(&server_context).await;
        point_at_platform(&server_context, "assets.example.com");
        server_context.acme.custom_domains.clear();

        apply_saved_custom_domain(
            &server_context,
            None,
            Some("assets.example.com"),
            true,
            TEST_ACCOUNT,
        );

        assert!(
            server_context
                .acme
                .custom_domains
                .contains_key("assets.example.com")
        );
        tokio::time::timeout(Duration::from_secs(30), listener.accept())
            .await
            .expect("the order should contact the ACME server")
            .expect("accept ACME connection");
    }

    #[tokio::test]
    async fn resolves_names_with_the_system_resolver() {
        let addresses = AcmeState::new()
            .resolve("localhost")
            .await
            .expect("localhost resolves");
        assert!(addresses.iter().any(IpAddr::is_loopback), "{addresses:?}");
    }

    /// Saves a custom domain on a new project in a new account on `plan`.
    async fn save_domain_on_plan(
        conn: &crate::db::main::Connection,
        tag: &str,
        plan: &'static str,
    ) -> (String, String) {
        let account_id = crate::test_utils::insert_test_account(conn, tag).await;
        crate::account_subscription::insert_open_period(
            conn,
            &account_id,
            &crate::account_subscription::PlanTerms {
                plan_type: plan,
                ..crate::account_subscription::PlanTerms::COMPLIMENTARY
            },
        )
        .await
        .expect("open subscription");
        conn.execute(
            "UPDATE account_subscriptions SET subscription_status = 'active'
             WHERE account_id = $1::text::uuid",
            &[&account_id],
        )
        .await
        .expect("activate subscription");
        let name = format!("p{}", account_id.replace('-', ""));
        let project_id: i32 = conn
            .query_one(
                "INSERT INTO projects (account_id, name) VALUES ($1::text::uuid, $2) RETURNING id",
                &[&account_id, &name],
            )
            .await
            .expect("insert project")
            .get(0);
        let domain = format!("{name}.example.com");
        conn.execute(
            "INSERT INTO project_settings (project_id, custom_domain) VALUES ($1, $2)",
            &[&project_id, &domain],
        )
        .await
        .expect("save custom domain");
        (domain, account_id)
    }

    async fn delete_account(conn: &crate::db::main::Connection, account_id: &str) {
        conn.execute(
            "DELETE FROM accounts WHERE id = $1::text::uuid",
            &[&account_id],
        )
        .await
        .expect("delete account");
    }

    #[tokio::test]
    async fn loads_each_custom_domain_with_its_owning_account() {
        let (server_context, conn) = crate::test_utils::main_test_db("acme-domain-account").await;
        let (domain, account_id) =
            save_domain_on_plan(&conn, "acme_domain_account", "starter").await;

        let domains = load_custom_domains(server_context).await.expect("load");

        let loaded = domains
            .iter()
            .find(|saved| saved.name == domain)
            .expect("domain");
        assert_eq!(loaded.account_id, account_id);
        assert!(loaded.included);
        delete_account(&conn, &account_id).await;
    }

    #[tokio::test]
    #[cfg(not(feature = "self-hosted"))]
    async fn a_free_accounts_domain_is_listed_but_not_active() {
        let (server_context, conn) = crate::test_utils::main_test_db("acme-domain-free").await;
        let (domain, account_id) = save_domain_on_plan(&conn, "acme_domain_free", "free").await;

        let saved = load_custom_domains(server_context).await.expect("load");

        assert!(saved.iter().any(|saved| saved.name == domain));
        assert!(!active_domains(saved).any(|(name, _)| name == domain));
        delete_account(&conn, &account_id).await;
    }

    #[tokio::test]
    #[cfg(not(feature = "self-hosted"))]
    async fn a_restart_keeps_serving_a_domain_the_plan_no_longer_covers() {
        let (server_context, conn) = crate::test_utils::main_test_db("acme-domain-kept").await;
        let (domain, account_id) = save_domain_on_plan(&conn, "acme_domain_kept", "free").await;
        let cache_dir = temporary_cache_dir("downgraded");
        let domain_dir = cache_dir.join(CUSTOM_CACHE_DIR).join(&domain);
        std::fs::create_dir_all(&domain_dir).expect("create domain cache");
        let (pem, key) = self_signed(&[domain.as_str()]);
        std::fs::write(domain_dir.join(CERTIFICATE_FILE), pem).expect("cache certificate");
        std::fs::write(domain_dir.join(PRIVATE_KEY_FILE), key).expect("cache key");
        let mut config = test_config(&["eu1.revscope.co"]);
        config.cache_dir = cache_dir.clone();

        prepare_acme(server_context.clone(), &config).await;

        assert!(server_context.acme.resolve_sni(Some(&domain)).is_some());
        assert!(!server_context.acme.is_allowed(&domain));
        std::fs::remove_dir_all(&cache_dir).expect("remove cache directory");
        delete_account(&conn, &account_id).await;
    }

    #[test]
    fn setting_custom_domains_drops_those_no_longer_listed() {
        let state = AcmeState::new();
        state.set_custom_domains([("old.example.com".to_string(), TEST_ACCOUNT.to_string())]);
        state.set_custom_domains([("new.example.com".to_string(), TEST_ACCOUNT.to_string())]);
        assert!(!state.custom_domains.contains_key("old.example.com"));
        assert!(state.custom_domains.contains_key("new.example.com"));
    }

    fn test_context() -> Arc<ServerContext> {
        Arc::new(ServerContext::new_in_memory("acme-tests"))
    }

    fn self_signed(names: &[&str]) -> (String, String) {
        self_signed_expiring(
            names,
            SystemTime::now() + Duration::from_secs(365 * DAY_SECS),
        )
    }

    fn self_signed_expiring(names: &[&str], not_after: SystemTime) -> (String, String) {
        let mut params = rcgen::CertificateParams::new(
            names
                .iter()
                .map(|name| (*name).to_string())
                .collect::<Vec<_>>(),
        )
        .expect("certificate parameters");
        params.not_after = not_after.into();
        let signing_key = rcgen::KeyPair::generate().expect("generate key pair");
        let certificate = params.self_signed(&signing_key).expect("self-sign");
        (certificate.pem(), signing_key.serialize_pem())
    }

    fn name_list(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_string()).collect()
    }

    fn test_config(platform_domains: &[&str]) -> AcmeConfig {
        AcmeConfig {
            contacts: Vec::new(),
            directory_url: LetsEncrypt::Staging.url().to_string(),
            cache_dir: PathBuf::from("unused"),
            platform_domains: name_list(platform_domains),
        }
    }

    #[test]
    fn serves_a_published_challenge_token() {
        let server_context = test_context();
        server_context
            .acme
            .challenges
            .insert("token-1".to_string(), "token-1.thumbprint".to_string());

        let response = challenge_response(&server_context, "/.well-known/acme-challenge/token-1")
            .expect("challenge path is handled");

        assert_eq!(response.body, b"token-1.thumbprint");
        assert!(
            String::from_utf8_lossy(&response.headers).starts_with("HTTP/1.1 200 OK"),
            "{:?}",
            String::from_utf8_lossy(&response.headers)
        );
    }

    #[test]
    fn answers_an_unknown_challenge_token_with_not_found() {
        let server_context = test_context();

        let response = challenge_response(&server_context, "/.well-known/acme-challenge/nope")
            .expect("challenge path is handled");

        assert!(
            String::from_utf8_lossy(&response.headers).starts_with("HTTP/1.1 404 Not Found"),
            "{:?}",
            String::from_utf8_lossy(&response.headers)
        );
    }

    #[test]
    fn leaves_other_paths_to_the_router() {
        let server_context = test_context();

        assert!(challenge_response(&server_context, "/demo").is_none());
        assert!(challenge_response(&server_context, "/v2/session/me").is_none());
        assert!(challenge_response(&server_context, "/.well-known/other").is_none());
    }

    #[test]
    fn recognizes_acme_challenge_paths() {
        assert!(is_acme_challenge_path(
            "/.well-known/acme-challenge/token-1"
        ));
        assert!(is_acme_challenge_path(
            "/.well-known/acme-challenge/token-1?foo=1"
        ));
        assert!(is_acme_challenge_path("/.well-known/acme-challenge/"));
        assert!(!is_acme_challenge_path("/.well-known/acme-challenge"));
        assert!(!is_acme_challenge_path("/.well-known/other"));
        assert!(!is_acme_challenge_path("/demo"));
    }

    #[test]
    fn unpublishes_challenge_tokens_once_the_order_finishes() {
        let server_context = test_context();
        let mut published = PublishedChallenges::new(server_context.acme.clone());
        published.publish("token-1".to_string(), "token-1.thumbprint".to_string());
        assert!(server_context.acme.challenges.contains_key("token-1"));

        drop(published);

        assert!(!server_context.acme.challenges.contains_key("token-1"));
    }

    #[test]
    fn renews_only_within_thirty_days_of_expiry() {
        let in_days = |days| SystemTime::now() + Duration::from_secs(days * DAY_SECS);
        assert_eq!(renewal_reason(in_days(31)), None);
        assert_eq!(
            renewal_reason(in_days(29)),
            Some("the current one is close to expiring")
        );
    }

    fn install_expiring_in(server_context: &ServerContext, names: &[&str], days: i64) {
        let now = SystemTime::now();
        let offset = Duration::from_secs(days.unsigned_abs() * DAY_SECS);
        let not_after = if days < 0 { now - offset } else { now + offset };
        let (pem, key) = self_signed_expiring(names, not_after);
        server_context
            .acme
            .certificates
            .install_pem(&name_list(names), &pem, &key)
            .expect("install certificate");
    }

    /// The fake server drops the connection it accepts, ending the order the way
    /// an ACME outage would.
    async fn assert_renewal_reaches_acme_server(names: &[&str], days_left: i64, order_key: &str) {
        let server_context = test_context();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fake ACME server");
        let mut config = test_config(&["eu1.revscope.co", "api.revscope.co"]);
        config.directory_url = format!(
            "https://{}/directory",
            listener.local_addr().expect("listener address")
        );
        server_context.acme.set_config(config);
        for name in names {
            if !server_context.acme.is_platform_domain(name) {
                point_at_platform(&server_context, name);
            }
        }
        // A missing platform certificate would start its own order, which could
        // be the one that reaches the fake server.
        if order_key != PLATFORM_ORDER_KEY {
            install_expiring_in(&server_context, &["eu1.revscope.co", "api.revscope.co"], 60);
        }
        install_expiring_in(&server_context, names, days_left);
        let current = server_context
            .acme
            .certificates
            .certified_key(names[0])
            .expect("installed certificate");

        assert!(start_due_orders(&server_context), "order should start");
        assert!(server_context.acme.in_flight.contains_key(order_key));

        tokio::time::timeout(Duration::from_secs(30), listener.accept())
            .await
            .expect("renewal should contact the ACME server")
            .expect("accept ACME connection");
        drop(listener);
        tokio::time::timeout(Duration::from_secs(30), async {
            while server_context.acme.in_flight.contains_key(order_key) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("failed order should finish");

        assert!(server_context.acme.last_failure.contains_key(order_key));
        let served = server_context
            .acme
            .resolve_sni(Some(names[0]))
            .expect("the current certificate stays in service");
        assert_eq!(served.cert, current.cert);
    }

    /// The fake server accepts the connection and never answers.
    async fn run_order_against_a_silent_acme_server(server_context: &Arc<ServerContext>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fake ACME server");
        let mut config = test_config(&["eu1.revscope.co"]);
        config.directory_url = format!(
            "https://{}/directory",
            listener.local_addr().expect("listener address")
        );
        server_context.acme.set_config(config);
        tokio::spawn(async move {
            let mut connections = Vec::new();
            while let Ok((stream, _)) = listener.accept().await {
                connections.push(stream);
            }
        });

        assert!(start_due_orders(server_context), "order should start");
        tokio::time::timeout(ORDER_DEADLINE * 2, async {
            while !server_context.acme.in_flight.is_empty() {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        })
        .await
        .expect("a stalled order should give up");
    }

    #[tokio::test(start_paused = true)]
    async fn a_stalled_renewal_gives_up_and_cools_down() {
        let server_context = test_context();
        install_expiring_in(&server_context, &["eu1.revscope.co"], 7);

        run_order_against_a_silent_acme_server(&server_context).await;

        assert!(
            server_context
                .acme
                .last_failure
                .contains_key(PLATFORM_ORDER_KEY)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_stalled_first_order_leaves_handshakes_free_to_retry() {
        let server_context = test_context();

        run_order_against_a_silent_acme_server(&server_context).await;

        assert!(server_context.acme.last_failure.is_empty());
    }

    #[tokio::test]
    async fn an_expired_certificate_reloaded_after_a_restart_is_renewed() {
        let cache_dir = temporary_cache_dir("restart");
        let platform_dir = cache_dir.join(PLATFORM_CACHE_DIR);
        std::fs::create_dir_all(&platform_dir).expect("create platform cache");
        let (pem, key) = self_signed_expiring(
            &["eu1.revscope.co"],
            SystemTime::now() - Duration::from_secs(DAY_SECS),
        );
        std::fs::write(platform_dir.join(CERTIFICATE_FILE), pem).expect("cache certificate");
        std::fs::write(platform_dir.join(PRIVATE_KEY_FILE), key).expect("cache key");
        let mut config = test_config(&["eu1.revscope.co"]);
        config.cache_dir = cache_dir.clone();
        config.directory_url = "https://127.0.0.1:1/directory".to_string();
        let server_context = test_context();
        server_context.acme.set_config(config.clone());

        load_cached_certificates(&server_context, &config, &[]);
        assert!(
            server_context
                .acme
                .certificates
                .certified_key("eu1.revscope.co")
                .is_some()
        );

        assert!(start_due_orders(&server_context));
        assert!(
            server_context
                .acme
                .in_flight
                .contains_key(PLATFORM_ORDER_KEY)
        );
        std::fs::remove_dir_all(&cache_dir).expect("remove cache directory");
    }

    #[test]
    fn a_cached_certificate_with_the_wrong_key_counts_as_missing() {
        let cache_dir = temporary_cache_dir("mismatched-key");
        let (pem, _) = self_signed(&["eu1.revscope.co"]);
        let (_, other_key) = self_signed(&["eu1.revscope.co"]);
        std::fs::write(cache_dir.join(CERTIFICATE_FILE), pem).expect("cache certificate");
        std::fs::write(cache_dir.join(PRIVATE_KEY_FILE), other_key).expect("cache key");
        let server_context = test_context();
        server_context
            .acme
            .set_config(test_config(&["eu1.revscope.co"]));

        let loaded = load_cached_certificate(
            &cache_dir,
            &name_list(&["eu1.revscope.co"]),
            server_context.acme.certificates(),
        );

        assert!(loaded.is_err());
        assert_eq!(missing_allowed_names(&server_context), ["eu1.revscope.co"]);
        std::fs::remove_dir_all(&cache_dir).expect("remove cache directory");
    }

    #[tokio::test]
    async fn renews_a_platform_certificate_close_to_expiring() {
        assert_renewal_reaches_acme_server(
            &["eu1.revscope.co", "api.revscope.co"],
            7,
            PLATFORM_ORDER_KEY,
        )
        .await;
    }

    #[tokio::test]
    async fn renews_an_expired_custom_domain_certificate() {
        assert_renewal_reaches_acme_server(&["assets.example.com"], -1, "assets.example.com").await;
    }

    #[test]
    fn does_not_order_a_certificate_that_is_not_due() {
        let server_context = test_context();
        server_context
            .acme
            .set_config(test_config(&["eu1.revscope.co"]));
        install_expiring_in(&server_context, &["eu1.revscope.co"], 60);

        assert!(!start_due_orders(&server_context));
        assert!(server_context.acme.in_flight.is_empty());
    }

    #[test]
    fn does_not_renew_a_removed_custom_domain() {
        let server_context = test_context();
        server_context
            .acme
            .set_config(test_config(&["eu1.revscope.co"]));
        install_expiring_in(&server_context, &["eu1.revscope.co"], 60);
        install_expiring_in(&server_context, &["removed.example.com"], 7);

        assert!(!start_due_orders(&server_context));
        assert!(server_context.acme.in_flight.is_empty());
    }

    #[test]
    fn a_cooling_down_platform_renewal_starts_no_order_but_retries_soon() {
        let server_context = test_context();
        server_context
            .acme
            .set_config(test_config(&["eu1.revscope.co"]));
        install_expiring_in(&server_context, &["eu1.revscope.co"], 7);
        server_context
            .acme
            .last_failure
            .insert(PLATFORM_ORDER_KEY.to_string(), Instant::now());

        assert!(start_due_orders(&server_context));
        assert!(server_context.acme.in_flight.is_empty());
    }

    #[test]
    fn a_failed_platform_renewal_waits_past_the_handshake_cooldown() {
        let server_context = test_context();
        server_context
            .acme
            .set_config(test_config(&["eu1.revscope.co"]));
        install_expiring_in(&server_context, &["eu1.revscope.co"], 7);
        let past_the_short_cooldown = Instant::now()
            .checked_sub(RENEWAL_RETRY_INTERVAL + Duration::from_secs(1))
            .expect("system has been up longer than the cooldown");
        server_context
            .acme
            .last_failure
            .insert(PLATFORM_ORDER_KEY.to_string(), past_the_short_cooldown);

        assert!(start_due_orders(&server_context));
        assert!(server_context.acme.in_flight.is_empty());
    }

    #[test]
    fn a_failing_platform_renewal_does_not_speed_up_custom_domain_retries() {
        let server_context = test_context();
        server_context
            .acme
            .set_config(test_config(&["eu1.revscope.co"]));
        install_expiring_in(&server_context, &["eu1.revscope.co"], 7);
        server_context
            .acme
            .last_failure
            .insert(PLATFORM_ORDER_KEY.to_string(), Instant::now());
        let past_the_short_cooldown = Instant::now()
            .checked_sub(RENEWAL_RETRY_INTERVAL + Duration::from_secs(1))
            .expect("system has been up longer than the cooldown");
        for (domain, days_left) in [("due.example.com", Some(7)), ("missing.example.com", None)] {
            server_context
                .acme
                .custom_domains
                .insert(domain.to_string(), TEST_ACCOUNT.to_string());
            if let Some(days_left) = days_left {
                install_expiring_in(&server_context, &[domain], days_left);
            }
            server_context
                .acme
                .last_failure
                .insert(domain.to_string(), past_the_short_cooldown);
        }

        assert!(start_due_orders(&server_context));
        assert!(server_context.acme.in_flight.is_empty());
    }

    #[test]
    fn a_cooling_down_custom_domain_renewal_waits_for_the_long_interval() {
        let server_context = test_context();
        server_context
            .acme
            .set_config(test_config(&["eu1.revscope.co"]));
        install_expiring_in(&server_context, &["eu1.revscope.co"], 60);
        server_context
            .acme
            .custom_domains
            .insert("assets.example.com".to_string(), TEST_ACCOUNT.to_string());
        install_expiring_in(&server_context, &["assets.example.com"], 7);
        server_context
            .acme
            .last_failure
            .insert("assets.example.com".to_string(), Instant::now());

        assert!(!start_due_orders(&server_context));
        assert!(server_context.acme.in_flight.is_empty());
    }

    #[test]
    fn saving_a_domain_does_not_replace_its_installed_certificate() {
        let server_context = test_context();
        server_context
            .acme
            .set_config(test_config(&["eu1.revscope.co"]));
        server_context
            .acme
            .custom_domains
            .insert("assets.example.com".to_string(), TEST_ACCOUNT.to_string());
        install_expiring_in(&server_context, &["assets.example.com"], 7);

        request_issuance(server_context.clone(), "assets.example.com");

        assert!(server_context.acme.in_flight.is_empty());
    }

    fn temporary_cache_dir(test_name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("revscope-acme-{test_name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create cache directory");
        dir
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn private_files_are_owner_only_and_leave_no_temporary_file() {
        use std::os::unix::fs::PermissionsExt;

        let dir = temporary_cache_dir("private-file");
        let path = dir.join("private-key.pem");

        write_private_file(&path, b"key").await.expect("write key");

        assert_eq!(std::fs::read(&path).expect("read key"), b"key");
        let mode = std::fs::metadata(&path)
            .expect("key metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(std::fs::read_dir(&dir).expect("list cache").count(), 1);
        std::fs::remove_dir_all(&dir).expect("remove cache directory");
    }

    #[tokio::test]
    async fn concurrent_writes_of_a_private_file_all_succeed() {
        let dir = temporary_cache_dir("concurrent-private-file");
        let path = dir.join("account.json");

        let writes: Vec<_> = (0..16)
            .map(|_| {
                let path = path.clone();
                tokio::spawn(async move { write_private_file(&path, b"account").await })
            })
            .collect();
        for write in writes {
            write.await.expect("join write").expect("write succeeds");
        }

        assert_eq!(std::fs::read(&path).expect("read account"), b"account");
        assert_eq!(std::fs::read_dir(&dir).expect("list cache").count(), 1);
        std::fs::remove_dir_all(&dir).expect("remove cache directory");
    }

    #[test]
    fn keeps_the_staging_cache_separate_from_production() {
        let acme_dir = PathBuf::from("data/acme");
        assert_eq!(cache_dir_for(acme_dir.clone(), false), acme_dir);
        assert_eq!(
            cache_dir_for(acme_dir.clone(), true),
            acme_dir.join("staging")
        );
    }

    #[test]
    fn normalizes_bare_email_contacts_to_mailto_uris() {
        assert_eq!(
            contact_uri("ops@revscope.co".to_string()),
            "mailto:ops@revscope.co"
        );
        assert_eq!(
            contact_uri("mailto:ops@revscope.co".to_string()),
            "mailto:ops@revscope.co"
        );
    }

    #[test]
    fn serves_the_certificate_matching_sni() {
        let server_context = test_context();
        let (api_pem, api_key) = self_signed(&["api.revscope.co"]);
        let (custom_pem, custom_key) = self_signed(&["assets.example.com"]);
        server_context
            .acme
            .certificates
            .install_pem(&name_list(&["api.revscope.co"]), &api_pem, &api_key)
            .expect("install api");
        server_context
            .acme
            .certificates
            .install_pem(
                &name_list(&["assets.example.com"]),
                &custom_pem,
                &custom_key,
            )
            .expect("install custom");

        let api = server_context
            .acme
            .resolve_sni(Some("api.revscope.co"))
            .expect("api sni");
        let custom = server_context
            .acme
            .resolve_sni(Some("assets.example.com"))
            .expect("custom sni");
        assert_ne!(api.cert, custom.cert);
        assert!(
            server_context
                .acme
                .resolve_sni(Some("unknown.example.com"))
                .is_none()
        );
    }

    #[test]
    fn missing_sni_uses_the_first_platform_domain() {
        let server_context = test_context();
        server_context
            .acme
            .set_config(test_config(&["analytics.example.com", "api.example.com"]));
        let (default_pem, default_key) = self_signed(&["analytics.example.com"]);
        let (other_pem, other_key) = self_signed(&["api.example.com"]);
        server_context
            .acme
            .certificates
            .install_pem(
                &name_list(&["analytics.example.com"]),
                &default_pem,
                &default_key,
            )
            .expect("install default");
        server_context
            .acme
            .certificates
            .install_pem(&name_list(&["api.example.com"]), &other_pem, &other_key)
            .expect("install other");

        let resolved = server_context
            .acme
            .resolve_sni(None)
            .expect("default certificate");
        let expected = server_context
            .acme
            .certificates
            .certified_key("analytics.example.com")
            .expect("installed default");
        assert_eq!(resolved.cert, expected.cert);
    }

    #[test]
    fn missing_sni_has_no_default_before_acme_is_configured() {
        let server_context = test_context();
        assert!(server_context.acme.resolve_sni(None).is_none());
    }

    #[test]
    fn reads_platform_domains_from_a_list() {
        assert_eq!(
            platform_domains_from(" Analytics.Example.com., api.example.com,analytics.example.com"),
            name_list(&["analytics.example.com", "api.example.com"])
        );
    }

    #[test]
    #[cfg(not(feature = "self-hosted"))]
    fn platform_domains_default_to_the_hosted_service() {
        assert_eq!(
            required_platform_domains(" , "),
            Ok(name_list(&["eu1.revscope.co", "api.revscope.co"]))
        );
    }

    #[test]
    #[cfg(feature = "self-hosted")]
    fn self_hosted_tls_requires_platform_domains() {
        let error = required_platform_domains(" , ").unwrap_err();
        assert!(error.contains("REVSCOPE_TLS_DOMAINS"), "{error}");
        assert_eq!(
            required_platform_domains("analytics.example.com"),
            Ok(name_list(&["analytics.example.com"]))
        );
    }

    #[test]
    fn orders_configured_platform_domains_together() {
        let state = AcmeState::new();
        state.set_config(test_config(&["analytics.example.com", "api.example.com"]));

        assert!(state.is_platform_domain("API.EXAMPLE.COM"));
        assert!(!state.is_platform_domain("api.revscope.co"));
        assert_eq!(state.order_key("api.example.com"), PLATFORM_ORDER_KEY);
        assert_eq!(
            state.domains_for("api.example.com"),
            name_list(&["analytics.example.com", "api.example.com"])
        );
        assert_eq!(
            state.cache_subdir("api.example.com"),
            PathBuf::from(PLATFORM_CACHE_DIR)
        );
        assert_eq!(state.order_key("assets.example.org"), "assets.example.org");
    }

    #[tokio::test]
    async fn waits_for_an_in_flight_certificate() {
        let server_context = test_context();
        let waiter = server_context.acme.start_test_order("assets.example.com");

        let installer = {
            let server_context = server_context.clone();
            let waiter = waiter.clone();
            tokio::spawn(async move {
                while !waiter.has_waiters() {
                    tokio::task::yield_now().await;
                }
                let (pem, key) = self_signed(&["assets.example.com"]);
                server_context
                    .acme
                    .certificates
                    .install_pem(&name_list(&["assets.example.com"]), &pem, &key)
                    .expect("install while waiting");
                waiter.finish();
            })
        };

        wait_for_handshake_certificate(&server_context, Some("assets.example.com")).await;
        tokio::time::timeout(Duration::from_secs(5), installer)
            .await
            .expect("the handshake should have waited on the order")
            .expect("installer");
        assert!(
            server_context
                .acme
                .resolve_sni(Some("assets.example.com"))
                .is_some(),
            "handshake should receive the new cert"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_handshake_without_sni_waits_on_the_platform_order() {
        let server_context = test_context();
        server_context
            .acme
            .set_config(test_config(&["eu1.revscope.co"]));
        server_context.acme.in_flight.insert(
            PLATFORM_ORDER_KEY.to_string(),
            Arc::new(IssuanceWaiter::new()),
        );
        let started = tokio::time::Instant::now();

        wait_for_handshake_certificate(&server_context, None).await;

        assert_eq!(started.elapsed(), ISSUANCE_WAIT);
    }

    #[tokio::test(start_paused = true)]
    async fn only_the_handshakes_past_the_limit_skip_the_wait() {
        let server_context = test_context();
        let _order = server_context.acme.start_test_order("assets.example.com");
        let waiting = u32::try_from(MAX_WAITING_HANDSHAKES).expect("limit fits");
        let _others = server_context
            .acme
            .handshake_waits
            .try_acquire_many(waiting - 1)
            .expect("places are free");

        let started = tokio::time::Instant::now();
        wait_for_handshake_certificate(&server_context, Some("assets.example.com")).await;
        assert_eq!(started.elapsed(), ISSUANCE_WAIT);

        let _last = server_context
            .acme
            .handshake_waits
            .try_acquire()
            .expect("one place is left");
        let started = tokio::time::Instant::now();
        wait_for_handshake_certificate(&server_context, Some("assets.example.com")).await;
        assert_eq!(started.elapsed(), Duration::ZERO);
    }

    #[tokio::test]
    async fn a_waiting_handshake_holds_its_place_until_cancelled() {
        let server_context = test_context();
        let order = server_context.acme.start_test_order("assets.example.com");
        let handshake = {
            let server_context = server_context.clone();
            tokio::spawn(async move {
                wait_for_handshake_certificate(&server_context, Some("assets.example.com")).await;
            })
        };

        tokio::time::timeout(Duration::from_secs(5), async {
            while !order.has_waiters() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the handshake should wait on the order");
        assert_eq!(
            server_context.acme.handshake_waits.available_permits(),
            MAX_WAITING_HANDSHAKES - 1
        );

        handshake.abort();
        let _ = handshake.await;
        assert_eq!(
            server_context.acme.handshake_waits.available_permits(),
            MAX_WAITING_HANDSHAKES
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_handshake_past_the_limit_still_starts_the_order() {
        let server_context = test_context();
        server_context
            .acme
            .set_config(test_config(&["eu1.revscope.co"]));
        point_at_platform(&server_context, "assets.example.com");
        let waiting = u32::try_from(MAX_WAITING_HANDSHAKES).expect("limit fits");
        let _others = server_context
            .acme
            .handshake_waits
            .try_acquire_many(waiting)
            .expect("places are free");

        wait_for_handshake_certificate(&server_context, Some("assets.example.com")).await;

        assert!(
            server_context
                .acme
                .in_flight
                .contains_key("assets.example.com")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_handshake_for_an_unknown_name_starts_no_order() {
        let server_context = test_context();
        server_context
            .acme
            .set_config(test_config(&["eu1.revscope.co"]));

        wait_for_handshake_certificate(&server_context, Some("unknown.example.com")).await;

        assert!(server_context.acme.in_flight.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_handshake_during_the_cooldown_starts_no_order() {
        let server_context = test_context();
        server_context
            .acme
            .set_config(test_config(&["eu1.revscope.co"]));
        server_context
            .acme
            .custom_domains
            .insert("assets.example.com".to_string(), TEST_ACCOUNT.to_string());
        server_context
            .acme
            .last_failure
            .insert("assets.example.com".to_string(), Instant::now());

        wait_for_handshake_certificate(&server_context, Some("assets.example.com")).await;

        assert!(server_context.acme.in_flight.is_empty());
    }

    #[test]
    fn resolving_a_missing_certificate_neither_waits_nor_orders() {
        let server_context = test_context();
        server_context
            .acme
            .set_config(test_config(&["eu1.revscope.co"]));

        assert!(server_context.acme.resolve_sni(None).is_none());
        assert!(server_context.acme.in_flight.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn issuance_waiter_times_out_when_the_order_never_finishes() {
        let waiter = IssuanceWaiter::new();
        assert!(!waiter.wait_timeout(ISSUANCE_WAIT).await);
    }

    #[tokio::test(start_paused = true)]
    async fn issuance_waiter_returns_at_once_after_the_order_finished() {
        let waiter = IssuanceWaiter::new();
        waiter.finish();
        assert!(waiter.wait_timeout(Duration::ZERO).await);
    }

    #[test]
    fn treats_revscope_hostnames_as_reserved() {
        assert!(is_revscope_hostname("assets.revscope.co"));
        assert!(is_revscope_hostname("revscope.co"));
        assert!(!is_revscope_hostname("assets.example.com"));
    }

    #[test]
    fn poll_timeout_is_distinct_from_acme_failures() {
        let error = AcmeError::from(instant_acme::Error::Timeout(None));
        assert!(matches!(error, AcmeError::Timeout));
        assert_eq!(error.to_string(), "timed out waiting for the ACME server");
    }

    fn http01_challenge(error: Option<&str>) -> Challenge {
        let error_json = match error {
            Some(error) => format!(r#", "error": {error}"#),
            None => String::new(),
        };
        serde_json::from_str(&format!(
            r#"{{
                "type": "http-01",
                "url": "https://example.com/acme/chall/1",
                "status": "invalid",
                "token": "abc"{error_json}
            }}"#
        ))
        .expect("challenge json")
    }

    #[test]
    fn formats_http01_challenge_problem_with_the_hostname() {
        let challenge = http01_challenge(Some(
            r#"{"type":"urn:ietf:params:acme:error:unauthorized","detail":"Invalid response from http://api.revscope.co/.well-known/acme-challenge/abc: 404"}"#,
        ));
        let message = format_failed_authorization(
            "api.revscope.co",
            AuthorizationStatus::Invalid,
            &[challenge],
        )
        .expect("challenge error");
        assert!(
            message.starts_with("api.revscope.co: Invalid response from http://api.revscope.co/"),
            "{message}"
        );
        assert!(
            message.contains("urn:ietf:params:acme:error:unauthorized"),
            "{message}"
        );
    }

    #[test]
    fn formats_invalid_authorization_when_the_challenge_has_no_problem() {
        let challenge = http01_challenge(None);
        let message = format_failed_authorization(
            "eu1.revscope.co",
            AuthorizationStatus::Invalid,
            &[challenge],
        )
        .expect("authorization status");
        assert_eq!(message, "eu1.revscope.co: authorization Invalid");
    }

    #[test]
    fn skips_valid_authorizations_without_challenge_errors() {
        let challenge = http01_challenge(None);
        assert_eq!(
            format_failed_authorization(
                "eu1.revscope.co",
                AuthorizationStatus::Valid,
                &[challenge]
            ),
            None
        );
    }
}
