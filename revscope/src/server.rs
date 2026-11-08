use std::io;
use std::io::SeekFrom;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use log::{error, info, warn};
use rustls::ServerConfig;
use rustls::server::Acceptor;
use socket2::{Domain, Socket, Type};
use tokio::fs::File;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Notify, mpsc};
use tokio::time::timeout;
use tokio_rustls::LazyConfigAcceptor;
use tokio_rustls::server::TlsStream;

use crate::db::default_server_context;
use crate::http::{
    BadFraming, FileBody, ReadRequestError, Response, cors_headers_for_request,
    forwarded_headers_trusted, ingest_preflight_response, is_trusted_forwarding_proxy,
    log_completed_request, parse_request_head, parse_status_line, preflight_response,
    read_request_with_limits, redact_sensitive_query, request_body, request_client_ip,
    trusted_cf_ipcountry, with_cors_headers, with_ingest_cors_headers,
};
use crate::ingest::{extract_payload, handle_ingest_request};
use crate::routes::{
    ApiRoute, IngestRoute, handle_api_route, request_session_key, resolve_api_route,
    resolve_ingest_route,
};
use crate::server_context::ServerContext;

const MAX_REQUEST_SIZE: usize = 128 * 1024;
const MAX_REQUEST_TIME: Duration = Duration::from_secs(2);
/// How long a peer may take to complete the TLS handshake. This includes the
/// bounded wait for an in-flight ACME order when a new custom domain is first
/// used. Anything slower is dropped rather than left holding a task, since the
/// public internet dials port 443 constantly without intending to speak TLS.
const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
/// Bounds on closing a connection, so a peer that keeps sending or has stopped
/// reading can't hold the connection slot; past them it may get a reset after
/// all.
const LINGER_TIMEOUT: Duration = Duration::from_secs(2);
const LINGER_MAX_BYTES: usize = 1024 * 1024;
/// How long a keep-alive connection may sit idle waiting for the next request.
/// Matches Cloudflare's proxy idle timeout so CF can reuse origin connections.
const KEEP_ALIVE_IDLE_TIMEOUT: Duration = Duration::from_secs(900);
/// How long the accept loop pauses after `accept` fails.
const ACCEPT_ERROR_BACKOFF: Duration = Duration::from_millis(100);
/// Least time between `error!` reports while `accept` keeps running out of
/// descriptors or memory.
const ACCEPT_EXHAUSTION_REPORT_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Clone, Default)]
pub struct ServerHooks {
    payload_sender: Option<mpsc::UnboundedSender<Vec<u8>>>,
}

impl ServerHooks {
    pub fn with_payload_sender(payload_sender: mpsc::UnboundedSender<Vec<u8>>) -> Self {
        Self {
            payload_sender: Some(payload_sender),
        }
    }
}

pub fn bind_listener(listen_addr: &str) -> io::Result<TcpListener> {
    let addr: SocketAddr = listen_addr
        .parse()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    let socket = Socket::new(Domain::for_address(addr), Type::STREAM, None)?;
    socket.set_reuse_port(true)?;
    socket.set_reuse_address(true)?;
    socket.bind(&addr.into())?;
    socket.listen(128)?;
    let listener: std::net::TcpListener = socket.into();
    listener.set_nonblocking(true)?;
    TcpListener::from_std(listener)
}

pub async fn run_server(listen_addr: &str) -> io::Result<()> {
    run_server_with_hooks(
        listen_addr,
        ServerHooks::default(),
        Arc::new(Notify::new()),
        default_server_context().clone(),
    )
    .await
}

pub async fn run_server_with_hooks(
    listen_addr: &str,
    hooks: ServerHooks,
    shutdown: Arc<Notify>,
    server_context: Arc<ServerContext>,
) -> io::Result<()> {
    let listener = bind_listener(listen_addr)?;
    run_listener(listener, hooks, shutdown, server_context).await
}

pub async fn run_listener(
    listener: TcpListener,
    hooks: ServerHooks,
    shutdown: Arc<Notify>,
    server_context: Arc<ServerContext>,
) -> io::Result<()> {
    accept_loop(listener, None, hooks, shutdown, server_context).await
}

/// Serves HTTPS using the certificate the ACME task keeps current. Handshakes
/// fail until the first certificate is installed, which is why the plain
/// listener answers the HTTP-01 challenge that obtains it.
pub async fn run_tls_listener(
    listener: TcpListener,
    hooks: ServerHooks,
    shutdown: Arc<Notify>,
    server_context: Arc<ServerContext>,
) -> io::Result<()> {
    let tls_config = crate::tls::server_config(crate::acme::sni_resolver(server_context.clone()));
    accept_loop(listener, Some(tls_config), hooks, shutdown, server_context).await
}

async fn accept_loop(
    listener: TcpListener,
    tls_config: Option<Arc<ServerConfig>>,
    hooks: ServerHooks,
    shutdown: Arc<Notify>,
    server_context: Arc<ServerContext>,
) -> io::Result<()> {
    crate::ingest::start_ingest_flush_scheduler(server_context.clone());
    // One future for the whole loop, registered before the first accept, since
    // `notify_waiters` only wakes futures that already exist and one made per
    // iteration misses a shutdown that lands between them.
    let shutdown_notified = shutdown.notified();
    tokio::pin!(shutdown_notified);
    shutdown_notified.as_mut().enable();
    // Throttled by time rather than reset on success, because each closing
    // connection lets one accept through mid-squeeze, which would otherwise
    // re-report it to Sentry up to ten times a second.
    let mut last_exhaustion_report: Option<Instant> = None;
    loop {
        tokio::select! {
            accept_result = listener.accept() => {
                let (stream, peer_addr) = match accept_result {
                    Ok(accepted) => accepted,
                    Err(error) => match classify_accept_error(&error) {
                        AcceptError::PerConnection => {
                            info!("accept dropped a connection: {error}");
                            continue;
                        }
                        // Returning would shut the whole server down, so wait
                        // for connections to close instead of spinning.
                        AcceptError::OutOfResources => {
                            if last_exhaustion_report
                                .is_none_or(|reported| reported.elapsed() >= ACCEPT_EXHAUSTION_REPORT_INTERVAL)
                            {
                                error!("accept failed, retrying every {ACCEPT_ERROR_BACKOFF:?} until it recovers: {error}");
                                last_exhaustion_report = Some(Instant::now());
                            }
                            tokio::select! {
                                _ = tokio::time::sleep(ACCEPT_ERROR_BACKOFF) => continue,
                                _ = &mut shutdown_notified => return Ok(()),
                            }
                        }
                        // main only prints the returned error to stderr.
                        AcceptError::Fatal => {
                            error!("accept failed, stopping the listener: {error}");
                            return Err(error);
                        }
                    },
                };
                // Over the cap, dropping the stream closes it before any TLS
                // handshake or read.
                let Some(connection_slot) = server_context.try_acquire_connection_slot() else {
                    drop(stream);
                    continue;
                };
                let connection_hooks = hooks.clone();
                let connection_context = server_context.clone();
                let connection_tls_config = tls_config.clone();
                crate::task::spawn(server_context.clone(), async move {
                    let _connection_slot = connection_slot;
                    let result = match connection_tls_config {
                        Some(tls_config) => serve_tls_connection(
                            tls_config,
                            stream,
                            peer_addr,
                            connection_hooks,
                            connection_context,
                        )
                        .await,
                        None => {
                            let mut stream = stream;
                            handle_connection(
                                &mut stream,
                                Some(peer_addr.ip()),
                                true,
                                connection_hooks,
                                connection_context,
                            )
                            .await
                        }
                    };
                    if let Err(error) = result {
                        if is_benign_connection_error(&error) {
                            info!("connection closed from {peer_addr}: {error}");
                        } else {
                            error!("connection error from {peer_addr}: {error}");
                        }
                    }
                });
            }
            _ = &mut shutdown_notified => {
                return Ok(());
            }
        }
    }
}

#[derive(Debug, PartialEq)]
enum AcceptError {
    /// The peer went away or the network hiccuped; only this connection is lost.
    PerConnection,
    /// Out of descriptors or memory, which closing connections will relieve.
    OutOfResources,
    /// The listener itself is broken, so retrying would never succeed.
    Fatal,
}

fn classify_accept_error(error: &io::Error) -> AcceptError {
    match error.kind() {
        io::ErrorKind::ConnectionAborted
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::Interrupted
        | io::ErrorKind::WouldBlock => return AcceptError::PerConnection,
        io::ErrorKind::OutOfMemory => return AcceptError::OutOfResources,
        _ => {}
    }
    #[cfg(unix)]
    if let Some(code) = error.raw_os_error() {
        // ENOSPC comes from tokio registering the accepted socket with epoll
        // once `max_user_watches` is used up; the socket is already closed.
        if [
            libc::EMFILE,
            libc::ENFILE,
            libc::ENOBUFS,
            libc::ENOMEM,
            libc::ENOSPC,
        ]
        .contains(&code)
        {
            return AcceptError::OutOfResources;
        }
        // accept(2) on Linux passes these pending network errors through and
        // says to retry them like EAGAIN. EPERM is a firewall rejecting the
        // connection.
        if [
            libc::EPROTO,
            libc::EPERM,
            libc::ENETDOWN,
            libc::ENOPROTOOPT,
            libc::EHOSTDOWN,
            libc::EHOSTUNREACH,
            libc::EOPNOTSUPP,
            libc::ENETUNREACH,
        ]
        .contains(&code)
        {
            return AcceptError::PerConnection;
        }
        #[cfg(target_os = "linux")]
        if code == libc::ENONET {
            return AcceptError::PerConnection;
        }
    }
    AcceptError::Fatal
}

/// Reads the ClientHello before committing to rustls, so a handshake for a
/// certificate still being issued waits on this task rather than in rustls's
/// synchronous resolver.
async fn accept_tls(
    tls_config: Arc<ServerConfig>,
    stream: TcpStream,
    server_context: &Arc<ServerContext>,
) -> io::Result<TlsStream<TcpStream>> {
    let handshake = LazyConfigAcceptor::new(Acceptor::default(), stream).await?;
    let server_name = handshake.client_hello().server_name().map(str::to_owned);
    crate::acme::wait_for_handshake_certificate(server_context, server_name.as_deref()).await;
    handshake.into_stream(tls_config).await
}

/// Completes the TLS handshake, then hands the decrypted stream to the same
/// request loop the plain listener uses.
async fn serve_tls_connection(
    tls_config: Arc<ServerConfig>,
    stream: TcpStream,
    peer_addr: SocketAddr,
    hooks: ServerHooks,
    server_context: Arc<ServerContext>,
) -> io::Result<()> {
    // A failed handshake is routine on port 443 (port scans, clients that
    // reject our certificate, connections opened before the first certificate
    // is issued), so it is logged rather than reported as a server error.
    let handshake = accept_tls(tls_config, stream, &server_context);
    let mut stream = match timeout(TLS_HANDSHAKE_TIMEOUT, handshake).await {
        Ok(Ok(stream)) => stream,
        Ok(Err(error)) => {
            info!("TLS handshake from {peer_addr} failed: {error}");
            return Ok(());
        }
        Err(_) => {
            info!("TLS handshake from {peer_addr} timed out");
            return Ok(());
        }
    };

    serve_then_close_notify(&mut stream, peer_addr.ip(), hooks, server_context).await
}

/// Generic over the stream so a test can stall the close.
async fn serve_then_close_notify<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    peer_ip: IpAddr,
    hooks: ServerHooks,
    server_context: Arc<ServerContext>,
) -> io::Result<()> {
    let result = handle_connection(stream, Some(peer_ip), false, hooks, server_context).await;
    // Best effort close_notify: it tells the peer the stream ended on purpose,
    // but the peer is often gone by the time we get here.
    shutdown_within_linger(stream).await;
    result
}

/// Client-side disconnects (reset/abort/broken pipe/stalled) are normal on the
/// public internet; log them at info so they do not become Sentry issues via
/// error!. rustls also reports a missing TLS close_notify as UnexpectedEof; HTTP here
/// is length-framed, so that is the same as a clean hangup. Other UnexpectedEof
/// values (for example a file that shrinks under Content-Length) stay errors.
fn is_benign_connection_error(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::BrokenPipe
    ) || is_peer_write_stall(error)
        || is_tls_eof_without_close_notify(error)
}

/// Excludes OS errors because a disk read can also report ETIMEDOUT, and that
/// is our fault rather than the client's.
fn is_peer_write_stall(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::TimedOut && error.raw_os_error().is_none()
}

fn is_tls_eof_without_close_notify(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::UnexpectedEof && error.to_string().contains("close_notify")
}

/// Serves requests off one connection until either side closes it. The stream
/// is generic so a TLS connection follows exactly the same request path as a
/// plain one.
///
/// `connection_ip` is the peer address from `accept`. Which forwarded headers
/// are believed is decided per request by [`forwarded_headers_trusted`].
///
/// `plain_http` is true on port 80: with `require_cloudflare` set, non-proxy
/// peers may only fetch ACME HTTP-01 challenges. TLS (443) serves anyone;
/// Let's Encrypt does not use HTTPS for HTTP-01.
async fn handle_connection<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    connection_ip: Option<IpAddr>,
    plain_http: bool,
    hooks: ServerHooks,
    server_context: Arc<ServerContext>,
) -> io::Result<()> {
    let end = serve_requests(stream, connection_ip, plain_http, hooks, server_context).await?;
    if end == ConnectionEnd::ClosedByServer {
        lingering_close(stream).await;
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
enum ConnectionEnd {
    /// The peer hung up or went idle; nothing of theirs is left to read.
    PeerDone,
    /// We answered and chose to close, possibly with request bytes unread.
    ClosedByServer,
}

/// Closes a connection we chose to end without discarding the response.
///
/// Closing a TCP socket with unread bytes in its receive buffer sends RST, and
/// a client that gets the RST before reading our response (say a 413 or 501
/// sent while it is still uploading) sees a reset instead. Shutting down our
/// write side first and draining what the peer still sends, bounded in time
/// and bytes, lets the response arrive with a FIN.
async fn lingering_close<S: AsyncRead + AsyncWrite + Unpin>(stream: &mut S) {
    if !shutdown_within_linger(stream).await {
        return;
    }
    let mut sink = [0_u8; 4096];
    let mut drained = 0;
    let _ = timeout(LINGER_TIMEOUT, async {
        while drained < LINGER_MAX_BYTES {
            match stream.read(&mut sink).await {
                Ok(0) | Err(_) => break,
                Ok(n) => drained += n,
            }
        }
    })
    .await;
}

/// A response that finished is already flushed, leaving only TLS's close_notify
/// to send, and one that timed out isn't worth waiting for. Each call gets the
/// whole bound. Returns whether the shutdown completed.
async fn shutdown_within_linger<S: AsyncWrite + Unpin>(stream: &mut S) -> bool {
    matches!(timeout(LINGER_TIMEOUT, stream.shutdown()).await, Ok(Ok(())))
}

async fn serve_requests<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    connection_ip: Option<IpAddr>,
    plain_http: bool,
    hooks: ServerHooks,
    server_context: Arc<ServerContext>,
) -> io::Result<ConnectionEnd> {
    let trusted_proxy = connection_ip.is_some_and(is_trusted_forwarding_proxy);
    let mut pending = Vec::new();
    loop {
        let raw_request = match read_request_with_limits(
            stream,
            &mut pending,
            MAX_REQUEST_SIZE,
            KEEP_ALIVE_IDLE_TIMEOUT,
            MAX_REQUEST_TIME,
        )
        .await
        {
            Ok(bytes) => bytes,
            Err(ReadRequestError::IdleTimeout | ReadRequestError::Closed) => {
                return Ok(ConnectionEnd::PeerDone);
            }
            Err(ReadRequestError::Timeout) => {
                write_response(stream, &mut Response::request_timeout()).await?;
                return Ok(ConnectionEnd::ClosedByServer);
            }
            Err(ReadRequestError::TooLarge) => {
                write_response(stream, &mut Response::payload_too_large()).await?;
                return Ok(ConnectionEnd::ClosedByServer);
            }
            Err(ReadRequestError::BadFraming(framing)) => {
                let mut response = match framing {
                    BadFraming::TransferEncoding => Response::not_implemented(),
                    BadFraming::Malformed => Response::bad_request(),
                };
                write_response(stream, &mut response).await?;
                return Ok(ConnectionEnd::ClosedByServer);
            }
            Err(ReadRequestError::Io(error)) => return Err(error),
        };

        let parsed = match parse_request_head(&raw_request) {
            Some(parsed) => parsed,
            None => {
                write_response(stream, &mut Response::bad_request()).await?;
                return Ok(ConnectionEnd::ClosedByServer);
            }
        };

        let api_route = resolve_api_route(parsed.target);
        let ingest_route = if api_route.is_none() {
            resolve_ingest_route(parsed.target)
        } else {
            None
        };

        // Owned copies of the bits we still need after `parsed` (and `raw_request`)
        // are moved into the blocking task; used for logging and CORS injection.
        let method_owned = parsed.method.to_string();
        let target_owned = redact_sensitive_query(parsed.target);
        let client_close = parsed.connection_close;
        let is_ingest = ingest_route.is_some();
        // Dashboard API keeps a credentialed allowlist; public ingest uses `*`.
        let cors_headers = if is_ingest {
            None
        } else {
            cors_headers_for_request(&parsed, &server_context.cors_origins)
        };
        let started = Instant::now();

        // With REVSCOPE_REQUIRE_CLOUDFLARE, plain HTTP is Cloudflare-only except
        // HTTP-01, which Let's Encrypt fetches directly. Unknown challenge tokens
        // still 404 rather than 403. HTTPS is left open: clients reach 443
        // without a Cloudflare hop.
        if plain_http
            && server_context.require_cloudflare
            && !trusted_proxy
            && !crate::acme::is_acme_challenge_path(parsed.target)
        {
            write_response_with_context(
                stream,
                Response::forbidden(),
                &method_owned,
                &target_owned,
                started,
                is_ingest,
            )
            .await?;
            return Ok(ConnectionEnd::ClosedByServer);
        }

        // Every arm but the last answers without `/v2/` or ingest routing.
        let mut log_as_ingest = is_ingest;
        let response = if parsed.method == "GET"
            // The ACME server validates domain control over plain HTTP, before
            // any certificate exists.
            && let Some(response) = crate::acme::challenge_response(&server_context, parsed.target)
        {
            response
        } else if let Some(response) = crate::mcp::discovery::well_known_response(&parsed) {
            // The OAuth discovery documents live at the root because the RFCs
            // that define them put them there. This also answers the MCP
            // preflight, which the dashboard's credentialed allowlist below
            // cannot: an MCP client's origin is not one of ours.
            response
        } else if let Some(response) = crate::robots::response(&parsed) {
            // Crawlers stop here; MCP clients never request this file.
            response
        } else if let Some(response) = crate::web_ui::response(&parsed) {
            // The bundled UI, when compiled in. Served same-origin with the
            // API, so it needs no CORS headers.
            log_as_ingest = false;
            response
        } else if parsed.method == "OPTIONS" {
            if is_ingest {
                ingest_preflight_response()
            } else {
                preflight_response(cors_headers.as_ref())
            }
        } else if parsed.method != "GET" && parsed.method != "POST" && parsed.method != "DELETE" {
            let mut response = Response::method_not_allowed();
            if is_ingest {
                with_ingest_cors_headers(&mut response);
            } else {
                with_cors_headers(&mut response, cors_headers.as_ref());
            }
            response
        } else {
            let headers_trusted = forwarded_headers_trusted(
                &parsed,
                connection_ip,
                trusted_proxy,
                server_context.trust_x_forwarded_for,
            );
            let client_ip = request_client_ip(
                &parsed,
                connection_ip,
                headers_trusted,
                server_context.trust_x_forwarded_for,
            );
            let cf_country = trusted_cf_ipcountry(parsed.cf_ipcountry, headers_trusted);

            // Forward the ingest payload to test hooks before we handle the request.
            if api_route.is_none()
                && ingest_route.is_some_and(|route| route != IngestRoute::Js)
                && let Some(sender) = hooks.payload_sender.as_ref()
            {
                let body = request_body(&raw_request).unwrap_or(&[]);
                let _ = sender.send(extract_payload(parsed.target, body).to_vec());
            }

            // MCP is the one API route that stays on the async runtime: `rmcp`'s
            // handler is async and its answer may be a stream that outlives this
            // call. The blocking work inside its tools is spawned individually.
            // API and MCP share a concurrency semaphore; ingest does not.
            let is_mcp = api_route
                .as_ref()
                .is_some_and(|route| route.route == ApiRoute::Mcp);
            // Copied before `raw_request` moves into the API blocking task so a
            // later 500 can still attach the JSON body to Sentry. GET has no body.
            let sentry_body = if method_owned.eq_ignore_ascii_case("GET") {
                Vec::new()
            } else {
                request_body(&raw_request).unwrap_or(&[]).to_vec()
            };
            let mut response = if is_mcp {
                crate::mcp::transport::handle_mcp_request(&raw_request, server_context.clone())
                    .await
            } else if let Some(route) = api_route {
                let session_key = request_session_key(parsed.cookie, parsed.authorization);
                match server_context.acquire_request_slot(session_key).await {
                    Some(_permit) => {
                        let raw_owned = raw_request;
                        let blocking_context = server_context.clone();
                        let blocking =
                            crate::task::spawn_blocking(blocking_context.clone(), move || {
                                let parsed = parse_request_head(&raw_owned).expect(
                                    "request already parsed successfully in the async stage",
                                );
                                let body = request_body(&raw_owned).unwrap_or(&[]);
                                handle_api_route(
                                    route,
                                    &parsed,
                                    body,
                                    client_ip,
                                    headers_trusted,
                                    blocking_context,
                                )
                            });
                        blocking.await.unwrap_or_else(|join_error| {
                            // Panic integration already reported the panic; avoid a duplicate
                            // Sentry event from error! -> SentryLogger.
                            if join_error.is_panic() {
                                warn!(
                                    "{method_owned} {target_owned} request task panicked: {join_error}"
                                );
                                Response::internal_server_error()
                            } else {
                                internal_error_response(join_error.to_string())
                            }
                        })
                    }
                    None => Response::service_unavailable(),
                }
            } else if let Some(route) = ingest_route {
                let body = request_body(&raw_request).unwrap_or(&[]);
                handle_ingest_request(
                    route,
                    &parsed,
                    body,
                    client_ip,
                    cf_country,
                    server_context.clone(),
                )
                .await
            } else {
                Response::route_not_found()
            };

            if let Some(detail) = response.internal_error.take() {
                // error! is forwarded to Sentry via SentryLogger; rate-limit noisy ingest
                // persist failures so an outage cannot exhaust quota.
                if crate::sentry_report::should_error_log_internal_detail(&detail) {
                    let status = parse_status_line(&response.headers)
                        .map(|(code, _)| code)
                        .unwrap_or(500);
                    crate::sentry_report::with_http_context(
                        &method_owned,
                        &target_owned,
                        status,
                        &sentry_body,
                        || error!("{method_owned} {target_owned} failed: {detail}"),
                    );
                } else {
                    warn!("{method_owned} {target_owned} failed: {detail}");
                }
            }

            if is_ingest {
                with_ingest_cors_headers(&mut response);
            } else if !is_mcp {
                // MCP answers carry their own wildcard CORS headers; adding the
                // credentialed ones too would send two `Allow-Origin` values, which
                // browsers reject outright.
                with_cors_headers(&mut response, cors_headers.as_ref());
            }
            response
        };

        let close_after = client_close || response.closes_connection();
        write_response_with_context(
            stream,
            response,
            &method_owned,
            &target_owned,
            started,
            log_as_ingest,
        )
        .await?;
        if close_after {
            return Ok(ConnectionEnd::ClosedByServer);
        }
    }
}

fn internal_error_response(detail: String) -> Response {
    Response::internal_server_error().with_internal_error(detail)
}

async fn write_response_with_context<S: AsyncWrite + Unpin>(
    stream: &mut S,
    mut response: Response,
    method: &str,
    target: &str,
    started: Instant,
    is_ingest: bool,
) -> io::Result<()> {
    write_response(stream, &mut response).await?;
    log_completed_request(method, target, &response, started.elapsed(), is_ingest);
    Ok(())
}

async fn write_response<S: AsyncWrite + Unpin>(
    stream: &mut S,
    response: &mut Response,
) -> io::Result<()> {
    write_to_peer(stream, &response.headers).await?;
    if let Some(file_body) = response.file_body.as_ref() {
        return write_file_body(stream, file_body).await;
    }
    if let Some(frames) = response.stream_body.as_mut() {
        return write_stream_body(stream, frames).await;
    }
    write_to_peer(stream, &response.body).await
}

/// Forwards frames to the socket until the producing task drops its sender.
/// Each frame is flushed on its own so a client waiting on server-sent events
/// sees them as they are produced rather than when a write buffer fills. Only
/// the writes are timed, since a stream may rightly go quiet between frames.
async fn write_stream_body<S: AsyncWrite + Unpin>(
    stream: &mut S,
    frames: &mut mpsc::Receiver<Vec<u8>>,
) -> io::Result<()> {
    while let Some(frame) = frames.recv().await {
        write_to_peer(stream, &frame).await?;
    }
    Ok(())
}

const RESPONSE_CHUNK_SIZE: usize = 256 * 1024;

/// A peer that takes longer than this to accept one chunk of a response is
/// treated as gone. Without a limit, a client that stops reading holds its
/// connection and the buffered response, or for a download the open file and a
/// snapshot (see [`crate::download_snapshots`]), indefinitely.
const PEER_WRITE_TIMEOUT: Duration = Duration::from_secs(60);

/// Times each chunk rather than the whole of `bytes`, so a slow client can still
/// receive a large response. The flush is inside the timeout because TLS can
/// report a write as finished with ciphertext still buffered.
async fn write_to_peer<S: AsyncWrite + Unpin>(stream: &mut S, bytes: &[u8]) -> io::Result<()> {
    for chunk in bytes.chunks(RESPONSE_CHUNK_SIZE) {
        let send = async {
            stream.write_all(chunk).await?;
            stream.flush().await
        };
        timeout(PEER_WRITE_TIMEOUT, send).await.map_err(|_| {
            io::Error::new(io::ErrorKind::TimedOut, "peer stopped reading the response")
        })??;
    }
    Ok(())
}

/// Copies `file_body` to the socket in chunks so large downloads never need to
/// be held in memory. A short read means the file shrank after the handler
/// measured it, which leaves the connection short of the advertised
/// `Content-Length`; closing it is the only way to signal the truncation.
async fn write_file_body<S: AsyncWrite + Unpin>(
    stream: &mut S,
    file_body: &FileBody,
) -> io::Result<()> {
    let mut file = File::open(&file_body.path).await?;
    if file_body.offset > 0 {
        file.seek(SeekFrom::Start(file_body.offset)).await?;
    }
    let mut remaining = file_body.length;
    let mut buffer = vec![0_u8; RESPONSE_CHUNK_SIZE];
    while remaining > 0 {
        let wanted = remaining.min(RESPONSE_CHUNK_SIZE as u64) as usize;
        let bytes_read = file.read(&mut buffer[..wanted]).await?;
        if bytes_read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!(
                    "{} ended before its advertised length",
                    file_body.path.display()
                ),
            ));
        }
        write_to_peer(stream, &buffer[..bytes_read]).await?;
        remaining -= bytes_read as u64;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn classifies_client_disconnect_kinds_as_benign() {
        assert!(is_benign_connection_error(&io::Error::new(
            io::ErrorKind::ConnectionReset,
            "connection reset by peer"
        )));
        assert!(is_benign_connection_error(&io::Error::new(
            io::ErrorKind::ConnectionAborted,
            "connection aborted"
        )));
        assert!(is_benign_connection_error(&io::Error::new(
            io::ErrorKind::BrokenPipe,
            "broken pipe"
        )));
        assert!(is_benign_connection_error(&io::Error::new(
            io::ErrorKind::TimedOut,
            "peer stopped reading the response"
        )));
        assert!(is_benign_connection_error(&io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "peer closed connection without sending TLS close_notify"
        )));
        assert!(!is_benign_connection_error(&io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "unexpected eof"
        )));
        assert!(!is_benign_connection_error(&io::Error::other("disk full")));
        assert!(!is_benign_connection_error(&io::Error::from_raw_os_error(
            libc::ETIMEDOUT
        )));
    }

    struct TestFile {
        path: PathBuf,
    }

    impl TestFile {
        fn new(name: &str, contents: &[u8]) -> Self {
            let path = std::env::temp_dir()
                .join(format!("revscope-file-body-{}-{name}", std::process::id()));
            std::fs::write(&path, contents).expect("write test file");
            Self { path }
        }
    }

    impl Drop for TestFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    /// Writes `file_body` through a real socket and returns what the peer read.
    async fn stream_to_peer(file_body: FileBody) -> Vec<u8> {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let writer = tokio::spawn(async move {
            let (mut server_side, _peer) = listener.accept().await.expect("accept");
            write_file_body(&mut server_side, &file_body)
                .await
                .expect("write file body");
        });

        let mut client = TcpStream::connect(addr).await.expect("connect");
        let mut received = Vec::new();
        client
            .read_to_end(&mut received)
            .await
            .expect("read response body");
        writer.await.expect("writer task");
        received
    }

    /// Certificate authority plus a `localhost` leaf signed by it, so the test
    /// client can verify the server the same way a browser would.
    fn test_certificate_chain() -> (String, String, rustls::pki_types::CertificateDer<'static>) {
        let mut authority_params =
            rcgen::CertificateParams::new(Vec::new()).expect("authority parameters");
        authority_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let authority_key = rcgen::KeyPair::generate().expect("authority key");
        let authority = authority_params
            .self_signed(&authority_key)
            .expect("self-sign authority");

        let leaf_params =
            rcgen::CertificateParams::new(vec!["localhost".to_string()]).expect("leaf parameters");
        let leaf_key = rcgen::KeyPair::generate().expect("leaf key");
        let leaf = leaf_params
            .signed_by(
                &leaf_key,
                &rcgen::Issuer::from_params(&authority_params, &authority_key),
            )
            .expect("sign leaf");

        (
            leaf.pem(),
            leaf_key.serialize_pem(),
            authority.der().clone(),
        )
    }

    /// Starts the HTTPS listener with a certificate for `localhost` and returns
    /// its address, the authority clients must trust, and its shutdown handle.
    async fn start_tls_listener(
        namespace: &str,
    ) -> (
        SocketAddr,
        rustls::pki_types::CertificateDer<'static>,
        Arc<Notify>,
        tokio::task::JoinHandle<io::Result<()>>,
    ) {
        let (leaf_pem, leaf_key_pem, authority) = test_certificate_chain();
        let server_context = Arc::new(crate::server_context::ServerContext::new_in_memory(
            namespace,
        ));
        server_context
            .acme
            .certificates()
            .install_pem(&["localhost".to_string()], &leaf_pem, &leaf_key_pem)
            .expect("install certificate");
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let shutdown = Arc::new(Notify::new());
        let server = tokio::spawn(run_tls_listener(
            listener,
            ServerHooks::default(),
            shutdown.clone(),
            server_context,
        ));
        (addr, authority, shutdown, server)
    }

    async fn tls_connect(
        addr: SocketAddr,
        authority: rustls::pki_types::CertificateDer<'static>,
    ) -> tokio_rustls::client::TlsStream<TcpStream> {
        let mut roots = rustls::RootCertStore::empty();
        roots.add(authority).expect("trust the test authority");
        let config = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("default protocol versions")
        .with_root_certificates(roots)
        .with_no_client_auth();

        let stream = TcpStream::connect(addr).await.expect("connect");
        let server_name =
            rustls::pki_types::ServerName::try_from("localhost").expect("valid server name");
        tokio_rustls::TlsConnector::from(Arc::new(config))
            .connect(server_name, stream)
            .await
            .expect("tls handshake")
    }

    /// With one runtime worker, the certificate can only arrive if the waiting
    /// handshake leaves that worker free.
    #[tokio::test(flavor = "current_thread")]
    async fn a_handshake_waiting_for_a_certificate_leaves_the_worker_free() {
        let (leaf_pem, leaf_key_pem, authority) = test_certificate_chain();
        let server_context = Arc::new(crate::server_context::ServerContext::new_in_memory(
            "tls-waits-without-blocking",
        ));
        let order = server_context.acme.start_test_order("localhost");
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let shutdown = Arc::new(Notify::new());
        let server = tokio::spawn(run_tls_listener(
            listener,
            ServerHooks::default(),
            shutdown.clone(),
            server_context.clone(),
        ));
        let client = tokio::spawn(tls_connect(addr, authority));

        timeout(Duration::from_secs(5), async {
            while !order.has_waiters() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("the handshake should wait for the order");
        server_context
            .acme
            .certificates()
            .install_pem(&["localhost".to_string()], &leaf_pem, &leaf_key_pem)
            .expect("install certificate");
        order.finish();

        timeout(Duration::from_secs(5), client)
            .await
            .expect("the handshake should finish once the certificate exists")
            .expect("client task");
        shutdown.notify_waiters();
        server.await.expect("server task").expect("server result");
    }

    async fn https_request(
        addr: SocketAddr,
        authority: rustls::pki_types::CertificateDer<'static>,
        request: &str,
    ) -> String {
        let mut stream = tls_connect(addr, authority).await;
        stream
            .write_all(request.as_bytes())
            .await
            .expect("write request");
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .await
            .expect("read response");
        String::from_utf8_lossy(&response).into_owned()
    }

    /// Sends a chunked upload far larger than the server reads before it
    /// answers 501 and closes, and asserts the whole upload is accepted and the
    /// answer is read to a clean end. Over TLS close_notify ends the stream
    /// cleanly even when a reset follows, so the stream is returned for checks
    /// below its application layer.
    async fn assert_501_survives_unread_upload<S>(stream: S) -> S
    where
        S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        let (mut reader, mut writer) = tokio::io::split(stream);
        let uploader = tokio::spawn(async move {
            let mut request = b"POST /v2/public/chat/status HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
            request.extend(std::iter::repeat_n(b'x', 256 * 1024));
            // A write hit by the reset consumes the socket's pending error, so
            // it must be checked here or a later pending-error check misses it.
            // Flushing makes TLS send the ciphertext it buffered on accepting
            // the write.
            let written = async {
                writer.write_all(&request).await?;
                writer.flush().await
            }
            .await;
            (writer, written)
        });
        let mut response = Vec::new();
        let read = timeout(Duration::from_secs(5), reader.read_to_end(&mut response))
            .await
            .expect("the server must close the connection");
        let response = String::from_utf8_lossy(&response);
        assert!(read.is_ok(), "{read:?} after {response:?}");
        assert!(
            response.starts_with("HTTP/1.1 501 Not Implemented"),
            "{response}"
        );
        let (writer, written) = timeout(Duration::from_secs(5), uploader)
            .await
            .expect("the server must drain the upload, not stall it")
            .expect("join");
        assert!(written.is_ok(), "upload during the drain: {written:?}");
        reader.unsplit(writer)
    }

    async fn exchange_on_context(context: Arc<ServerContext>, request: &str) -> String {
        let (mut client, mut server) = tokio::io::duplex(16 * 1024);
        let server_task = tokio::spawn(async move {
            handle_connection(
                &mut server,
                Some("173.245.48.1".parse().unwrap()),
                true,
                ServerHooks::default(),
                context,
            )
            .await
        });
        client.write_all(request.as_bytes()).await.expect("write");
        let mut response = Vec::new();
        client.read_to_end(&mut response).await.expect("read");
        drop(client);
        server_task.await.expect("join").expect("handle");
        String::from_utf8_lossy(&response).into_owned()
    }

    async fn exchange_from_peer(peer: IpAddr, plain_http: bool, request: &str) -> String {
        exchange_from_peer_with(peer, plain_http, false, request).await
    }

    async fn exchange_from_peer_with(
        peer: IpAddr,
        plain_http: bool,
        require_cloudflare: bool,
        request: &str,
    ) -> String {
        let (mut client, mut server) = tokio::io::duplex(16 * 1024);
        let mut context = crate::server_context::ServerContext::new_in_memory(format!(
            "origin-peer-{peer}-{plain_http}-{require_cloudflare}"
        ));
        context.require_cloudflare = require_cloudflare;
        let context = Arc::new(context);
        let server_task = tokio::spawn(async move {
            handle_connection(
                &mut server,
                Some(peer),
                plain_http,
                ServerHooks::default(),
                context,
            )
            .await
        });
        client.write_all(request.as_bytes()).await.expect("write");
        let mut response = Vec::new();
        client.read_to_end(&mut response).await.expect("read");
        drop(client);
        server_task.await.expect("join").expect("handle");
        String::from_utf8_lossy(&response).into_owned()
    }

    #[test]
    fn accept_errors_retry_only_when_the_listener_can_recover() {
        let os = io::Error::from_raw_os_error;
        assert_eq!(
            classify_accept_error(&os(libc::EMFILE)),
            AcceptError::OutOfResources
        );
        assert_eq!(
            classify_accept_error(&os(libc::ENFILE)),
            AcceptError::OutOfResources
        );
        assert_eq!(
            classify_accept_error(&os(libc::ENOBUFS)),
            AcceptError::OutOfResources
        );
        assert_eq!(
            classify_accept_error(&os(libc::ECONNABORTED)),
            AcceptError::PerConnection
        );
        assert_eq!(
            classify_accept_error(&os(libc::EPROTO)),
            AcceptError::PerConnection
        );
        assert_eq!(classify_accept_error(&os(libc::EBADF)), AcceptError::Fatal);
        assert_eq!(classify_accept_error(&os(libc::EINVAL)), AcceptError::Fatal);
        assert_eq!(
            classify_accept_error(&os(libc::ENOTSOCK)),
            AcceptError::Fatal
        );
    }

    #[tokio::test]
    async fn connections_over_the_cap_are_closed_until_a_slot_frees() {
        let context = Arc::new(
            crate::server_context::ServerContext::new_in_memory_with_connection_limit(
                "connection-cap",
                1,
            ),
        );
        let listener = bind_listener("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let shutdown = Arc::new(Notify::new());
        let server = tokio::spawn(run_listener(
            listener,
            ServerHooks::default(),
            shutdown.clone(),
            context,
        ));
        let request =
            b"GET /v2/public/chat/status HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";

        // An idle connection holds the only slot.
        let idle = TcpStream::connect(addr).await.expect("connect idle");
        let mut rejected = TcpStream::connect(addr).await.expect("connect rejected");
        let _ = rejected.write_all(request).await;
        let mut response = Vec::new();
        let _ = timeout(Duration::from_secs(5), rejected.read_to_end(&mut response))
            .await
            .expect("an over-cap connection must be closed, not left waiting");
        assert!(
            response.is_empty(),
            "{}",
            String::from_utf8_lossy(&response)
        );

        // Closing the idle connection frees its slot once its task notices.
        drop(idle);
        let deadline = Instant::now() + Duration::from_secs(5);
        let served = loop {
            let mut stream = TcpStream::connect(addr).await.expect("connect");
            let _ = stream.write_all(request).await;
            let mut response = Vec::new();
            let _ = stream.read_to_end(&mut response).await;
            if !response.is_empty() || Instant::now() > deadline {
                break String::from_utf8_lossy(&response).into_owned();
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        assert!(served.starts_with("HTTP/1.1 "), "{served}");

        shutdown.notify_waiters();
        server.await.expect("join").expect("server");
    }

    #[tokio::test]
    async fn error_responses_survive_a_close_with_request_bytes_unread() {
        let context = Arc::new(crate::server_context::ServerContext::new_in_memory(
            "lingering-close",
        ));
        let listener = bind_listener("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let shutdown = Arc::new(Notify::new());
        let server = tokio::spawn(run_listener(
            listener,
            ServerHooks::default(),
            shutdown.clone(),
            context,
        ));

        for _ in 0..5 {
            drop(
                assert_501_survives_unread_upload(TcpStream::connect(addr).await.expect("connect"))
                    .await,
            );
        }

        shutdown.notify_waiters();
        server.await.expect("join").expect("server");
    }

    #[tokio::test]
    async fn tls_error_responses_survive_a_close_with_request_bytes_unread() {
        let (addr, authority, shutdown, server) = start_tls_listener("tls-lingering-close").await;

        for _ in 0..5 {
            let tls =
                assert_501_survives_unread_upload(tls_connect(addr, authority.clone()).await).await;
            // close_notify ends the TLS stream cleanly even when a reset follows,
            // so a reset shows only on the TCP socket underneath. Linux keeps
            // reading 0 after a FIN even once an RST arrives and records the
            // reset as the socket's pending error instead, so check that after
            // giving an RST time to land. While draining, the server sends none.
            let (mut tcp, _) = tls.into_inner();
            let mut rest = [0_u8; 1];
            let after_close_notify = timeout(Duration::from_secs(5), tcp.read(&mut rest))
                .await
                .expect("the server must shut down its side of the TCP connection");
            assert!(
                matches!(after_close_notify, Ok(0)),
                "{after_close_notify:?}"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
            let pending_error = tcp.take_error().expect("socket error");
            assert!(pending_error.is_none(), "{pending_error:?}");
        }

        shutdown.notify_waiters();
        server.await.expect("join listener").expect("serve");
    }

    #[tokio::test]
    async fn require_cloudflare_rejects_direct_origin_requests_except_acme_challenges() {
        let internet: IpAddr = "198.51.100.1".parse().unwrap();
        let cloudflare: IpAddr = "173.245.48.1".parse().unwrap();
        let status_request =
            "GET /v2/public/chat/status HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
        let acme_request = "GET /.well-known/acme-challenge/token-1 HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";

        let rejected = exchange_from_peer_with(internet, true, true, status_request).await;
        assert!(rejected.starts_with("HTTP/1.1 403 Forbidden"), "{rejected}");

        let challenge = exchange_from_peer_with(internet, true, true, acme_request).await;
        assert!(
            challenge.starts_with("HTTP/1.1 404 Not Found"),
            "{challenge}"
        );

        let allowed = exchange_from_peer_with(cloudflare, true, true, status_request).await;
        assert!(allowed.starts_with("HTTP/1.1 200 OK"), "{allowed}");

        let tls_direct = exchange_from_peer_with(internet, false, true, status_request).await;
        assert!(tls_direct.starts_with("HTTP/1.1 200 OK"), "{tls_direct}");
    }

    #[tokio::test]
    async fn plain_http_serves_direct_peers_by_default() {
        let internet: IpAddr = "198.51.100.1".parse().unwrap();
        let response = exchange_from_peer(
            internet,
            true,
            "GET /v2/public/chat/status HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    }

    #[tokio::test]
    async fn pipelined_requests_in_one_write_are_each_answered() {
        let request = "GET /robots.txt HTTP/1.1\r\nHost: localhost\r\n\r\n\
            GET /robots.txt HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
        let response = exchange_from_peer("198.51.100.1".parse().unwrap(), false, request).await;
        assert_eq!(response.matches("HTTP/1.1 200 OK").count(), 2, "{response}");
    }

    #[tokio::test]
    async fn framing_errors_answer_once_and_close_without_reading_the_body_as_a_request() {
        let smuggled = "GET /robots.txt HTTP/1.1\r\nHost: localhost\r\n\r\n";
        for (head, status) in [
            (
                "Transfer-Encoding: chunked\r\n",
                "HTTP/1.1 501 Not Implemented",
            ),
            (
                "Content-Length: 5\r\nTransfer-Encoding: chunked\r\n",
                "HTTP/1.1 501 Not Implemented",
            ),
            ("Content-Length: x\r\n", "HTTP/1.1 400 Bad Request"),
        ] {
            let request = format!(
                "POST /v2/public/chat/status HTTP/1.1\r\nHost: localhost\r\n{head}\r\n{:x}\r\n{smuggled}\r\n0\r\n\r\n",
                smuggled.len()
            );
            let response =
                exchange_from_peer("198.51.100.1".parse().unwrap(), false, &request).await;
            assert!(response.starts_with(status), "{head}: {response}");
            assert!(response.contains("Connection: close"), "{head}: {response}");
            assert_eq!(
                response.matches("HTTP/1.1 ").count(),
                1,
                "{head}: {response}"
            );
        }
    }

    #[tokio::test]
    async fn robots_txt_disallows_crawlers_on_every_public_host() {
        for host in ["api.revscope.co", "in.revscope.co", "eu1.revscope.co"] {
            let response = exchange_from_peer(
                "198.51.100.1".parse().unwrap(),
                false,
                &format!("GET /robots.txt HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"),
            )
            .await;
            assert!(
                response.starts_with("HTTP/1.1 200 OK"),
                "{host}: {response}"
            );
            assert!(
                response.contains("Content-Type: text/plain; charset=utf-8"),
                "{host}"
            );
            assert!(
                response.ends_with("User-agent: *\nDisallow: /\n"),
                "{host}: {response}"
            );
        }
    }

    #[tokio::test]
    async fn api_returns_503_when_request_slots_are_exhausted() {
        let context = Arc::new(
            crate::server_context::ServerContext::new_in_memory_with_request_slots(
                "api-slot-timeout",
                1,
            ),
        );
        let _held = context
            .acquire_request_slot(None)
            .await
            .expect("the only slot must be available");
        let response = exchange_on_context(
            context,
            "GET /v2/public/chat/status HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert!(
            response.starts_with("HTTP/1.1 503 Service Unavailable"),
            "{response}"
        );
        assert!(response.contains("Retry-After: 5\r\n"), "{response}");
    }

    #[tokio::test]
    async fn one_session_cannot_take_every_request_slot() {
        let context = Arc::new(
            crate::server_context::ServerContext::new_in_memory_with_request_slots(
                "api-per-session-slots",
                64,
            ),
        );
        let session = crate::uuid_token::UuidToken::generate().expect("generate");
        let session_id = session.to_string();
        let session_key = Some(session.key());
        let mut held = Vec::new();
        for _ in 0..crate::request_slots::PER_SESSION_SLOTS {
            held.push(
                context
                    .acquire_request_slot(session_key)
                    .await
                    .expect("a session slot must be free"),
            );
        }

        let same_session = exchange_on_context(
            context.clone(),
            &format!(
                "GET /v2/public/chat/status HTTP/1.1\r\nHost: localhost\r\nCookie: rr_session={session_id}\r\nConnection: close\r\n\r\n"
            ),
        )
        .await;
        assert!(
            same_session.starts_with("HTTP/1.1 503 Service Unavailable"),
            "{same_session}"
        );

        let anonymous = exchange_on_context(
            context,
            "GET /v2/public/chat/status HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert!(
            !anonymous.starts_with("HTTP/1.1 503"),
            "other clients must still be served: {anonymous}"
        );
    }

    #[tokio::test]
    async fn ingest_is_not_gated_by_the_request_slot_semaphore() {
        let context = Arc::new(
            crate::server_context::ServerContext::new_in_memory_with_request_slots(
                "ingest-ungated",
                1,
            ),
        );
        let _held = context
            .acquire_request_slot(None)
            .await
            .expect("the only slot must be available");
        let response = exchange_on_context(
            context,
            "GET /demo.js HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    }

    #[tokio::test]
    async fn serves_https_with_the_certificate_the_acme_task_installed() {
        let (addr, authority, shutdown, server) = start_tls_listener("tls-listener").await;

        let response = https_request(
            addr,
            authority,
            "GET /v2/public/chat/status HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await;

        shutdown.notify_waiters();
        server.await.expect("join listener").expect("serve");
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    }

    #[tokio::test]
    async fn streams_a_body_larger_than_one_chunk() {
        let contents: Vec<u8> = (0..RESPONSE_CHUNK_SIZE * 2 + 17)
            .map(|index| index as u8)
            .collect();
        let file = TestFile::new("whole", &contents);

        let received = stream_to_peer(FileBody {
            path: file.path.clone(),
            offset: 0,
            length: contents.len() as u64,
            _keep_alive: None,
        })
        .await;

        assert_eq!(received, contents);
    }

    #[tokio::test]
    async fn streams_only_the_requested_range() {
        let contents: Vec<u8> = (0..1_000_u32).map(|index| index as u8).collect();
        let file = TestFile::new("range", &contents);

        let received = stream_to_peer(FileBody {
            path: file.path.clone(),
            offset: 100,
            length: 250,
            _keep_alive: None,
        })
        .await;

        assert_eq!(received, contents[100..350]);
    }

    #[tokio::test(start_paused = true)]
    async fn gives_up_on_a_peer_that_never_reads_the_headers() {
        let file = TestFile::new("unread-headers", b"body");
        let (mut writer, _unread_peer) = tokio::io::duplex(16);
        let mut response = Response::file(
            "200 OK",
            FileBody {
                path: file.path.clone(),
                offset: 0,
                length: 4,
                _keep_alive: None,
            },
            &[("Content-Type", "application/octet-stream".to_string())],
        );
        let error = write_response(&mut writer, &mut response)
            .await
            .expect_err("a stalled peer times out");
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    }

    #[tokio::test(start_paused = true)]
    async fn gives_up_on_a_peer_that_stops_reading() {
        let contents = vec![7_u8; 1_000];
        let file = TestFile::new("stalled", &contents);
        let (mut writer, _unread_peer) = tokio::io::duplex(16);
        let error = write_file_body(
            &mut writer,
            &FileBody {
                path: file.path.clone(),
                offset: 0,
                length: contents.len() as u64,
                _keep_alive: None,
            },
        )
        .await
        .expect_err("a stalled peer times out");
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    }

    /// Fails rather than hangs if nothing bounds the wait, because a paused
    /// clock with no timer pending would otherwise never advance.
    async fn assert_gives_up_on_the_peer(writing: impl Future<Output = io::Result<()>>) {
        let error = timeout(PEER_WRITE_TIMEOUT * 4, writing)
            .await
            .expect("the write must be bounded")
            .expect_err("a stalled peer times out");
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    }

    #[tokio::test(start_paused = true)]
    async fn gives_up_on_a_peer_that_stops_reading_an_in_memory_body() {
        let (mut writer, _unread_peer) = tokio::io::duplex(1024);
        let mut response = Response::new(b"HTTP/1.1 200 OK\r\n\r\n".to_vec(), vec![7_u8; 4096]);
        assert_gives_up_on_the_peer(write_response(&mut writer, &mut response)).await;
    }

    #[tokio::test(start_paused = true)]
    async fn gives_up_on_a_peer_that_never_reads_a_timeout_response() {
        let (mut client, mut server) = tokio::io::duplex(16);
        client.write_all(b"GET").await.expect("write");
        let context = Arc::new(crate::server_context::ServerContext::new_in_memory(
            "stalled-408",
        ));
        let serving = async {
            handle_connection(&mut server, None, false, ServerHooks::default(), context).await
        };
        assert_gives_up_on_the_peer(serving).await;
    }

    #[tokio::test(start_paused = true)]
    async fn gives_up_on_a_peer_that_never_reads_a_too_large_response() {
        let (mut client, mut server) = tokio::io::duplex(64);
        client
            .write_all(b"POST /v2/x HTTP/1.1\r\nContent-Length: 999999999\r\n\r\n")
            .await
            .expect("write");
        let context = Arc::new(crate::server_context::ServerContext::new_in_memory(
            "stalled-413",
        ));
        let serving = async {
            handle_connection(&mut server, None, false, ServerHooks::default(), context).await
        };
        assert_gives_up_on_the_peer(serving).await;
    }

    #[tokio::test(start_paused = true)]
    async fn gives_up_on_a_stream_the_peer_stops_reading_but_not_on_a_quiet_one() {
        let (mut writer, mut peer) = tokio::io::duplex(8);
        let (frames, mut receiver) = mpsc::channel(1);
        let producer = tokio::spawn(async move {
            frames.send(b"first".to_vec()).await.expect("send");
            tokio::time::sleep(PEER_WRITE_TIMEOUT * 2).await;
            frames.send(vec![7_u8; 64]).await.expect("send");
            // Keeps the stream open while the write above stalls.
            tokio::time::sleep(PEER_WRITE_TIMEOUT * 10).await;
        });

        let mut first = [0_u8; 5];
        let writing = write_stream_body(&mut writer, &mut receiver);
        tokio::pin!(writing);
        tokio::select! {
            _ = &mut writing => panic!("a quiet stream must stay open"),
            read = peer.read_exact(&mut first) => { read.expect("read"); }
        }
        assert_eq!(&first, b"first");
        let quiet_since = tokio::time::Instant::now();
        assert_gives_up_on_the_peer(writing).await;
        assert!(quiet_since.elapsed() >= PEER_WRITE_TIMEOUT * 2);
        producer.abort();
    }

    /// Reads end of stream and writes succeed, except for what is set to stall.
    #[derive(Default)]
    struct StallingStream {
        stall_flush: bool,
        stall_shutdown: bool,
    }

    impl StallingStream {
        fn poll_unless(stalled: bool) -> std::task::Poll<io::Result<()>> {
            if stalled {
                std::task::Poll::Pending
            } else {
                std::task::Poll::Ready(Ok(()))
            }
        }
    }

    impl AsyncRead for StallingStream {
        fn poll_read(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
            _: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    impl AsyncWrite for StallingStream {
        fn poll_write(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
            bytes: &[u8],
        ) -> std::task::Poll<io::Result<usize>> {
            std::task::Poll::Ready(Ok(bytes.len()))
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<io::Result<()>> {
            Self::poll_unless(self.stall_flush)
        }

        fn poll_shutdown(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<io::Result<()>> {
            Self::poll_unless(self.stall_shutdown)
        }
    }

    fn stalled_shutdown() -> StallingStream {
        StallingStream {
            stall_shutdown: true,
            ..Default::default()
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_shutdown_the_peer_never_completes_is_abandoned() {
        timeout(LINGER_TIMEOUT * 2, lingering_close(&mut stalled_shutdown()))
            .await
            .expect("the shutdown must be bounded");
    }

    #[tokio::test(start_paused = true)]
    async fn a_close_notify_the_peer_never_accepts_is_abandoned() {
        let context = Arc::new(crate::server_context::ServerContext::new_in_memory(
            "stalled-close-notify",
        ));
        let mut stream = stalled_shutdown();
        let serving = serve_then_close_notify(
            &mut stream,
            "198.51.100.1".parse().unwrap(),
            ServerHooks::default(),
            context,
        );
        timeout(LINGER_TIMEOUT * 2, serving)
            .await
            .expect("the close must be bounded")
            .expect("the peer hung up cleanly");
    }

    #[tokio::test(start_paused = true)]
    async fn gives_up_on_a_flush_the_peer_never_accepts() {
        let mut stream = StallingStream {
            stall_flush: true,
            ..Default::default()
        };
        assert_gives_up_on_the_peer(write_to_peer(&mut stream, b"response")).await;
    }

    #[tokio::test(start_paused = true)]
    async fn a_client_slower_than_the_timeout_overall_still_gets_the_whole_body() {
        let chunks = 3;
        let body = vec![7_u8; RESPONSE_CHUNK_SIZE * chunks];
        let (mut writer, mut peer) = tokio::io::duplex(1024);
        let reader = tokio::spawn(async move {
            let mut chunk = vec![0_u8; RESPONSE_CHUNK_SIZE];
            for _ in 0..chunks {
                tokio::time::sleep(PEER_WRITE_TIMEOUT * 2 / 3).await;
                peer.read_exact(&mut chunk).await.expect("read");
            }
        });
        let started = tokio::time::Instant::now();

        write_to_peer(&mut writer, &body)
            .await
            .expect("each chunk arrives within the timeout");

        assert!(started.elapsed() > PEER_WRITE_TIMEOUT);
        reader.await.expect("reader");
    }
}
