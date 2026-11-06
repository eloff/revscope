use std::io;
use std::sync::Arc;
use std::time::Duration;

use revscope::ServerHooks;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{Notify, mpsc};
use tokio::time::timeout;

#[tokio::test]
async fn routes_ingest_project_path_and_extracts_query_payload() -> io::Result<()> {
    let (addr, shutdown, mut payloads) = spawn_test_server().await?;
    let response = send_request(
        addr,
        "GET /demo?type=pageview&id=abc HTTP/1.1\r\nHost: ingest.example.test\r\nConnection: close\r\n\r\n",
    )
    .await?;

    assert!(
        response.starts_with(b"HTTP/1.1 200 OK"),
        "unexpected response: {}",
        String::from_utf8_lossy(&response)
    );
    assert_eq!(next_payload(&mut payloads).await?, b"type=pageview&id=abc");

    shutdown.notify_one();
    Ok(())
}

#[tokio::test]
async fn routes_ingest_path_gif_and_extracts_query_payload() -> io::Result<()> {
    let (addr, shutdown, mut payloads) = spawn_test_server().await?;
    let request = concat!(
        "POST /demo.gif?type=event&id=abc HTTP/1.1\r\n",
        "Host: app.example.test\r\n",
        "Content-Length: 0\r\n",
        "Connection: close\r\n\r\n",
    );
    let response = send_request(addr, request).await?;

    assert!(
        response.starts_with(b"HTTP/1.1 200 OK\r\nContent-Type: image/gif\r\n"),
        "unexpected response: {}",
        String::from_utf8_lossy(&response)
    );
    assert!(response.ends_with(&[
        71, 73, 70, 56, 57, 97, 1, 0, 1, 0, 128, 0, 0, 0, 0, 0, 255, 255, 255, 33, 249, 4, 1, 0, 0,
        0, 0, 44, 0, 0, 0, 0, 1, 0, 1, 0, 0, 2, 1, 68, 0, 59,
    ]));
    assert_eq!(next_payload(&mut payloads).await?, b"type=event&id=abc");

    shutdown.notify_one();
    Ok(())
}

#[tokio::test]
async fn serves_tracker_js_for_js_paths() -> io::Result<()> {
    let (addr, shutdown, _payloads) = spawn_test_server().await?;
    let response = send_request(
        addr,
        "GET /demo.js HTTP/1.1\r\nHost: ingest.example.test\r\nConnection: close\r\n\r\n",
    )
    .await?;

    let headers = String::from_utf8_lossy(&response);
    assert!(
        headers.starts_with(
            "HTTP/1.1 200 OK\r\nContent-Type: application/javascript; charset=utf-8\r\nCache-Control: public, max-age=14400\r\nVary: Accept-Encoding\r\n"
        ),
        "unexpected response: {headers}"
    );
    assert!(
        !headers.contains("Content-Encoding:"),
        "identity response must not set Content-Encoding"
    );
    assert!(
        response
            .windows(b"!function(".len())
            .any(|window| window == b"!function("),
        "tracker-min.js body missing from response"
    );

    shutdown.notify_one();
    Ok(())
}

#[tokio::test]
async fn serves_gzip_tracker_when_accepted() -> io::Result<()> {
    let (addr, shutdown, _payloads) = spawn_test_server().await?;
    let response = send_request(
        addr,
        "GET /script.js HTTP/1.1\r\nHost: ingest.example.test\r\nAccept-Encoding: gzip\r\nConnection: close\r\n\r\n",
    )
    .await?;

    let headers = String::from_utf8_lossy(&response);
    assert!(
        headers.contains("Content-Encoding: gzip\r\n"),
        "unexpected response: {headers}"
    );
    assert!(
        headers.contains("Vary: Accept-Encoding\r\n"),
        "missing Vary header: {headers}"
    );
    let body = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|idx| &response[idx + 4..])
        .unwrap_or(&[]);
    assert!(
        body.starts_with(&[0x1f, 0x8b]),
        "gzip magic missing from response body"
    );

    shutdown.notify_one();
    Ok(())
}

#[tokio::test]
async fn serves_brotli_tracker_when_accepted() -> io::Result<()> {
    let (addr, shutdown, _payloads) = spawn_test_server().await?;
    let response = send_request(
        addr,
        "GET /script.js HTTP/1.1\r\nHost: ingest.example.test\r\nAccept-Encoding: gzip, deflate, br\r\nConnection: close\r\n\r\n",
    )
    .await?;

    let headers = String::from_utf8_lossy(&response);
    assert!(
        headers.contains("Content-Encoding: br\r\n"),
        "unexpected response: {headers}"
    );
    assert!(
        !headers.contains("Content-Encoding: gzip\r\n"),
        "should prefer brotli over gzip: {headers}"
    );

    shutdown.notify_one();
    Ok(())
}

#[tokio::test]
async fn reuses_connection_for_keep_alive_requests() -> io::Result<()> {
    let (addr, shutdown, _payloads) = spawn_test_server().await?;
    let mut stream = TcpStream::connect(addr).await?;
    let request = "GET /demo.js HTTP/1.1\r\nHost: ingest.example.test\r\n\r\n";
    stream.write_all(request.as_bytes()).await?;
    let first = read_one_http_response(&mut stream).await?;
    stream.write_all(request.as_bytes()).await?;
    let second = read_one_http_response(&mut stream).await?;

    for (label, response) in [("first", &first), ("second", &second)] {
        let headers = String::from_utf8_lossy(response);
        assert!(
            headers.starts_with("HTTP/1.1 200 OK\r\n"),
            "{label} unexpected status: {headers}"
        );
        assert!(
            !headers.contains("Connection: close"),
            "{label} should stay keep-alive: {headers}"
        );
        assert!(
            response
                .windows(b"!function(".len())
                .any(|window| window == b"!function("),
            "{label} tracker body missing"
        );
    }

    shutdown.notify_one();
    Ok(())
}

#[tokio::test]
async fn rejects_non_ingest_paths() -> io::Result<()> {
    let (addr, shutdown, _payloads) = spawn_test_server().await?;
    let response = send_request(
        addr,
        "GET /nope! HTTP/1.1\r\nHost: ingest.example.test\r\nConnection: close\r\n\r\n",
    )
    .await?;

    let response_text = String::from_utf8_lossy(&response);
    assert!(
        response_text.starts_with("HTTP/1.1 200 OK\r\n"),
        "unexpected status: {response_text}"
    );
    assert!(
        response_text.contains("Content-Type: text/html\r\n"),
        "missing content type: {response_text}"
    );
    assert!(
        response_text.contains("Content-Encoding: gzip\r\n"),
        "missing content encoding: {response_text}"
    );
    assert!(
        response_text.contains("Cache-Control: no-store, no-cache, must-revalidate\r\n"),
        "missing cache-control: {response_text}"
    );
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("header terminator");
    let body = &response[header_end + 4..];
    assert_eq!(
        &body[..2],
        &[0x1f, 0x8b],
        "gzip magic missing from response body"
    );
    shutdown.notify_one();
    Ok(())
}

async fn spawn_test_server() -> io::Result<(
    std::net::SocketAddr,
    Arc<Notify>,
    mpsc::UnboundedReceiver<Vec<u8>>,
)> {
    let listener = revscope::bind_listener("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let shutdown = Arc::new(Notify::new());
    let (tx, rx) = mpsc::unbounded_channel();
    let hooks = ServerHooks::with_payload_sender(tx);
    let shutdown_for_task = shutdown.clone();
    let server_context = revscope::db::default_server_context().clone();
    tokio::spawn(async move {
        let _ = revscope::run_listener(listener, hooks, shutdown_for_task, server_context).await;
    });
    Ok((addr, shutdown, rx))
}

async fn send_request(addr: std::net::SocketAddr, request: &str) -> io::Result<Vec<u8>> {
    let mut stream = TcpStream::connect(addr).await?;
    stream.write_all(request.as_bytes()).await?;
    stream.shutdown().await?;

    let mut response = Vec::new();
    stream.read_to_end(&mut response).await?;
    Ok(response)
}

/// Reads a single Content-Length-delimited HTTP response without closing the
/// socket, so a follow-up request can reuse the connection.
async fn read_one_http_response(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut raw = Vec::new();
    let mut chunk = [0_u8; 1024];
    let header_end = loop {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "connection closed before response headers",
            ));
        }
        raw.extend_from_slice(&chunk[..n]);
        if let Some(end) = raw.windows(4).position(|window| window == b"\r\n\r\n") {
            break end;
        }
    };

    let header_text = String::from_utf8_lossy(&raw[..header_end]);
    let content_length = header_text
        .split("\r\n")
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing Content-Length"))?;

    let body_start = header_end + 4;
    while raw.len() < body_start + content_length {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "connection closed before response body finished",
            ));
        }
        raw.extend_from_slice(&chunk[..n]);
    }
    raw.truncate(body_start + content_length);
    Ok(raw)
}

async fn next_payload(payloads: &mut mpsc::UnboundedReceiver<Vec<u8>>) -> io::Result<Vec<u8>> {
    timeout(Duration::from_secs(2), payloads.recv())
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "payload not received in time"))?
        .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "payload channel closed"))
}
