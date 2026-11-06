//! Running out of file descriptors must not stop the server.
//!
//! This lowers the open file limit and fills the descriptor table, which would
//! break any other test running alongside it, so it is the only test in this
//! binary.

#![cfg(unix)]

use std::fs::File;
use std::sync::Arc;
use std::time::Duration;

use revscope::server_context::ServerContext;
use revscope::{ServerHooks, bind_listener, run_listener};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Notify;
use tokio::time::timeout;

#[tokio::test]
async fn accept_failing_for_lack_of_descriptors_does_not_stop_the_server() {
    let listener = bind_listener("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let shutdown = Arc::new(Notify::new());
    let server = tokio::spawn(run_listener(
        listener,
        ServerHooks::default(),
        shutdown.clone(),
        Arc::new(ServerContext::new_in_memory("accept-out-of-descriptors")),
    ));

    lower_open_file_limit(256);
    let mut filler = Vec::new();
    while let Ok(file) = File::open("/dev/null") {
        filler.push(file);
    }
    // Free one descriptor for the client, leaving none for the server to
    // accept the connection with.
    filler.pop();
    let mut client = TcpStream::connect(addr).await.expect("connect");
    client
        .write_all(b"GET /nope HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .expect("write");

    // accept() keeps failing with EMFILE meanwhile.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!server.is_finished(), "the accept loop must survive EMFILE");

    drop(filler);
    let mut response = Vec::new();
    timeout(Duration::from_secs(5), client.read_to_end(&mut response))
        .await
        .expect("the queued connection is served once descriptors free up")
        .expect("read");
    let response = String::from_utf8_lossy(&response);
    assert!(response.starts_with("HTTP/1.1 "), "{response}");

    shutdown.notify_waiters();
    timeout(Duration::from_secs(5), server)
        .await
        .expect("server stops on shutdown")
        .expect("join")
        .expect("server");
}

fn lower_open_file_limit(soft: libc::rlim_t) {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `limit` is a valid, writable rlimit.
    assert_eq!(
        unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) },
        0
    );
    limit.rlim_cur = soft.min(limit.rlim_max);
    // SAFETY: `limit` is a valid rlimit; the call only reads it.
    assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) }, 0);
}
