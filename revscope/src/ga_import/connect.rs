//! Connecting a Google credential: the OAuth consent redirect, or a pasted service
//! account key, ending with the properties it can read.

use std::sync::Arc;

pub(crate) use revscope_api::ga_import::ConnectRequest;
use revscope_api::ga_import::{ConnectResponse, OAuthUrlResponse};
use url::Url;

use crate::auth::AuthError;
use crate::authorize::AdminProject;
use crate::db::{open_main_database_write, open_project_database_read};
use crate::server_context::ServerContext;
use crate::util::{block_on, web_base_url};

use super::api::{self, GA_SCOPE, GaApiError, GaProperty};
use super::credential::GaCredential;
use super::store;

const GOOGLE_CONSENT_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
/// Time to finish Google's consent screen.
const OAUTH_STATE_LIFETIME: &str = "30 minutes";
/// Time from connecting to pressing start. Expired connections get revoked by the worker.
const CONNECTION_LIFETIME: &str = "2 hours";
const OAUTH_REDIRECT_PATH: &str = "/oauth/google-analytics";

/// The web client asking for access, with the id the sign-in button uses.
#[derive(Debug, Clone)]
pub(crate) struct GoogleOAuthClient {
    pub(crate) client_id: String,
    pub(crate) client_secret: String,
}

impl GoogleOAuthClient {
    pub(crate) fn from_env() -> Option<Self> {
        let read = |name: &str| {
            std::env::var(name)
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };
        Some(Self {
            client_id: read("REVSCOPE_GOOGLE_CLIENT_ID")?,
            client_secret: read("REVSCOPE_GOOGLE_CLIENT_SECRET")?,
        })
    }
}

fn redirect_uri() -> String {
    format!("{}{OAUTH_REDIRECT_PATH}", web_base_url())
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).expect("system RNG must be available");
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Starts Google's consent flow. The state starts with the project id so the
/// callback page knows which project's route to finish on; the server then checks
/// it against the row, so the prefix grants nothing.
pub(crate) fn oauth_url(
    server_context: Arc<ServerContext>,
    project: &AdminProject,
) -> Result<OAuthUrlResponse, AuthError> {
    let oauth = server_context
        .google_oauth
        .clone()
        .ok_or(AuthError::InvalidRequest("Google OAuth isn't configured"))?;
    let state = format!("{}.{}", project.id(), random_token());
    let project_key = project.key();
    let user_id = project.admin().user().user_id.clone();
    block_on(async {
        let conn = open_main_database_write(server_context.clone()).await?;
        conn.execute(
            &format!(
                "INSERT INTO ga_import_connections (project_id, created_by, oauth_state, expires_at)
                 VALUES ($1, $2::text::uuid, $3, NOW() + INTERVAL '{OAUTH_STATE_LIFETIME}')"
            ),
            &[&project_key, &user_id, &state],
        )
        .await
        .map_err(AuthError::from)?;
        Ok::<(), AuthError>(())
    })?;
    let mut url = Url::parse(GOOGLE_CONSENT_URL).expect("the consent URL parses");
    url.query_pairs_mut()
        .append_pair("client_id", &oauth.client_id)
        .append_pair("redirect_uri", &redirect_uri())
        .append_pair("response_type", "code")
        // openid names the Google account the token is for; see revoke_stranded.
        .append_pair("scope", &format!("openid {GA_SCOPE}"))
        // A refresh token is what lets the job outlast the first access token,
        // and Google only sends one on a fresh consent.
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent")
        .append_pair("state", &state);
    Ok(OAuthUrlResponse { url: url.into() })
}

pub(crate) fn connect(
    server_context: Arc<ServerContext>,
    project: &AdminProject,
    request: ConnectRequest,
) -> Result<ConnectResponse, AuthError> {
    let (connection_id, properties) = block_on(connect_async(&server_context, project, request))?;
    let conn = open_project_database_read(server_context.as_ref(), project.id())?;
    let known_hostnames = store::known_hostnames(&conn).map_err(crate::auth::map_row_error)?;
    Ok(ConnectResponse {
        connection_id,
        properties,
        known_hostnames,
    })
}

async fn connect_async(
    server_context: &Arc<ServerContext>,
    project: &AdminProject,
    request: ConnectRequest,
) -> Result<(String, Vec<GaProperty>), AuthError> {
    let user_id = project.admin().user().user_id.clone();
    let project_key = project.key();
    match request {
        ConnectRequest::ServiceAccount {
            service_account_json,
        } => {
            let credential = GaCredential::from_service_account_json(&service_account_json).ok_or(
                AuthError::InvalidRequest(
                    "that isn't a service account JSON key: it needs client_email and private_key",
                ),
            )?;
            let token = api::fetch_access_token(server_context, &credential)
                .await
                .map_err(connect_error)?;
            let properties = api::list_properties(server_context, &token.access_token)
                .await
                .map_err(connect_error)?;
            let (email, secret) = credential.encrypt();
            let conn = open_main_database_write(server_context.clone()).await?;
            let connection_id: String = conn
                .query_one(
                    &format!(
                        "INSERT INTO ga_import_connections
                            (project_id, created_by, encrypted_client_email, encrypted_secret, expires_at)
                         VALUES ($1, $2::text::uuid, $3, $4, NOW() + INTERVAL '{CONNECTION_LIFETIME}')
                         RETURNING id::text"
                    ),
                    &[&project_key, &user_id, &email, &secret],
                )
                .await
                .map_err(AuthError::from)?
                .get(0);
            Ok((connection_id, properties))
        }
        ConnectRequest::OAuth {
            oauth_code,
            oauth_state,
        } => {
            let conn = open_main_database_write(server_context.clone()).await?;
            // Consumed here, before the code exchange, so a state works only once.
            let connection_id: String = conn
                .query_opt(
                    "UPDATE ga_import_connections
                     SET oauth_state = NULL
                     WHERE oauth_state = $1
                       AND project_id = $2
                       AND created_by = $3::text::uuid
                       AND expires_at > NOW()
                     RETURNING id::text",
                    &[&oauth_state, &project_key, &user_id],
                )
                .await
                .map_err(AuthError::from)?
                .ok_or(AuthError::InvalidRequest(
                    "this Google sign-in has expired; start again from Settings",
                ))?
                .get(0);
            let token = api::exchange_oauth_code(server_context, &oauth_code, &redirect_uri())
                .await
                .map_err(connect_error)?;
            let subject = token.subject();
            let refresh_token = token.refresh_token.ok_or(AuthError::InvalidRequest(
                "Google didn't grant offline access; try connecting again",
            ))?;
            let credential = GaCredential::OAuth { refresh_token };
            let (_, secret) = credential.encrypt();
            let stored = conn
                .execute(
                    &format!(
                        "UPDATE ga_import_connections
                         SET encrypted_secret = $2,
                             oauth_subject = $3,
                             expires_at = NOW() + INTERVAL '{CONNECTION_LIFETIME}'
                         WHERE id = $1::text::uuid"
                    ),
                    &[&connection_id, &secret, &subject],
                )
                .await
                .map_err(AuthError::from)?;
            if stored == 0 {
                return Err(AuthError::InvalidRequest(
                    "this Google sign-in has expired; start again from Settings",
                ));
            }
            let properties = api::list_properties(server_context, &token.access_token)
                .await
                .map_err(connect_error)?;
            Ok((connection_id, properties))
        }
    }
}

pub(super) fn connect_error(error: GaApiError) -> AuthError {
    match error {
        GaApiError::Denied(message) | GaApiError::TokenRejected(message) => {
            log::info!("Google refused a Google Analytics credential: {message}");
            AuthError::InvalidRequest(
                "Google refused this credential. Check that the key is current and the Google Analytics Admin and Data APIs are enabled for its Google Cloud project.",
            )
        }
        GaApiError::Rejected(message) => {
            log::warn!("Google rejected a Google Analytics request: {message}");
            AuthError::InvalidRequest("Google rejected the request; please try again")
        }
        GaApiError::ClientSettings(message) => {
            log::error!("Google refused this server's OAuth client: {message}");
            AuthError::Temporary("Google sign-in isn't set up correctly on this server")
        }
        GaApiError::Quota => {
            AuthError::Temporary("Google Analytics is out of quota; try again in an hour")
        }
        GaApiError::Transient(message) => {
            log::warn!("couldn't reach Google Analytics: {message}");
            AuthError::Temporary("couldn't reach Google Analytics; please try again")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_requests_are_told_apart_by_their_fields() {
        let service: ConnectRequest =
            serde_json::from_str(r#"{"service_account_json":"{}"}"#).expect("service");
        assert!(matches!(service, ConnectRequest::ServiceAccount { .. }));
        let oauth: ConnectRequest =
            serde_json::from_str(r#"{"oauth_code":"c","oauth_state":"s"}"#).expect("oauth");
        assert!(matches!(oauth, ConnectRequest::OAuth { .. }));
    }
}
