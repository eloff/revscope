use std::io;
use std::sync::Arc;
use std::time::Duration;

use log::{info, warn};
use revscope::socket_cleanup::terminate_processes_listening_on_addr;
use sentry::integrations::log::{LogFilter, SentryLogger};
use tokio::sync::Notify;
use tokio::task::JoinError;

fn main() -> io::Result<()> {
    // Load .env before Sentry so SENTRY_DSN can come from the file.
    let _ = dotenv::dotenv();
    if let Err(error) = revscope::require_billing_when_configured() {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
    if let Err(error) = revscope::require_integration_encryption_key() {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }

    // Must run before any rustls client is built or a JWT is signed. rustls
    // and jsonwebtoken keep separate process defaults, and neither will pick
    // aws-lc-rs on its own in this binary.
    revscope::install_default_crypto_provider();

    // Init Sentry before the Tokio runtime so worker and blocking-pool threads
    // inherit a Hub with the client bound (required for panic capture).
    let _sentry = init_sentry();

    let mut env_logger = env_logger::Builder::from_default_env();
    // default_filter_or only covers an unset RUST_LOG; an empty one, as an
    // uncommented `RUST_LOG=` in .env gives, would otherwise mean error.
    if std::env::var(env_logger::DEFAULT_FILTER_ENV).map_or(true, |value| value.trim().is_empty()) {
        env_logger.parse_filters("warn");
    }
    let env_logger = env_logger.format_timestamp_millis().build();
    // error! -> Sentry issue; lower levels are local-only (env_logger still prints them).
    let logger = SentryLogger::with_dest(env_logger).filter(|md| match md.level() {
        log::Level::Error => LogFilter::Exception,
        _ => LogFilter::Ignore,
    });
    log::set_boxed_logger(Box::new(logger)).expect("set logger");
    log::set_max_level(log::LevelFilter::Trace);
    // Before the server context is built, which fits the connection cap to
    // this limit.
    revscope::raise_open_file_limit();

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async_main())
}

fn init_sentry() -> sentry::ClientInitGuard {
    let dsn = std::env::var("SENTRY_DSN")
        .ok()
        .filter(|value| !value.trim().is_empty());
    sentry::init((
        dsn,
        sentry::ClientOptions {
            release: sentry::release_name!(),
            attach_stacktrace: true,
            ..Default::default()
        },
    ))
}

async fn async_main() -> io::Result<()> {
    revscope::db::migrations::initialize_available_migrations().map_err(std::io::Error::other)?;
    revscope::initialize_ip_ranges().map_err(std::io::Error::other)?;
    let server_context = revscope::db::default_server_context().clone();

    // Apply main database migrations before binding the listener, so this process serves
    // nothing until its schema is ready. An existing server may still be running on the old one.
    revscope::db::apply_main_migrations(server_context.clone())
        .await
        .map_err(io::Error::other)?;

    // We use SO_REUSEPORT so that we can listen on the same port as any existing revscope server
    let listen_addr = std::env::var("REVSCOPE_LISTEN").unwrap_or_else(|_| "127.0.0.1:4320".into());
    let listener = revscope::bind_listener(&listen_addr)?;

    // HTTPS is served only when REVSCOPE_LISTEN_TLS is set. Deployments behind
    // a proxy that terminates TLS leave it unset. The TLS listener uses
    // SO_REUSEPORT too, so an upgrade hands over 443 the same way as 80.
    let tls_listen_addr = std::env::var("REVSCOPE_LISTEN_TLS")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let acme_config = tls_listen_addr
        .as_ref()
        .map(|_| revscope::AcmeConfig::from_env())
        .transpose()
        .map_err(io::Error::other)?;
    if let Some(config) = &acme_config {
        revscope::prepare_acme(server_context.clone(), config).await;
    }
    let tls_listener = match &tls_listen_addr {
        Some(addr) => Some(revscope::bind_listener(addr)?),
        None => None,
    };

    // Once we've bound the listeners, we signal any other process listening on the same ports to exit gracefully
    // This allows us to do a zero downtime upgrade by just running the new binary on the server.
    let mut previous_pids = Vec::new();
    let mut handover_addrs = vec![listen_addr.as_str()];
    if let Some(addr) = tls_listen_addr.as_deref() {
        handover_addrs.push(addr);
    }
    for addr in handover_addrs {
        let terminated_pids = terminate_processes_listening_on_addr(addr)?;
        if !terminated_pids.is_empty() {
            info!("terminated processes listening on {addr}: {terminated_pids:?}");
        }
        previous_pids.extend(terminated_pids);
    }

    revscope::db::start_db_cache_expiry_task();
    revscope::start_integration_sync_task(server_context.clone());
    revscope::start_ga_import_task(server_context.clone());
    revscope::start_main_db_cleanup_task(server_context.clone());
    revscope::start_download_snapshot_cleanup_task(server_context.clone());
    revscope::start_project_db_sweep_task(server_context.clone(), previous_pids);

    let listener_shutdown = Arc::new(Notify::new());
    // At warn so the address shows at the default log level.
    warn!("revscope listening on {listen_addr}");
    let mut server_task = revscope::task::spawn(
        server_context.clone(),
        revscope::run_listener(
            listener,
            revscope::ServerHooks::default(),
            listener_shutdown.clone(),
            server_context.clone(),
        ),
    );

    let mut tls_server_task = tls_listener.map(|listener| {
        let addr = tls_listen_addr.as_deref().expect("tls listen address");
        warn!("revscope listening for https on {addr}");
        revscope::task::spawn(
            server_context.clone(),
            revscope::run_tls_listener(
                listener,
                revscope::ServerHooks::default(),
                listener_shutdown.clone(),
                server_context.clone(),
            ),
        )
    });

    // Started after the listeners so the HTTP-01 challenge is already reachable
    // by the time the first order asks the ACME server to validate it.
    if let Some(config) = acme_config {
        warn_if_http_challenge_is_unreachable(&listen_addr, &config);
        revscope::start_acme_task(server_context.clone(), config);
    }

    let serve_outcome = tokio::select! {
        result = &mut server_task => ServeOutcome::Listener(join_server_task(result)),
        result = join_optional_listener(&mut tls_server_task) => ServeOutcome::Listener(result),
        signal_result = wait_for_sigterm() => ServeOutcome::Sigterm(signal_result),
    };

    let listener_result = match serve_outcome {
        ServeOutcome::Listener(result) => result,
        ServeOutcome::Sigterm(result) => {
            result?;
            info!("received SIGTERM, shutting down");
            begin_shutdown(&server_context, &listener_shutdown);
            let plain_result = join_server_task(server_task.await);
            let tls_result = match tls_server_task {
                Some(task) => join_server_task(task.await),
                None => Ok(()),
            };
            plain_result.and(tls_result)
        }
    };

    begin_shutdown(&server_context, &listener_shutdown);
    wait_for_active_tasks_to_drain(&server_context).await;
    // Flushing buffered data can miss rows enqueued concurrently; once
    // request handlers have finished, flush anything still buffered that we missed.
    revscope::drain_ingest_on_shutdown(server_context).await;
    listener_result
}

enum ServeOutcome {
    Listener(io::Result<()>),
    Sigterm(io::Result<()>),
}

fn begin_shutdown(
    server_context: &revscope::server_context::ServerContext,
    listener_shutdown: &Notify,
) {
    server_context.request_shutdown();
    listener_shutdown.notify_waiters();
}

async fn wait_for_active_tasks_to_drain(server_context: &revscope::server_context::ServerContext) {
    while server_context.active_task_count() > 0 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn join_server_task(result: Result<io::Result<()>, JoinError>) -> io::Result<()> {
    match result {
        Ok(result) => result,
        Err(error) => Err(io::Error::other(error)),
    }
}

/// Awaits the TLS listener when there is one, and otherwise never resolves so
/// the surrounding `select!` only watches the listeners that exist.
async fn join_optional_listener(
    task: &mut Option<tokio::task::JoinHandle<io::Result<()>>>,
) -> io::Result<()> {
    match task.as_mut() {
        Some(task) => join_server_task(task.await),
        None => std::future::pending().await,
    }
}

/// Let's Encrypt only follows HTTP-01 redirects from port 80, so ACME cannot
/// finish an order unless the plain listener is reachable there.
fn warn_if_http_challenge_is_unreachable(listen_addr: &str, config: &revscope::AcmeConfig) {
    let port = listen_addr
        .parse::<std::net::SocketAddr>()
        .map(|addr| addr.port());
    if port != Ok(80) {
        let cname_target = config.platform_domains().first().map_or("", String::as_str);
        warn!(
            "REVSCOPE_LISTEN is {listen_addr}; ACME http-01 validation needs \
             http://<domain>/.well-known/acme-challenge/ to reach this server on port 80. \
             Custom domains must CNAME to {cname_target} with DNS-only (not proxied) records."
        );
    }
}

#[cfg(unix)]
async fn wait_for_sigterm() -> io::Result<()> {
    let mut signal = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let _ = signal.recv().await;
    Ok(())
}

#[cfg(not(unix))]
async fn wait_for_sigterm() -> io::Result<()> {
    tokio::signal::ctrl_c().await
}
