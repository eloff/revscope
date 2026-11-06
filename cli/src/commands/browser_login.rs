//! Signing in through a browser, for accounts with no password, such as those made
//! with Google or Apple. The standard OAuth loopback flow with PKCE: the server's
//! consent page sends the browser back to a port this process is listening on.

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as BASE64URL;
use revscope_api::oauth::{
    AuthorizationServerMetadata, CLI_SCOPE, RegisterRequest, RegisterResponse, TokenRequest,
    TokenResponse,
};
use revscope_api::paths;
use sha2::{Digest, Sha256};
use url::Url;

use crate::context::Context;
use crate::error::{CliError, Exit, Result};
use crate::output::print_stderr;

const CALLBACK_PATH: &str = "/callback";
const POLL_EVERY: Duration = Duration::from_millis(100);
/// A connection that never finishes its request mustn't hold up the real redirect.
const READ_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_REQUEST_BYTES: usize = 16 * 1024;

pub struct Options {
    pub open_browser: bool,
    pub timeout: Duration,
}

/// Returns the API token the approval bought.
pub fn sign_in(context: &Context, options: &Options) -> Result<String> {
    let metadata: AuthorizationServerMetadata = context.client.get(paths::OAUTH_METADATA)?;
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|error| CliError::local(format!("listening for the browser: {error}")))?;
    let port = listener
        .local_addr()
        .map_err(|error| CliError::local(format!("listening for the browser: {error}")))?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}{CALLBACK_PATH}");

    let client: RegisterResponse = context.client.post(
        paths::OAUTH_REGISTER,
        &RegisterRequest {
            client_name: Some("RevScope CLI".to_string()),
            redirect_uris: vec![redirect_uri.clone()],
            grant_types: Some(vec!["authorization_code".to_string()]),
            token_endpoint_auth_method: Some("none".to_string()),
        },
    )?;

    let verifier = random_token()?;
    let state = random_token()?;
    let challenge = BASE64URL.encode(Sha256::digest(verifier.as_bytes()));
    let mut url =
        Url::parse(&metadata.authorization_endpoint).map_err(|error| CliError::BadResponse {
            path: paths::OAUTH_METADATA.to_string(),
            detail: error.to_string(),
        })?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &client.client_id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("scope", CLI_SCOPE)
        .append_pair("state", &state);

    // On stderr, so it's seen even under --json, whose result goes to stdout.
    print_stderr(&format!("Approve this sign-in in a browser:\n\n  {url}\n"));
    if options.open_browser {
        open_in_browser(url.as_str());
    }

    let code = wait_for_code(&listener, &state, options.timeout)?;
    let token: TokenResponse = context.client.post(
        paths::OAUTH_TOKEN,
        &TokenRequest {
            grant_type: "authorization_code".to_string(),
            code,
            redirect_uri: Some(redirect_uri),
            client_id: Some(client.client_id),
            code_verifier: verifier,
        },
    )?;
    // An older server takes this for an MCP client, whose token can't run the CLI.
    if token.scope != CLI_SCOPE {
        return Err(CliError::Failed {
            exit: Exit::Failure,
            message: "this server doesn't support signing in from the CLI through a browser; use --email or upgrade it".to_string(),
        });
    }
    Ok(token.access_token)
}

/// 256 random bits in the unreserved alphabet PKCE allows.
fn random_token() -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|error| CliError::local(format!("getting random bytes: {error}")))?;
    Ok(BASE64URL.encode(bytes))
}

/// Best effort: the URL is already printed for whoever is running this. Output is
/// discarded so a browser's own messages don't land in the command's.
fn open_in_browser(url: &str) {
    let mut command = if cfg!(target_os = "macos") {
        let mut command = Command::new("open");
        command.arg(url);
        command
    } else if cfg!(windows) {
        let mut command = Command::new("rundll32");
        command.args(["url.dll,FileProtocolHandler", url]);
        command
    } else {
        let mut command = Command::new("xdg-open");
        command.arg(url);
        command
    };
    let _ = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

fn wait_for_code(listener: &TcpListener, state: &str, timeout: Duration) -> Result<String> {
    listener
        .set_nonblocking(true)
        .map_err(|error| CliError::local(format!("listening for the browser: {error}")))?;
    let deadline = Instant::now() + timeout;
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                if let Some(outcome) = handle_request(stream, state) {
                    return outcome;
                }
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err(CliError::Failed {
                        exit: Exit::Retry,
                        message: format!(
                            "nobody approved the sign-in within {} seconds",
                            timeout.as_secs()
                        ),
                    });
                }
                thread::sleep(POLL_EVERY);
            }
            Err(error) => {
                return Err(CliError::local(format!(
                    "listening for the browser: {error}"
                )));
            }
        }
    }
}

/// `None` for anything that isn't the awaited redirect, such as a favicon request
/// or some other local process poking the port, which then can't end the wait.
fn handle_request(mut stream: TcpStream, state: &str) -> Option<Result<String>> {
    let target = read_request_target(&mut stream)?;
    let outcome = parse_callback(&target, state);
    let page = match &outcome {
        Some(Ok(_)) => "Signed in. You can close this tab and return to your terminal.",
        Some(Err(_)) => "The sign-in did not complete. Your terminal says why.",
        None => "Nothing here.",
    };
    let status = if outcome.is_some() {
        "200 OK"
    } else {
        "404 Not Found"
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}",
        page.len()
    );
    outcome
}

fn read_request_target(stream: &mut TcpStream) -> Option<String> {
    stream.set_nonblocking(false).ok()?;
    stream.set_read_timeout(Some(READ_TIMEOUT)).ok()?;
    let mut buffer = vec![0u8; MAX_REQUEST_BYTES];
    let mut filled = 0;
    while filled < buffer.len() {
        let read = stream.read(&mut buffer[filled..]).ok()?;
        if read == 0 {
            break;
        }
        filled += read;
        if buffer[..filled].windows(2).any(|pair| pair == b"\r\n") {
            break;
        }
    }
    let line = std::str::from_utf8(&buffer[..filled])
        .ok()?
        .lines()
        .next()?;
    let mut parts = line.split(' ');
    (parts.next()? == "GET").then(|| parts.next().map(str::to_string))?
}

/// The outcome the redirect carries. `None` unless it's for this path and carries
/// this sign-in's state, since any page could send a browser to this port.
fn parse_callback(target: &str, state: &str) -> Option<Result<String>> {
    let url = Url::parse(&format!("http://localhost{target}")).ok()?;
    if url.path() != CALLBACK_PATH {
        return None;
    }
    let value = |name: &str| {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    };
    if value("state").as_deref() != Some(state) {
        return None;
    }
    if let Some(code) = value("code") {
        return Some(Ok(code));
    }
    let reason = value("error_description")
        .or_else(|| value("error"))
        .unwrap_or_else(|| "the redirect carried no code".to_string());
    Some(Err(CliError::Failed {
        exit: Exit::Auth,
        message: format!("the sign-in was not approved: {reason}"),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_redirect_gives_up_its_code_only_with_the_right_state() {
        let code = parse_callback("/callback?code=abc&state=s1", "s1")
            .unwrap()
            .unwrap();
        assert_eq!(code, "abc");
        assert!(parse_callback("/callback?code=abc&state=other", "s1").is_none());
        assert!(parse_callback("/callback?code=abc", "s1").is_none());
    }

    #[test]
    fn other_requests_are_ignored() {
        assert!(parse_callback("/favicon.ico", "s1").is_none());
        assert!(parse_callback("/other?code=abc&state=s1", "s1").is_none());
    }

    #[test]
    fn a_denial_is_reported_with_its_reason() {
        let denied = parse_callback(
            "/callback?error=access_denied&error_description=the+user+declined&state=s1",
            "s1",
        )
        .unwrap()
        .unwrap_err();
        assert_eq!(denied.exit(), Exit::Auth);
        assert!(denied.to_string().contains("the user declined"), "{denied}");
    }

    #[test]
    fn a_random_token_is_a_legal_pkce_verifier() {
        let token = random_token().unwrap();
        assert!((43..=128).contains(&token.len()), "{}", token.len());
        assert_ne!(token, random_token().unwrap());
    }
}
