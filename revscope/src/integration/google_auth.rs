//! Access tokens for Google APIs, from a service account key or an OAuth
//! refresh token, shared by Search Console and the Google Analytics import.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde::{Deserialize, Serialize};

use crate::util::unix_seconds;

pub(crate) const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
/// Lifetime requested for a service account's token, and assumed for one whose
/// response doesn't say.
pub(crate) const GOOGLE_TOKEN_LIFETIME_SECS: i64 = 3600;

#[derive(Serialize)]
struct ServiceAccountClaims<'a> {
    iss: &'a str,
    scope: &'a str,
    aud: &'a str,
    iat: i64,
    exp: i64,
}

#[derive(Deserialize)]
pub(crate) struct GoogleToken {
    pub(crate) access_token: String,
    #[serde(default = "default_token_lifetime")]
    pub(crate) expires_in: i64,
    /// Only the authorization code exchange returns one.
    #[serde(default)]
    pub(crate) refresh_token: Option<String>,
    /// Present when the `openid` scope was granted.
    #[serde(default)]
    pub(crate) id_token: Option<String>,
}

impl GoogleToken {
    /// The stable id of the Google account this token is for. The signature goes
    /// unchecked because the token came straight from Google's token endpoint over
    /// TLS, which OpenID Connect allows, and it's never used to sign anyone in.
    pub(crate) fn subject(&self) -> Option<String> {
        let payload = self.id_token.as_deref()?.split('.').nth(1)?;
        let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
        let claims: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
        claims.get("sub")?.as_str().map(str::to_string)
    }
}

fn default_token_lifetime() -> i64 {
    GOOGLE_TOKEN_LIFETIME_SECS
}

pub(crate) enum GoogleTokenError {
    Http(reqwest::Error),
    /// The token endpoint is having trouble; the credential may be fine.
    Unavailable(String),
    /// Google refused our OAuth client, so our settings are wrong, whatever the
    /// credential is like.
    ClientRejected(String),
    /// The token endpoint refused the credential.
    Rejected(String),
    /// The credential can't be used to ask at all.
    InvalidCredential(String),
}

/// A form-encoded POST to Google's token endpoint.
pub(crate) async fn request_token(
    client: &reqwest::Client,
    token_url: &str,
    form: &[(&str, &str)],
) -> Result<GoogleToken, GoogleTokenError> {
    let body = serde_urlencoded::to_string(form)
        .map_err(|error| GoogleTokenError::InvalidCredential(error.to_string()))?;
    let response = client
        .post(token_url)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(GoogleTokenError::Http)?;
    let status = response.status();
    let text = response.text().await.map_err(GoogleTokenError::Http)?;
    if !status.is_success() {
        let message = format!("token request failed with {status}: {text}");
        return Err(classify_token_failure(&text, message));
    }
    serde_json::from_str(&text).map_err(|error| GoogleTokenError::Unavailable(error.to_string()))
}

/// Goes by OAuth's error code, since only `invalid_grant` is about the credential.
/// A status alone can't tell that from our own bad client secret or a proxy's refusal.
fn classify_token_failure(body: &str, message: String) -> GoogleTokenError {
    let code = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|body| body.get("error")?.as_str().map(str::to_string));
    match code.as_deref() {
        Some("invalid_grant") => GoogleTokenError::Rejected(message),
        Some("invalid_client" | "unauthorized_client" | "deleted_client") => {
            GoogleTokenError::ClientRejected(message)
        }
        _ => GoogleTokenError::Unavailable(message),
    }
}

pub(crate) async fn service_account_token(
    client: &reqwest::Client,
    token_url: &str,
    client_email: &str,
    private_key: &str,
    scope: &str,
) -> Result<GoogleToken, GoogleTokenError> {
    let assertion = service_account_assertion(token_url, client_email, private_key, scope)
        .map_err(GoogleTokenError::InvalidCredential)?;
    request_token(
        client,
        token_url,
        &[
            ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
            ("assertion", assertion.as_str()),
        ],
    )
    .await
}

fn service_account_assertion(
    token_url: &str,
    client_email: &str,
    private_key: &str,
    scope: &str,
) -> Result<String, String> {
    let now = unix_seconds();
    let claims = ServiceAccountClaims {
        iss: client_email,
        scope,
        aud: token_url,
        iat: now,
        exp: now + GOOGLE_TOKEN_LIFETIME_SECS,
    };
    let private_key = normalize_private_key(private_key);
    let key = EncodingKey::from_rsa_pem(private_key.as_bytes())
        .map_err(|error| format!("invalid private key: {error}"))?;
    encode(&Header::new(Algorithm::RS256), &claims, &key)
        .map_err(|error| format!("failed to sign auth assertion: {error}"))
}

fn normalize_private_key(private_key: &str) -> String {
    private_key.trim().replace("\\n", "\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_newline_escapes_in_a_pasted_key_are_restored() {
        assert_eq!(
            normalize_private_key("  -----BEGIN\\nKEY-----\n"),
            "-----BEGIN\nKEY-----"
        );
    }

    #[test]
    fn only_invalid_grant_blames_the_credential() {
        let classify = |body: &str| classify_token_failure(body, String::new());
        assert!(matches!(
            classify(r#"{"error":"invalid_grant"}"#),
            GoogleTokenError::Rejected(_)
        ));
        assert!(matches!(
            classify(r#"{"error":"invalid_client"}"#),
            GoogleTokenError::ClientRejected(_)
        ));
        assert!(matches!(
            classify("<html>403 from a proxy</html>"),
            GoogleTokenError::Unavailable(_)
        ));
    }

    #[test]
    fn the_subject_comes_from_the_id_token() {
        let token = |id_token: Option<String>| GoogleToken {
            access_token: String::new(),
            expires_in: 0,
            refresh_token: None,
            id_token,
        };
        let payload = URL_SAFE_NO_PAD.encode(r#"{"sub":"google-1"}"#);
        assert_eq!(
            token(Some(format!("header.{payload}.signature"))).subject(),
            Some("google-1".to_string())
        );
        assert_eq!(token(None).subject(), None);
        assert_eq!(token(Some("garbage".to_string())).subject(), None);
    }

    #[test]
    fn a_malformed_key_is_reported_rather_than_signed() {
        assert!(service_account_assertion(GOOGLE_TOKEN_URL, "a@b", "nope", "scope").is_err());
    }
}
