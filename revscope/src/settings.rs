//! Project integration credentials and project settings (timezone, custom domain).
//!
//! Backed by the main Postgres `integrations` and `project_settings` tables.
//! The whole settings page is loaded with one GET and persisted with one POST.
//! API keys and secrets are stored encrypted (see [`crate::crypto`]) and are
//! never returned to the client: only a masked preview (asterisks + last 6
//! characters) is exposed.

use std::sync::Arc;

use revscope_api::integrations;
pub(crate) use revscope_api::settings::{
    IntegrationInput, IntegrationView, SaveSettingsRequest, SettingsResponse,
};
use tokio_postgres::error::SqlState;
use tokio_postgres::types::ToSql;

use crate::auth::AuthError;
use crate::authorize::{AdminProject, OwnedProject};
use crate::crypto;
use crate::db::main::Connection;
use crate::db::{open_main_database_read, open_main_database_write};
use crate::server_context::ServerContext;
use crate::timezone::{self, DEFAULT_TIMEZONE};
use crate::util::block_on;

const MAX_PAYMENT_SUCCESS_PATH_LEN: usize = 2048;
const MAX_CUSTOM_DOMAIN_LEN: usize = 253;

pub(crate) fn get_settings(
    server_context: Arc<ServerContext>,
    project: &OwnedProject,
) -> Result<SettingsResponse, AuthError> {
    let project_key = project.key();

    block_on(async move {
        let conn = open_main_database_read(server_context).await?;
        let (timezone, custom_domain) = load_project_settings(&conn, project_key).await?;
        let payment_success_path = load_payment_success_path(&conn, project_key).await?;
        let integrations = load_integrations(&conn, project_key).await?;
        Ok(SettingsResponse {
            timezone,
            payment_success_path,
            custom_domain,
            integrations,
        })
    })
}

pub(crate) fn save_settings(
    server_context: Arc<ServerContext>,
    project: &AdminProject,
    request: SaveSettingsRequest,
) -> Result<SettingsResponse, AuthError> {
    for integration in &request.integrations {
        if integrations::find(&integration.integration_type).is_none() {
            return Err(AuthError::InvalidRequest("unknown integration type"));
        }
    }

    let timezone = timezone_to_save(request.timezone.as_deref());
    let payment_success_path = request
        .payment_success_path
        .as_ref()
        .map(|value| validate_payment_success_path(value.as_deref()))
        .transpose()?;
    let custom_domain = request
        .custom_domain
        .as_ref()
        .map(|value| validate_custom_domain(value.as_deref()))
        .transpose()?;

    let project_key = project.key();
    let timezone_saved = timezone.is_some();

    {
        let server_context = server_context.clone();
        block_on(async move {
            let conn = open_main_database_write(server_context.clone()).await?;
            if let Some(timezone) = timezone {
                conn.execute(
                    "INSERT INTO project_settings (project_id, timezone) VALUES ($1, $2)
                     ON CONFLICT (project_id) DO UPDATE SET timezone = EXCLUDED.timezone",
                    &[&project_key, &timezone],
                )
                .await
                .map_err(AuthError::from)?;
            }
            if let Some(payment_success_path) = payment_success_path {
                conn.execute(
                    "UPDATE projects SET payment_success_path = $2 WHERE id = $1",
                    &[&project_key, &payment_success_path],
                )
                .await
                .map_err(AuthError::from)?;
            }
            if let Some(custom_domain) = custom_domain {
                let previous = load_custom_domain(&conn, project_key).await?;
                conn.execute(
                    "INSERT INTO project_settings (project_id, custom_domain) VALUES ($1, $2)
                     ON CONFLICT (project_id) DO UPDATE SET custom_domain = EXCLUDED.custom_domain",
                    &[&project_key, &custom_domain],
                )
                .await
                .map_err(map_custom_domain_error)?;
                server_context
                    .acme
                    .replace_custom_domain(previous.as_deref(), custom_domain.as_deref());
                if let Some(domain) = custom_domain.as_deref() {
                    crate::acme::request_issuance(server_context.clone(), domain);
                }
            }
            for integration in &request.integrations {
                upsert_integration(&conn, project_key, integration).await?;
            }
            Ok::<(), AuthError>(())
        })?;
    }

    if timezone_saved {
        // Bucket boundaries and the cached dashboard bodies built from them are
        // both derived from the old zone, so neither may outlive the save.
        timezone::invalidate(server_context.clone(), project_key);
        server_context.demo_overview_cache.clear();
    }

    get_settings(server_context, project)
}

async fn load_project_settings(
    conn: &Connection,
    project_id: i32,
) -> Result<(String, Option<String>), AuthError> {
    let row = conn
        .query_opt(
            "SELECT timezone, custom_domain FROM project_settings WHERE project_id = $1",
            &[&project_id],
        )
        .await
        .map_err(AuthError::from)?;
    Ok(row
        .map(|row| (row.get(0), row.get(1)))
        .unwrap_or_else(|| (DEFAULT_TIMEZONE.to_string(), None)))
}

async fn load_custom_domain(
    conn: &Connection,
    project_id: i32,
) -> Result<Option<String>, AuthError> {
    let row = conn
        .query_opt(
            "SELECT custom_domain FROM project_settings WHERE project_id = $1",
            &[&project_id],
        )
        .await
        .map_err(AuthError::from)?;
    Ok(row.and_then(|row| row.get(0)))
}

async fn load_payment_success_path(
    conn: &Connection,
    project_id: i32,
) -> Result<Option<String>, AuthError> {
    let row = conn
        .query_opt(
            "SELECT payment_success_path FROM projects WHERE id = $1",
            &[&project_id],
        )
        .await
        .map_err(AuthError::from)?;
    Ok(row.and_then(|row| row.get::<_, Option<String>>(0)))
}

async fn load_integrations(
    conn: &Connection,
    project_id: i32,
) -> Result<Vec<IntegrationView>, AuthError> {
    let params: [&(dyn ToSql + Sync); 1] = [&project_id];
    let rows = conn
        .query(
            "SELECT
                i.integration_type::text,
                i.external_account_id,
                i.encrypted_api_key,
                i.encrypted_secret,
                i.last_sync_success_at::text,
                latest.error_text,
                latest.started_at::text
             FROM integrations i
             LEFT JOIN LATERAL (
                SELECT error_text, started_at
                FROM integration_syncs s
                WHERE s.integration_id = i.id
                ORDER BY s.started_at DESC
                LIMIT 1
             ) latest ON true
             WHERE i.project_id = $1
             ORDER BY i.integration_type",
            &params,
        )
        .await
        .map_err(AuthError::from)?;

    Ok(rows
        .iter()
        .map(|row| {
            let encrypted_api_key: Option<Vec<u8>> = row.get(2);
            let encrypted_secret: Option<Vec<u8>> = row.get(3);
            IntegrationView {
                integration_type: row.get(0),
                external_account_id: row.get(1),
                api_key_masked: encrypted_api_key
                    .as_deref()
                    .and_then(crypto::mask_stored_secret),
                secret_masked: encrypted_secret
                    .as_deref()
                    .and_then(crypto::mask_stored_secret),
                last_synced_at: row.get(4),
                last_sync_error: row.get(5),
                last_sync_attempted_at: row.get(6),
            }
        })
        .collect())
}

async fn upsert_integration(
    conn: &Connection,
    project_id: i32,
    input: &IntegrationInput,
) -> Result<(), AuthError> {
    let integration_type = input.integration_type.as_str();
    let select_params: [&(dyn ToSql + Sync); 2] = [&project_id, &integration_type];
    let existing = conn
        .query_opt(
            "SELECT encrypted_api_key, encrypted_secret
             FROM integrations
             WHERE project_id = $1 AND integration_type = $2::text::integration_type",
            &select_params,
        )
        .await
        .map_err(AuthError::from)?;

    let (existing_api_key, existing_secret): (Option<Vec<u8>>, Option<Vec<u8>>) = match &existing {
        Some(row) => (row.get(0), row.get(1)),
        None => (None, None),
    };

    let credentials_changed = secret_changes(input.api_key.as_deref(), existing_api_key.as_deref())
        || secret_changes(input.secret.as_deref(), existing_secret.as_deref());
    let final_api_key = resolve_secret(input.api_key.as_deref(), existing_api_key);
    let final_secret = resolve_secret(input.secret.as_deref(), existing_secret);

    // Don't create an empty row for a never-configured integration.
    if existing.is_none() && final_api_key.is_none() && final_secret.is_none() {
        return Ok(());
    }

    let external_account_id = normalize_optional(input.external_account_id.as_deref());

    let insert_params: [&(dyn ToSql + Sync); 6] = [
        &project_id,
        &integration_type,
        &external_account_id,
        &final_api_key,
        &final_secret,
        &credentials_changed,
    ];
    // Pointing the integration at a different account invalidates the day-by-day
    // sync cursor, since the rows already stored describe the previous account.
    conn.execute(
        "INSERT INTO integrations
            (project_id, integration_type, external_account_id, encrypted_api_key, encrypted_secret)
         VALUES ($1, $2::text::integration_type, $3, $4, $5)
         ON CONFLICT (project_id, integration_type) DO UPDATE SET
            external_account_id = EXCLUDED.external_account_id,
            encrypted_api_key = EXCLUDED.encrypted_api_key,
            encrypted_secret = EXCLUDED.encrypted_secret,
            synced_through_date = CASE
                WHEN integrations.external_account_id
                     IS DISTINCT FROM EXCLUDED.external_account_id
                THEN NULL
                ELSE integrations.synced_through_date
            END,
            -- What makes a sync due, cleared so a new credential or account is tried on the
            -- next pass, and a fix can be confirmed. Not for the same credential again, or
            -- a retry loop on a bad key would hammer the provider past the throttle.
            last_sync_completed_at = CASE
                WHEN $6::bool
                     OR integrations.external_account_id
                        IS DISTINCT FROM EXCLUDED.external_account_id
                THEN NULL
                ELSE integrations.last_sync_completed_at
            END,
            last_full_sync_success_at = CASE
                WHEN $6::bool
                     OR integrations.external_account_id
                        IS DISTINCT FROM EXCLUDED.external_account_id
                THEN NULL
                ELSE integrations.last_full_sync_success_at
            END",
        &insert_params,
    )
    .await
    .map_err(AuthError::from)?;
    Ok(())
}

/// Whether saving `input` changes the stored credential. Compared as plaintext, since
/// the same key encrypts differently every time.
fn secret_changes(input: Option<&str>, existing: Option<&[u8]>) -> bool {
    match input {
        None => false,
        Some("") => existing.is_some(),
        Some(value) => existing.and_then(crypto::decrypt).as_deref() != Some(value.as_bytes()),
    }
}

/// Resolves the credential to persist from the request value and the stored
/// value: `None` keeps the existing bytes, an empty string clears it, and any
/// other string is freshly encrypted.
fn resolve_secret(input: Option<&str>, existing: Option<Vec<u8>>) -> Option<Vec<u8>> {
    match input {
        None => existing,
        Some("") => None,
        Some(value) => Some(crypto::encrypt(value.as_bytes())),
    }
}

/// The zone to write, or `None` to leave the stored one alone.
///
/// Validation resolves the name against the IANA database, so what reaches the
/// column is always a zone the bucketing paths can use. An unresolvable name is
/// dropped rather than failing the save: the rest of the form is still valid, and
/// keeping a zone that buckets correctly beats rejecting the whole request over
/// one field. The response echoes the stored zone, so the form snaps back.
fn timezone_to_save(requested: Option<&str>) -> Option<String> {
    requested
        .and_then(timezone::validate)
        .map(|timezone| timezone.name().to_string())
}

fn normalize_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn validate_payment_success_path(value: Option<&str>) -> Result<Option<String>, AuthError> {
    let Some(path) = normalize_optional(value) else {
        return Ok(None);
    };
    if path.len() > MAX_PAYMENT_SUCCESS_PATH_LEN || !path.starts_with('/') {
        return Err(AuthError::InvalidRequest("invalid payment success path"));
    }
    Ok(Some(path))
}

fn validate_custom_domain(value: Option<&str>) -> Result<Option<String>, AuthError> {
    let Some(raw) = normalize_optional(value) else {
        return Ok(None);
    };
    let host = hostname_from_input(&raw)?;
    if host.len() > MAX_CUSTOM_DOMAIN_LEN || !is_dns_hostname(&host) {
        return Err(AuthError::InvalidRequest("invalid custom domain"));
    }
    if crate::acme::is_revscope_hostname(&host) {
        return Err(AuthError::InvalidRequest(
            "custom domain cannot be a revscope.co hostname",
        ));
    }
    Ok(Some(host))
}

/// Accepts a bare hostname or a URL; SNI and ACME need the hostname only.
fn hostname_from_input(value: &str) -> Result<String, AuthError> {
    let host = if value.contains("://") || value.starts_with("//") {
        let parsed = if value.starts_with("//") {
            url::Url::parse(&format!("https:{value}"))
        } else {
            url::Url::parse(value)
        }
        .map_err(|_| AuthError::InvalidRequest("invalid custom domain"))?;
        if parsed.port().is_some() {
            return Err(AuthError::InvalidRequest("invalid custom domain"));
        }
        parsed
            .host_str()
            .ok_or(AuthError::InvalidRequest("invalid custom domain"))?
            .to_string()
    } else if value.contains('/')
        || value.contains('?')
        || value.contains('#')
        || value.contains(':')
    {
        return Err(AuthError::InvalidRequest("invalid custom domain"));
    } else {
        value.to_string()
    };
    Ok(host.trim_end_matches('.').to_ascii_lowercase())
}

fn is_dns_hostname(host: &str) -> bool {
    if host.parse::<std::net::Ipv4Addr>().is_ok() {
        return false;
    }
    let labels: Vec<&str> = host.split('.').collect();
    labels.len() >= 2
        && labels.iter().all(|label| {
            let len = label.len();
            (1..=63).contains(&len)
                && label.as_bytes()[0].is_ascii_alphanumeric()
                && label.as_bytes()[len - 1].is_ascii_alphanumeric()
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

fn map_custom_domain_error(error: tokio_postgres::Error) -> AuthError {
    if error.code() == Some(&SqlState::UNIQUE_VIOLATION)
        && error
            .as_db_error()
            .and_then(tokio_postgres::error::DbError::constraint)
            == Some("project_settings_custom_domain_key")
    {
        return AuthError::Conflict("This domain is already used by another project.");
    }
    AuthError::from(error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_bare_hostname_and_lowercases_it() {
        assert_eq!(
            validate_custom_domain(Some("Assets.Example.COM")).unwrap(),
            Some("assets.example.com".to_string())
        );
    }

    #[test]
    fn accepts_a_url_and_keeps_only_the_hostname() {
        assert_eq!(
            validate_custom_domain(Some("https://assets.example.com/tracker/")).unwrap(),
            Some("assets.example.com".to_string())
        );
    }

    #[test]
    fn clears_an_empty_custom_domain() {
        assert_eq!(validate_custom_domain(Some("")).unwrap(), None);
        assert_eq!(validate_custom_domain(None).unwrap(), None);
    }

    #[test]
    fn rejects_revscope_hostnames() {
        assert!(validate_custom_domain(Some("eu1.revscope.co")).is_err());
        assert!(validate_custom_domain(Some("api.revscope.co")).is_err());
        assert!(validate_custom_domain(Some("assets.revscope.co")).is_err());
    }

    #[test]
    fn rejects_invalid_custom_domains() {
        assert!(validate_custom_domain(Some("localhost")).is_err());
        assert!(validate_custom_domain(Some("assets.example.com:443")).is_err());
        assert!(validate_custom_domain(Some("-bad.example.com")).is_err());
        assert!(validate_custom_domain(Some("127.0.0.1")).is_err());
    }

    #[test]
    fn saves_a_resolvable_timezone_and_keeps_the_stored_one_otherwise() {
        assert_eq!(
            timezone_to_save(Some(" America/Chicago ")).as_deref(),
            Some("America/Chicago")
        );
        // Well-formed but not a real zone: the old length-only check let this
        // through, leaving a name nothing could bucket with.
        assert_eq!(timezone_to_save(Some("America/Chigago")), None);
        assert_eq!(timezone_to_save(Some("")), None);
        assert_eq!(timezone_to_save(Some("UTC+2")), None);
        assert_eq!(timezone_to_save(None), None);
    }
}
