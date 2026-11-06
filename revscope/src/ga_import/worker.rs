//! The background task that runs imports, one chunk at a time.

use std::sync::Arc;
use std::time::Duration;

use fxhash::FxHashMap;
use log::warn;

use crate::db::{open_main_database_write, open_project_database_write_for};
use crate::integration::{ensure_project_cached, sleep_or_shutdown};
use crate::sentry_report::report_error;
use crate::server_context::ServerContext;
use crate::timezone;
use crate::util::unix_seconds;

use super::api::{self, GaApiError};
use super::credential::GaCredential;
use super::jobs::{self, DueImport};
use super::reports::{chunk_at, chunk_count};
use super::store;

const IDLE_INTERVAL: Duration = Duration::from_secs(5);
const DUE_BATCH_SIZE: i64 = 20;
/// Renew a token this far ahead of expiry; a large report can take a while.
const TOKEN_REFRESH_MARGIN_SECS: i64 = 300;
/// A Google outage shouldn't lose an import, but one that never recovers has to stop.
const MAX_FAILED_ATTEMPTS: i32 = 12;
const MAX_RETRY_DELAY_SECS: i64 = 60 * 60;

pub fn start_ga_import_task(server_context: Arc<ServerContext>) {
    crate::task::spawn(server_context.clone(), async move {
        let mut tokens = TokenCache::default();
        while !server_context.shutdown_requested() {
            let did_work = match run_due_chunks(&server_context, &mut tokens).await {
                Ok(did_work) => did_work,
                Err(error) => {
                    report_error!(
                        "ga_import.worker",
                        "Google Analytics import failed: {error}"
                    );
                    false
                }
            };
            if !did_work && sleep_or_shutdown(&server_context, IDLE_INTERVAL).await {
                break;
            }
        }
    });
}

#[derive(Default)]
struct TokenCache {
    tokens: FxHashMap<i32, (String, i64)>,
}

impl TokenCache {
    async fn get(
        &mut self,
        server_context: &ServerContext,
        import_id: i32,
        credential: &GaCredential,
    ) -> Result<String, GaApiError> {
        if let Some((token, expires_at)) = self.tokens.get(&import_id)
            && unix_seconds() + TOKEN_REFRESH_MARGIN_SECS < *expires_at
        {
            return Ok(token.clone());
        }
        let fetched = api::fetch_access_token(server_context, credential).await?;
        self.tokens.insert(
            import_id,
            (
                fetched.access_token.clone(),
                unix_seconds() + fetched.expires_in,
            ),
        );
        Ok(fetched.access_token)
    }
}

async fn run_due_chunks(
    server_context: &Arc<ServerContext>,
    tokens: &mut TokenCache,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let conn = open_main_database_write(server_context.clone()).await?;
    for token in jobs::take_expired_connections(&conn).await? {
        // Out of storage already, so one failure mustn't cost the rest their revoke.
        if let Err(error) = jobs::revoke_stranded(server_context, &conn, token).await {
            warn!("couldn't revoke an expired Google Analytics token: {error}");
        }
    }
    let due = jobs::load_due_imports(&conn, DUE_BATCH_SIZE).await?;
    drop(conn);
    let did_work = !due.is_empty();
    tokens
        .tokens
        .retain(|id, _| due.iter().any(|import| import.id == *id));
    for import in due {
        if server_context.shutdown_requested() {
            break;
        }
        run_chunk(server_context, tokens, import).await?;
    }
    Ok(did_work)
}

enum ChunkFailure {
    Api(GaApiError),
    /// The project database wouldn't take the rows; try again like an outage.
    Storage(String),
}

async fn run_chunk(
    server_context: &Arc<ServerContext>,
    tokens: &mut TokenCache,
    import: DueImport,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let outcome = fetch_and_write_chunk(server_context, tokens, &import).await;
    let conn = open_main_database_write(server_context.clone()).await?;
    match outcome {
        Ok(ChunkResult::Written) => {
            jobs::advance(&conn, import.id, import.chunks_done).await?;
            if import.chunks_done + 1 >= chunk_count(import.start, import.end) {
                stop(server_context, &import, "completed", None).await?;
            }
        }
        // Postgres still lists it, so its rows were lost some other way, like
        // restoring the project's database. Left active it would run forever.
        Ok(ChunkResult::ImportDeleted) => {
            let error = "This import's data is no longer in the project's database. Delete it and start it again.";
            stop(server_context, &import, "failed", Some(error)).await?;
        }
        Ok(ChunkResult::NothingLeft) => {
            stop(server_context, &import, "completed", None).await?;
        }
        Err(ChunkFailure::Api(GaApiError::ClientSettings(message))) => {
            report_error!(
                "ga_import.client_settings",
                "Google Analytics imports are waiting on the OAuth client settings: {message}"
            );
            let status = "Waiting for this server's Google sign-in settings to be fixed; the import will continue by itself.";
            jobs::wait_for_settings(&conn, import.id, MAX_RETRY_DELAY_SECS, status).await?;
        }
        Err(ChunkFailure::Api(GaApiError::Quota)) => {
            jobs::pause_until_next_hour(&conn, import.id).await?;
        }
        Err(ChunkFailure::Api(GaApiError::Denied(message))) => {
            warn!(
                "Google Analytics import {} was refused: {message}",
                import.id
            );
            let error = "Google Analytics refused access. Check the credential still has access to this property, then delete this import and start it again.";
            stop(server_context, &import, "failed", Some(error)).await?;
        }
        Err(ChunkFailure::Api(GaApiError::Rejected(message))) => {
            report_error!(
                "ga_import.rejected",
                "Google Analytics rejected import {}: {message}",
                import.id
            );
            let error = "Google Analytics rejected the request for this report. Delete this import and try again, or contact support if it keeps happening.";
            stop(server_context, &import, "failed", Some(error)).await?;
        }
        Err(ChunkFailure::Api(
            GaApiError::Transient(message) | GaApiError::TokenRejected(message),
        ))
        | Err(ChunkFailure::Storage(message)) => {
            warn!(
                "Google Analytics import {} will retry: {message}",
                import.id
            );
            // The next attempt gets a fresh token, which also tells an expired token
            // from a revoked grant: the token endpoint refuses the latter.
            tokens.tokens.remove(&import.id);
            if import.failed_attempts + 1 >= MAX_FAILED_ATTEMPTS {
                let error = "This import kept failing for hours, so it stopped. Delete it and start it again.";
                stop(server_context, &import, "failed", Some(error)).await?;
            } else {
                let delay = retry_delay_secs(import.failed_attempts);
                let status = "Hit a temporary problem; retrying automatically.";
                jobs::retry_later(&conn, import.id, delay, status).await?;
            }
        }
    }
    Ok(())
}

fn retry_delay_secs(failed_attempts: i32) -> i64 {
    30i64
        .saturating_mul(1i64 << failed_attempts.clamp(0, 20))
        .min(MAX_RETRY_DELAY_SECS)
}

async fn stop(
    server_context: &Arc<ServerContext>,
    import: &DueImport,
    status: &str,
    error_text: Option<&str>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let conn = open_main_database_write(server_context.clone()).await?;
    let stopped = jobs::finish(&conn, import.id, import.project_id, status, error_text).await?;
    if let Some(Some(token)) = stopped {
        jobs::revoke_stranded(server_context, &conn, token).await?;
    }
    Ok(())
}

enum ChunkResult {
    Written,
    ImportDeleted,
    NothingLeft,
}

async fn fetch_and_write_chunk(
    server_context: &Arc<ServerContext>,
    tokens: &mut TokenCache,
    import: &DueImport,
) -> Result<ChunkResult, ChunkFailure> {
    let Some(((from, to), kind)) = chunk_at(import.start, import.end, import.chunks_done) else {
        return Ok(ChunkResult::NothingLeft);
    };
    let Some(credential) = import.credential.as_ref() else {
        return Err(ChunkFailure::Api(GaApiError::Denied(
            "stored credential can't be decrypted".to_string(),
        )));
    };
    let token = tokens
        .get(server_context, import.id, credential)
        .await
        .map_err(ChunkFailure::Api)?;
    let request = kind.request(from, to, &import.hostnames);
    let rows = api::run_report(server_context, &token, &import.property_id, &request)
        .await
        .map_err(ChunkFailure::Api)?;

    let project_id = import.project_id.to_string();
    let project = ensure_project_cached(server_context.clone(), &project_id)
        .await
        .map_err(|error| ChunkFailure::Storage(error.to_string()))?;
    let context = server_context.clone();
    let import_id = import.id;
    let project_key = import.project_id;
    let written = crate::task::spawn_blocking(server_context.clone(), move || {
        let timezone = timezone::project_timezone(context.clone(), project_key);
        let conn = open_project_database_write_for(&context, &project)
            .map_err(|error| error.to_string())?;
        store::write_chunk(
            &conn,
            import_id,
            kind,
            &from.to_string(),
            &to.to_string(),
            &timezone,
            &rows,
        )
        .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| ChunkFailure::Storage(error.to_string()))?
    .map_err(ChunkFailure::Storage)?;
    Ok(if written {
        ChunkResult::Written
    } else {
        ChunkResult::ImportDeleted
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_back_off_up_to_an_hour() {
        assert_eq!(retry_delay_secs(0), 30);
        assert_eq!(retry_delay_secs(3), 240);
        assert_eq!(retry_delay_secs(MAX_FAILED_ATTEMPTS), MAX_RETRY_DELAY_SECS);
    }
}
