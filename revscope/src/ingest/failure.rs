use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use compact_str::CompactString;
use log::{error, warn};
use tokio_postgres::types::ToSql;

use super::error::PersistError;
use super::parse::ValidatedPayload;
use super::wire::TrackRequest;
use crate::auth::AuthError;
use crate::db::open_main_database_write;
use crate::rate_limit::ip_key;
use crate::server_context::ServerContext;
use crate::util::{truncate_chars, uuid_v7_age_cutoff};

/// Room for a message plus a typical stack trace.
const MAX_ERROR_TEXT_CHARS: usize = 4096;
const MAX_ERROR_PATH_CHARS: usize = 1024;

/// Rows in `errors` older than this are deleted by the cleanup task.
const ERROR_RETENTION: Duration = Duration::from_secs(60 * 24 * 60 * 60);

/// An ingest row that could not be written, recorded in the main `errors` table
/// so the dropped data is visible instead of silently lost. Raised either at
/// enqueue time (a row that would violate a project-DB CHECK constraint) or at
/// flush time (a row the project database rejected).
#[derive(Debug)]
pub(super) struct IngestRowFailure {
    pub(super) error_type: CompactString,
    pub(super) error_text: String,
    pub(super) data: String,
}

pub(super) fn constraint_failure(
    wire: &TrackRequest,
    error_text: impl Into<String>,
) -> IngestRowFailure {
    let error_text = error_text.into();
    warn!("dropping ingest hit due to check constraint: {error_text}");
    IngestRowFailure {
        error_type: CompactString::new("check_constraint"),
        error_text,
        data: wire_json(wire),
    }
}

pub(super) fn wire_json(wire: &TrackRequest) -> String {
    serde_json::to_string(wire).unwrap_or_else(|_| "{}".to_string())
}

pub(super) fn unknown_page_failure(data: String) -> IngestRowFailure {
    let error_text = "event has no resolvable pageview (missing or unknown pg)";
    error!("dropping ingest event: {error_text}");
    IngestRowFailure {
        error_type: CompactString::new("unknown_page"),
        error_text: error_text.to_string(),
        data,
    }
}

/// Takes a token from the per-IP limit on `errors` rows a client can cause:
/// its JS error reports and the ingest rows it sends that we have to drop. A
/// request with no known address (only in tests that bypass the listener)
/// skips the limit.
pub(super) fn may_record_error(server_context: &ServerContext, client_ip: Option<IpAddr>) -> bool {
    client_ip.is_none_or(|ip| {
        server_context
            .public_write_rate_limits
            .error_report_ip
            .check(&ip_key(ip))
            .is_ok()
    })
}

/// Records a tracker JS error. Repeats of the same text, for the same project
/// and UTC day, increment one row's `count`.
pub(super) async fn persist_error_to_main(
    server_context: Arc<ServerContext>,
    project_id: &str,
    validated: &ValidatedPayload,
) -> Result<(), PersistError> {
    let error_text = validated
        .wire
        .error
        .as_deref()
        .map(|text| truncate_chars(text, MAX_ERROR_TEXT_CHARS));
    let path = validated
        .wire
        .path
        .as_deref()
        .map(|path| truncate_chars(path, MAX_ERROR_PATH_CHARS));

    let conn = open_main_database_write(server_context).await?;
    let project_key = crate::db::parse_project_id(project_id)?;
    let params: &[&(dyn ToSql + Sync)] = &[&project_key, &error_text, &path];
    // The conflict target is the partial unique index from migration 21. Path
    // is left out of the key, so one bug on many pages is still one row a day.
    conn.execute(
        "INSERT INTO errors (project_id, error_type, error_text, path, error_day, fingerprint)
         VALUES (
             $1, 'javascript', $2, $3, (NOW() AT TIME ZONE 'UTC')::date,
             md5(coalesce($2, ''))
         )
         ON CONFLICT (project_id, error_day, fingerprint) WHERE error_day IS NOT NULL
         DO UPDATE SET count = errors.count + 1",
        params,
    )
    .await?;
    Ok(())
}

/// Deletes `errors` rows first recorded more than [`ERROR_RETENTION`] ago. A
/// deduplicated row only counts repeats from the day it was created, so its id
/// dates the whole row.
pub(crate) async fn delete_expired_errors(
    server_context: Arc<ServerContext>,
) -> Result<u64, AuthError> {
    let conn = open_main_database_write(server_context).await?;
    let cutoff = uuid_v7_age_cutoff(ERROR_RETENTION.as_millis() as u64);
    Ok(conn
        .execute("DELETE FROM errors WHERE id < $1::text::uuid", &[&cutoff])
        .await?)
}

pub(super) async fn persist_row_failure_to_main(
    server_context: Arc<ServerContext>,
    project_id: &str,
    failure: &IngestRowFailure,
) -> Result<(), PersistError> {
    let conn = match open_main_database_write(server_context).await {
        Ok(conn) => conn,
        // Tests often run without DATABASE_URL; skip quietly there.
        Err(crate::db::DbError::MissingMainPostgresUrlEnv(_)) => return Ok(()),
        Err(error) => return Err(PersistError::from(error)),
    };
    let project_key = crate::db::parse_project_id(project_id)?;
    let is_server_error = true;
    let params: &[&(dyn ToSql + Sync)] = &[
        &project_key,
        &failure.error_type.as_str(),
        &is_server_error,
        &failure.error_text,
        &failure.data,
    ];
    conn.execute(
        "INSERT INTO errors (project_id, error_type, is_server_error, error_text, data)
         VALUES ($1, $2, $3, $4, $5)",
        params,
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::test_support::sample_payload;
    use serde_json::Value;

    #[test]
    fn constraint_failure_serializes_wire_event_json() {
        let wire = sample_payload();
        let failure = constraint_failure(&wire, "landings.os_version must be NULL or > 0");
        assert_eq!(failure.error_type, "check_constraint");
        let parsed: Value = serde_json::from_str(&failure.data).expect("json data");
        assert_eq!(parsed["p"], "/pricing");
        assert_eq!(parsed["h"], "example.com");
    }
}
