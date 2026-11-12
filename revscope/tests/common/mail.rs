//! A local stand-in for MailChannels that keeps every message it is sent, for
//! tests of links that only ever leave the server in an email.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Mutex, Once};
use std::time::Duration;

use serde_json::{Value, json};

static MAIL_SERVER: Once = Once::new();
static SENT_EMAILS: Mutex<Vec<Value>> = Mutex::new(Vec::new());

/// The paths of the links our emails carry, up to the token.
pub const RESET_LINK: &str = "/reset-password?token=";
pub const VERIFY_LINK: &str = "/verify-email?token=";

/// Points MailChannels at a local listener for the whole test binary. Every
/// test that sends mail calls this before it starts a server.
pub fn start_mail_server() {
    MAIL_SERVER.call_once(|| {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mail server");
        let addr = listener.local_addr().expect("mail server address");
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                if let Some(email) = read_email(stream) {
                    SENT_EMAILS.lock().expect("sent emails").push(email);
                }
            }
        });
        // SAFETY: every test in the binary calls this before it starts a
        // server, and the `Once` makes the first caller do it alone.
        unsafe {
            std::env::set_var("MAILCHANNELS_API_URL", format!("http://{addr}/send"));
            std::env::set_var("MAILCHANNELS_API_KEY", "test-key");
            std::env::set_var("MAILCHANNELS_FROM_EMAIL", "noreply@example.com");
        }
    });
}

fn read_email(stream: std::net::TcpStream) -> Option<Value> {
    let mut reader = BufReader::new(stream);
    let mut content_length = 0;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).ok()?;
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            content_length = value.trim().parse().ok()?;
        }
    }
    let mut body = vec![0; content_length];
    reader.read_exact(&mut body).ok()?;
    let mut stream = reader.into_inner();
    stream
        .write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
        .ok()?;
    serde_json::from_slice(&body).ok()
}

pub fn emails_sent_to(address: &str) -> Vec<Value> {
    SENT_EMAILS
        .lock()
        .expect("sent emails")
        .iter()
        .filter(|email| email["personalizations"][0]["to"][0]["email"] == json!(address))
        .cloned()
        .collect()
}

/// The tokens of the `link` links sent to `address`, oldest first.
pub fn links_sent_to(address: &str, link: &str) -> Vec<String> {
    emails_sent_to(address)
        .iter()
        .filter_map(|email| {
            let text = email["content"][0]["value"].as_str()?;
            let rest = text.split(link).nth(1)?;
            let token = rest.split_whitespace().next()?;
            // An invite link carries `&invite=1` after the token.
            Some(token.split('&').next()?.to_string())
        })
        .collect()
}

/// Waits for more than `already` `link` links to reach `address`, since mail is
/// sent in the background after the response, and returns the newest.
pub async fn new_link_sent_to(address: &str, link: &str, already: usize) -> String {
    for _ in 0..250 {
        let mut links = links_sent_to(address, link);
        if links.len() > already {
            return links.pop().expect("a link");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("no new {link} link was sent to {address}");
}
