//! `POST /v2/public/mcp/token` — exchanges an authorization code for an MCP
//! access token.
//!
//! Public because the client is public: it holds no secret. What proves the
//! caller is the same client that started the flow is the PKCE verifier, which
//! only it knows, plus the code being bound to the client id and redirect URI it
//! was minted for. Codes are single use, so a replay finds nothing.
//!
//! No refresh token is issued. The access token lasts a year, which is the point
//! of putting a user through a browser consent screen to get one.

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as BASE64URL;
use revscope_api::oauth::{CLI_SCOPE, TokenRequest, TokenResponse};
use sha2::{Digest, Sha256};

use crate::auth;
use crate::auth::AuthError;
use crate::http::Response;
use crate::mcp::authorize::{AuthorizationGrant, GrantScope};
use crate::mcp::{MCP_SCOPE, invalid_request, oauth_error, oauth_json, token};
use crate::routes::map_auth_error;
use crate::server_context::ServerContext;
use crate::tokens::{API_TOKEN_TTL_SECONDS, MCP_TOKEN_TTL_SECONDS, TokenSecret};
use crate::util::block_on;

/// RFC 7636 section 4.1.
const MIN_CODE_VERIFIER_LEN: usize = 43;
const MAX_CODE_VERIFIER_LEN: usize = 128;

pub(crate) fn grant(server_context: Arc<ServerContext>, body: &[u8]) -> Response {
    let Some(request) = parse_token_request(body) else {
        return invalid_request("the token request must be form encoded");
    };
    if request.grant_type != "authorization_code" {
        return oauth_error(
            "400 Bad Request",
            "unsupported_grant_type",
            "only the authorization_code grant is supported",
        );
    }
    let Some(code) = TokenSecret::parse(&request.code) else {
        return invalid_grant("the authorization code is not valid");
    };

    // Redeemed whether or not the checks below pass: a code that was guessed at
    // once must not be worth guessing at twice.
    let Some(grant) = server_context.mcp_authorization_codes.take(code) else {
        return invalid_grant("the authorization code has expired or was already used");
    };
    if let Err(response) = check_grant_matches(&grant, &request) {
        return response;
    }

    if grant.scope == GrantScope::Cli {
        return issue_cli_token(server_context, &grant.user_id);
    }

    match block_on(token::issue_token(
        server_context,
        &grant.user_id,
        &grant.account_id,
        &grant.client_id,
    )) {
        Ok(Some(access_token)) => oauth_json(
            "200 OK",
            TokenResponse {
                access_token,
                token_type: "Bearer".to_string(),
                expires_in: i64::from(MCP_TOKEN_TTL_SECONDS),
                scope: MCP_SCOPE.to_string(),
            },
        ),
        Ok(None) => oauth_error(
            "400 Bad Request",
            "invalid_client",
            "the client registration has expired; register again",
        ),
        Err(AuthError::InvalidCredentials) => {
            invalid_grant("the user who approved this is no longer an account admin")
        }
        Err(error) => map_auth_error(error, "mcp_token"),
    }
}

/// An API token for the user who approved, with their role, like a signup's or login's.
fn issue_cli_token(server_context: Arc<ServerContext>, user_id: &str) -> Response {
    let token = block_on(async {
        let session = auth::issue_session(server_context.clone(), user_id)
            .await?
            .session;
        auth::extend_session_to_api_token(server_context, &session).await
    });
    match token {
        Ok(access_token) => oauth_json(
            "200 OK",
            TokenResponse {
                access_token,
                token_type: "Bearer".to_string(),
                expires_in: i64::from(API_TOKEN_TTL_SECONDS),
                scope: CLI_SCOPE.to_string(),
            },
        ),
        Err(error) => map_auth_error(error, "cli_token"),
    }
}

/// The code is bound to one client, one redirect URI, and one PKCE challenge.
/// All three are re-checked here so a stolen code is useless on its own.
fn check_grant_matches(grant: &AuthorizationGrant, request: &TokenRequest) -> Result<(), Response> {
    if request
        .client_id
        .as_deref()
        .is_some_and(|client_id| client_id != grant.client_id)
    {
        return Err(oauth_error(
            "400 Bad Request",
            "invalid_client",
            "the code was issued to a different client",
        ));
    }
    if request
        .redirect_uri
        .as_deref()
        .is_some_and(|redirect_uri| redirect_uri != grant.redirect_uri)
    {
        return Err(invalid_grant(
            "redirect_uri does not match the authorization request",
        ));
    }
    if !verify_code_challenge(&request.code_verifier, &grant.code_challenge) {
        return Err(invalid_grant("the code_verifier does not match"));
    }
    Ok(())
}

/// Form encoding is what RFC 6749 specifies; JSON is accepted too because some
/// MCP clients send it.
fn parse_token_request(body: &[u8]) -> Option<TokenRequest> {
    serde_urlencoded::from_bytes::<TokenRequest>(body)
        .ok()
        .or_else(|| serde_json::from_slice::<TokenRequest>(body).ok())
}

/// PKCE `S256`: the challenge is the base64url, without padding, of the sha256
/// of the verifier. `plain` is deliberately not supported.
fn verify_code_challenge(code_verifier: &str, code_challenge: &str) -> bool {
    if !(MIN_CODE_VERIFIER_LEN..=MAX_CODE_VERIFIER_LEN).contains(&code_verifier.len()) {
        return false;
    }
    let computed = BASE64URL.encode(Sha256::digest(code_verifier.as_bytes()));
    crate::crypto::constant_time_eq(computed.as_bytes(), code_challenge.as_bytes())
}

fn invalid_grant(description: &'static str) -> Response {
    oauth_error("400 Bad Request", "invalid_grant", description)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The example pair from RFC 7636 appendix B.
    const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

    #[test]
    fn accepts_the_verifier_its_challenge_was_derived_from() {
        assert!(verify_code_challenge(VERIFIER, CHALLENGE));
    }

    #[test]
    fn rejects_a_verifier_that_does_not_hash_to_the_challenge() {
        assert!(!verify_code_challenge(
            &VERIFIER.replace('d', "e"),
            CHALLENGE
        ));
        assert!(!verify_code_challenge(VERIFIER, "short"));
        // A verifier below the RFC length floor is rejected before hashing, so a
        // client cannot pick one weak enough to guess.
        assert!(!verify_code_challenge(
            "abc",
            &BASE64URL.encode(Sha256::digest(b"abc"))
        ));
    }

    #[test]
    fn reads_both_form_encoded_and_json_bodies() {
        let form = parse_token_request(
            b"grant_type=authorization_code&code=abc&code_verifier=xyz&client_id=id",
        )
        .expect("form body");
        assert_eq!(form.grant_type, "authorization_code");
        assert_eq!(form.code, "abc");
        assert_eq!(form.client_id.as_deref(), Some("id"));

        let json = parse_token_request(
            br#"{"grant_type":"authorization_code","code":"abc","code_verifier":"xyz"}"#,
        )
        .expect("json body");
        assert_eq!(json.code, "abc");
        assert_eq!(json.code_verifier, "xyz");

        assert!(parse_token_request(b"nonsense").is_none());
    }
}
