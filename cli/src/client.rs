//! Typed HTTP access to the RevScope API.

use std::thread;
use std::time::Duration;

use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{AUTHORIZATION, RETRY_AFTER};
use reqwest::{Certificate, StatusCode};
use revscope_api::errors::ErrorBody;
use serde::Serialize;
use serde::de::DeserializeOwned;
use url::{Host, Url};

use crate::error::{CliError, Result};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// Reads are safe to repeat, and a restart or a brief overload shouldn't fail a
/// command. Writes are never retried: the first may have taken effect.
const READ_RETRY_DELAYS: [Duration; 2] = [Duration::from_millis(500), Duration::from_millis(1500)];

/// The API's base URL in the form credentials are keyed by. Plain `http` is
/// refused except to a loopback host, since passwords and tokens would otherwise
/// cross the network unencrypted.
pub fn normalize_api_url(input: &str) -> Result<String> {
    let url = Url::parse(input.trim()).map_err(|error| {
        CliError::usage(format!(
            "`{input}` is not an API URL ({error}); it should look like https://api.revscope.co"
        ))
    })?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(CliError::usage(
            "the API URL must not contain a username or password",
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(CliError::usage(
            "the API URL must not contain a query or fragment",
        ));
    }
    let loopback = match url.host() {
        Some(Host::Domain(domain)) => domain == "localhost",
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    };
    match url.scheme() {
        "https" => {}
        "http" if loopback => {}
        "http" => {
            return Err(CliError::usage(format!(
                "refusing http:// to {}: use https://, or a loopback address for local development",
                url.host_str().unwrap_or_default()
            )));
        }
        other => {
            return Err(CliError::usage(format!(
                "unsupported URL scheme `{other}`; use https://"
            )));
        }
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

/// Trusts the system's certificates, as a corporate proxy may need. A minimal container
/// has none, which fails right here, so it falls back on the ones built in.
fn http_client() -> reqwest::Result<Client> {
    let builder = || {
        Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .user_agent(concat!("revscope-cli/", env!("CARGO_PKG_VERSION")))
    };
    builder().build().or_else(|_| {
        let bundled = webpki_root_certs::TLS_SERVER_ROOT_CERTS
            .iter()
            .filter_map(|certificate| Certificate::from_der(certificate).ok());
        builder().tls_certs_only(bundled).build()
    })
}

pub struct ApiClient {
    base: String,
    credential: Option<String>,
    http: Client,
}

impl ApiClient {
    /// `base` comes from [`normalize_api_url`].
    pub fn new(base: &str, credential: Option<String>) -> Result<Self> {
        let http = http_client()
            .map_err(|error| CliError::local(format!("starting the HTTP client: {error}")))?;
        Ok(Self {
            base: base.to_string(),
            credential,
            http,
        })
    }

    pub fn has_credential(&self) -> bool {
        self.credential.is_some()
    }

    pub fn get<R: DeserializeOwned>(&self, path: &str) -> Result<R> {
        let response = self.send_read(path)?;
        decode(path, response)
    }

    /// A `GET` whose answer is plain text, such as the schema's SQL.
    pub fn get_text(&self, path: &str) -> Result<String> {
        let response = self.send_read(path)?;
        check_status(response)?
            .text()
            .map_err(|error| self.unreachable(error))
    }

    pub fn post<B: Serialize + ?Sized, R: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<R> {
        self.post_with_timeout(path, body, DEFAULT_TIMEOUT)
    }

    pub fn post_with_timeout<B: Serialize + ?Sized, R: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
        timeout: Duration,
    ) -> Result<R> {
        let request = self.http.post(self.url(path)).timeout(timeout).json(body);
        decode(path, self.send(request)?)
    }

    /// A `POST` with no body, answered with one.
    pub fn post_empty<R: DeserializeOwned>(&self, path: &str) -> Result<R> {
        decode(path, self.send(self.http.post(self.url(path)))?)
    }

    pub fn delete<R: DeserializeOwned>(&self, path: &str) -> Result<R> {
        decode(path, self.send(self.http.delete(self.url(path)))?)
    }

    fn send_read(&self, path: &str) -> Result<Response> {
        let mut delays = READ_RETRY_DELAYS.iter();
        loop {
            let outcome = self.send(self.http.get(self.url(path)));
            let retryable = match &outcome {
                Ok(response) => matches!(
                    response.status(),
                    StatusCode::BAD_GATEWAY
                        | StatusCode::SERVICE_UNAVAILABLE
                        | StatusCode::GATEWAY_TIMEOUT
                ),
                Err(_) => true,
            };
            match delays.next().filter(|_| retryable) {
                Some(delay) => thread::sleep(*delay),
                None => return outcome,
            }
        }
    }

    fn send(&self, request: RequestBuilder) -> Result<Response> {
        let request = match &self.credential {
            Some(credential) => request.header(AUTHORIZATION, format!("Bearer {credential}")),
            None => request,
        };
        request.send().map_err(|error| self.unreachable(error))
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    fn unreachable(&self, error: reqwest::Error) -> CliError {
        // Malformed by this side, so retrying can't help.
        if error.is_builder() {
            return CliError::local(format!(
                "could not build the request: {}",
                error.without_url()
            ));
        }
        // The URL is ours, so the error's copy of it adds nothing.
        CliError::Unreachable {
            url: self.base.clone(),
            detail: error.without_url().to_string(),
        }
    }
}

fn decode<R: DeserializeOwned>(path: &str, response: Response) -> Result<R> {
    let response = check_status(response)?;
    let text = response.text().map_err(|error| CliError::BadResponse {
        path: path.to_string(),
        detail: error.without_url().to_string(),
    })?;
    serde_json::from_str(&text).map_err(|error| CliError::BadResponse {
        path: path.to_string(),
        detail: error.to_string(),
    })
}

fn check_status(response: Response) -> Result<Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let retry_after_seconds = response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok());
    let body = response.text().unwrap_or_default();
    let parsed = serde_json::from_str::<ErrorBody>(&body).ok();
    let (message, code) = match parsed {
        Some(parsed) => (parsed.error, parsed.code),
        // Not our API's error, such as a proxy's HTML page: the status says enough.
        None => (String::new(), None),
    };
    Err(CliError::Api {
        status: status.as_u16(),
        code,
        message,
        retry_after_seconds,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_server_has_one_credentials_key() {
        assert_eq!(
            normalize_api_url("HTTP://LOCALHOST:4320/").unwrap(),
            "http://localhost:4320"
        );
        assert_eq!(
            normalize_api_url(" https://api.revscope.co/ ").unwrap(),
            "https://api.revscope.co"
        );
    }

    #[test]
    fn plain_http_is_only_accepted_to_loopback() {
        assert!(normalize_api_url("http://127.0.0.1:4320").is_ok());
        assert!(normalize_api_url("http://127.0.0.2:4320").is_ok());
        assert!(normalize_api_url("http://[::1]:4320").is_ok());
        assert!(normalize_api_url("http://api.revscope.co").is_err());
        assert!(normalize_api_url("http://192.168.1.5:4320").is_err());
        assert!(normalize_api_url("ftp://api.revscope.co").is_err());
        assert!(normalize_api_url("api.revscope.co").is_err());
    }

    #[test]
    fn a_loopback_lookalike_is_not_loopback() {
        assert!(normalize_api_url("http://localhost:x@evil.invalid").is_err());
        assert!(normalize_api_url("http://127.0.0.1:x@evil.invalid").is_err());
        assert!(normalize_api_url("http://localhost@evil.invalid").is_err());
        assert!(normalize_api_url("http://localhost.evil.invalid").is_err());
        assert!(normalize_api_url("https://user:pw@api.revscope.co").is_err());
    }
}
