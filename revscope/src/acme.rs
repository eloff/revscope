use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tokio::io::AsyncWriteExt;

use dashmap::DashMap;
use dashmap::DashSet;
use instant_acme::{
    Account, AccountCredentials, AuthorizationStatus, Challenge, ChallengeType, Identifier,
    LetsEncrypt, NewAccount, NewOrder, Order, OrderStatus, Problem, RetryPolicy,
};
use log::{error, info, warn};
use parking_lot::{Condvar, Mutex, RwLock};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;

use crate::db::open_main_database_read;
use crate::http::Response;
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
/// How long a TLS handshake may block waiting for an in-flight HTTP-01 order.
/// Kept under typical browser TLS timeouts; the settings save already starts
/// issuance so this is the remainder, not a full order from a cold start.
const ISSUANCE_WAIT: Duration = Duration::from_secs(25);
/// How long to poll Let's Encrypt for one order to become ready, and again for
/// the certificate after finalize. Handshake waiters use [`ISSUANCE_WAIT`] and
/// return sooner; this is the background order, not the first connection.
const ORDER_WAIT: Duration = Duration::from_secs(5 * 60);
/// Limit on a whole order. instant-acme sets no timeout on its requests, so a
/// stalled connection would otherwise keep the order in flight until a restart,
/// and every later check would join it instead of retrying.
const ORDER_DEADLINE: Duration = Duration::from_secs(15 * 60);

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
    /// Customer CNAME hostnames currently stored in `project_settings`.
    custom_domains: DashSet<String>,
    /// In-flight ACME orders, keyed by [`PLATFORM_ORDER_KEY`] or the custom
    /// hostname, so concurrent handshakes wait on one order.
    in_flight: DashMap<String, Arc<IssuanceWaiter>>,
    /// Last failed order per key, so a broken CNAME cannot retry on every
    /// handshake.
    last_failure: DashMap<String, Instant>,
    config: RwLock<Option<AcmeConfig>>,
}

impl AcmeState {
    pub(crate) fn new() -> Self {
        Self {
            challenges: DashMap::new(),
            certificates: Arc::new(CertificateStore::new()),
            custom_domains: DashSet::new(),
            in_flight: DashMap::new(),
            last_failure: DashMap::new(),
            config: RwLock::new(None),
        }
    }

    pub(crate) fn certificates(&self) -> &Arc<CertificateStore> {
        &self.certificates
    }

    pub(crate) fn set_config(&self, config: AcmeConfig) {
        *self.config.write() = Some(config);
    }

    pub(crate) fn replace_custom_domain(&self, previous: Option<&str>, next: Option<&str>) {
        if let Some(previous) = previous {
            self.custom_domains.remove(previous);
        }
        if let Some(next) = next {
            self.custom_domains.insert(next.to_string());
        }
    }

    pub(crate) fn set_custom_domains(&self, domains: impl IntoIterator<Item = String>) {
        self.custom_domains.clear();
        for domain in domains {
            self.custom_domains.insert(domain);
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
        self.is_platform_domain(name) || self.custom_domains.contains(name)
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

    /// Picks the certificate for this handshake. Missing SNI uses the first
    /// platform domain. If the name is allowed but the certificate is not
    /// installed yet, blocks up to [`ISSUANCE_WAIT`] while HTTP-01 completes.
    pub(crate) fn resolve_sni(
        &self,
        server_context: &Arc<ServerContext>,
        server_name: Option<&str>,
    ) -> Option<Arc<CertifiedKey>> {
        let name = match server_name
            .map(normalize_sni)
            .filter(|name| !name.is_empty())
        {
            Some(name) => name,
            None => self.platform_domains().into_iter().next()?,
        };
        if let Some(key) = self.certificates.certified_key(&name) {
            return Some(key);
        }
        if !self.is_allowed(&name) {
            return None;
        }
        wait_for_certificate(server_context, &name)
    }
}

/// Waiter a handshake parks on while an ACME order for that name is running.
struct IssuanceWaiter {
    finished: Mutex<bool>,
    condvar: Condvar,
}

impl IssuanceWaiter {
    fn new() -> Self {
        Self {
            finished: Mutex::new(false),
            condvar: Condvar::new(),
        }
    }

    fn wait_timeout(&self, timeout: Duration) -> bool {
        let mut finished = self.finished.lock();
        if *finished {
            return true;
        }
        self.condvar.wait_for(&mut finished, timeout);
        *finished
    }

    fn finish(&self) {
        let mut finished = self.finished.lock();
        *finished = true;
        self.condvar.notify_all();
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
            .resolve_sni(&self.server_context, client_hello.server_name())
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
    match load_custom_domains(server_context.clone()).await {
        Ok(domains) => {
            let count = domains.len();
            server_context.acme.set_custom_domains(domains);
            if count > 0 {
                info!("loaded {count} custom ACME domain(s) from project settings");
            }
        }
        Err(error) => warn!("unable to load custom ACME domains: {error}"),
    }
    load_cached_certificates(&server_context, config);
}

/// Installs any cached certificate, then starts the task that orders one over
/// ACME HTTP-01 and renews it before it expires.
pub fn start_acme_task(server_context: Arc<ServerContext>, config: AcmeConfig) {
    server_context.acme.set_config(config);
    crate::task::spawn(server_context.clone(), renewal_loop(server_context));
}

/// Starts an ACME order for `domain` if one is not already running. Used when
/// a project saves a custom domain so the first visitor does not have to wait
/// out the whole issuance.
pub(crate) fn request_issuance(server_context: Arc<ServerContext>, domain: &str) {
    if server_context.acme.config.read().is_none() {
        return;
    }
    let _ = order_if_missing(&server_context, domain);
}

fn wait_for_certificate(
    server_context: &Arc<ServerContext>,
    name: &str,
) -> Option<Arc<CertifiedKey>> {
    if let Some(key) = server_context.acme.certificates.certified_key(name) {
        return Some(key);
    }
    let order_key = server_context.acme.order_key(name);
    if let Some(failed_at) = server_context.acme.last_failure.get(&order_key)
        && server_context.acme.in_flight.get(&order_key).is_none()
        && failed_at.elapsed() < RENEWAL_RETRY_INTERVAL
    {
        return None;
    }
    let Some(waiter) = order_if_missing(server_context, name) else {
        return server_context.acme.certificates.certified_key(name);
    };
    waiter.wait_timeout(ISSUANCE_WAIT);
    server_context.acme.certificates.certified_key(name)
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
            provision_certificate(&server_context, &config, &domains, &cache_subdir),
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
            let renewing = domains.iter().any(|domain| {
                server_context
                    .acme
                    .certificates
                    .certified_key(domain)
                    .is_some()
            });
            if renewing {
                server_context
                    .acme
                    .last_failure
                    .insert(order_key.clone(), Instant::now());
            }
            warn!("ACME order for {names} timed out; will retry");
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

fn finish_order(server_context: &ServerContext, order_key: &str, _succeeded: bool) {
    if let Some((_, waiter)) = server_context.acme.in_flight.remove(order_key) {
        waiter.finish();
    }
}

async fn renewal_loop(server_context: Arc<ServerContext>) {
    loop {
        match load_custom_domains(server_context.clone()).await {
            Ok(domains) => server_context.acme.set_custom_domains(domains),
            Err(error) => warn!("unable to refresh custom ACME domains: {error}"),
        }

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
    server_context
        .acme
        .last_failure
        .get(&server_context.acme.order_key(name))
        .is_some_and(|failed_at| failed_at.elapsed() < wait)
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

fn load_cached_certificates(server_context: &ServerContext, config: &AcmeConfig) {
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

    for domain in server_context.acme.custom_domains.iter() {
        let domain = domain.key().clone();
        let cache_dir = config.cache_dir.join(CUSTOM_CACHE_DIR).join(&domain);
        match load_cached_certificate(
            &cache_dir,
            std::slice::from_ref(&domain),
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

async fn load_custom_domains(server_context: Arc<ServerContext>) -> Result<Vec<String>, AcmeError> {
    let conn = open_main_database_read(server_context)
        .await
        .map_err(|error| AcmeError::Database(error.to_string()))?;
    let rows = conn
        .query(
            "SELECT custom_domain FROM project_settings WHERE custom_domain IS NOT NULL",
            &[],
        )
        .await
        .map_err(|error| AcmeError::Database(error.to_string()))?;
    Ok(rows
        .iter()
        .filter_map(|row| row.get::<_, Option<String>>(0))
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
    Authorization(String),
    /// The cached certificate or key is not valid PEM text.
    CachedPemNotUtf8,
    Certificate(CertificateError),
    Credentials(serde_json::Error),
    Database(String),
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
            Self::Authorization(status) => {
                write!(formatter, "unusable authorization status: {status}")
            }
            Self::CachedPemNotUtf8 => write!(formatter, "cached certificate is not valid PEM"),
            Self::Certificate(error) => write!(formatter, "{error}"),
            Self::Credentials(error) => write!(formatter, "invalid account credentials: {error}"),
            Self::Database(error) => write!(formatter, "{error}"),
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
                server_context.acme.custom_domains.insert(name.to_string());
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
            .resolve_sni(&server_context, Some(names[0]))
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

        load_cached_certificates(&server_context, &config);
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
                .insert(domain.to_string());
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
            .insert("assets.example.com".to_string());
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
            .insert("assets.example.com".to_string());
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
            .resolve_sni(&server_context, Some("api.revscope.co"))
            .expect("api sni");
        let custom = server_context
            .acme
            .resolve_sni(&server_context, Some("assets.example.com"))
            .expect("custom sni");
        assert_ne!(api.cert, custom.cert);
        assert!(
            server_context
                .acme
                .resolve_sni(&server_context, Some("unknown.example.com"))
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
            .resolve_sni(&server_context, None)
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
        assert!(
            server_context
                .acme
                .resolve_sni(&server_context, None)
                .is_none()
        );
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

    #[test]
    fn waits_for_an_in_flight_certificate_up_to_the_time_limit() {
        let server_context = test_context();
        server_context
            .acme
            .custom_domains
            .insert("assets.example.com".to_string());
        let waiter = Arc::new(IssuanceWaiter::new());
        server_context
            .acme
            .in_flight
            .insert("assets.example.com".to_string(), waiter.clone());

        let installer = {
            let server_context = server_context.clone();
            let waiter = waiter.clone();
            std::thread::spawn(move || {
                let (pem, key) = self_signed(&["assets.example.com"]);
                server_context
                    .acme
                    .certificates
                    .install_pem(&name_list(&["assets.example.com"]), &pem, &key)
                    .expect("install while waiting");
                waiter.finish();
            })
        };

        let resolved = server_context
            .acme
            .resolve_sni(&server_context, Some("assets.example.com"));
        installer.join().expect("installer");
        assert!(resolved.is_some(), "handshake should receive the new cert");
    }

    #[test]
    fn issuance_waiter_times_out_when_the_order_never_finishes() {
        let waiter = IssuanceWaiter::new();
        assert!(!waiter.wait_timeout(Duration::from_millis(20)));
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
