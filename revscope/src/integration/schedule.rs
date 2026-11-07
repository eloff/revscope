//! When each integration is due to sync, and the retry loop every sync runs in.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use log::{info, warn};

use super::sync::{
    SyncCounts, SyncError, SyncKind, SyncResult, complete_integration_sync,
    create_integration_sync_row, credentials_are_current, sleep_or_shutdown,
};
use crate::crypto;
use crate::db::open_main_database_read;
use crate::sentry_report::report_error;
use crate::server_context::ServerContext;
use crate::util::unix_seconds;

/// Most integrations of one type started per scheduler pass.
pub(super) const SYNC_BATCH_SIZE: i64 = 20;
/// Least time between one sync of an integration finishing and the next starting.
pub(super) const SYNC_THROTTLE_SECS: i64 = 10 * 60;
/// How often a payment integration gets a full sync instead of an incremental one.
pub(super) const PAYMENT_FULL_SYNC_INTERVAL_SECS: i64 = 24 * 60 * 60;
/// How far before the last successful sync's start an incremental sync reads from.
const PAYMENT_INCREMENTAL_LOOKBACK_SECS: i64 = 4 * 60 * 60;
/// How far back a full sync of a payment integration reads.
const PAYMENT_FULL_LOOKBACK_SECS: i64 = 30 * 24 * 60 * 60;
const SYNC_MAX_ATTEMPTS: usize = 3;
const SYNC_RETRY_BACKOFF: Duration = Duration::from_secs(5);

/// A full sync is due when we've never completed one, or the last one started
/// at least `full_sync_interval_secs` ago; otherwise an incremental sync is
/// sufficient.
pub(super) fn sync_kind(
    last_full_success_started_at: Option<i64>,
    full_sync_interval_secs: i64,
) -> SyncKind {
    let full_sync_due = last_full_success_started_at
        .is_none_or(|started_at| unix_seconds() - started_at >= full_sync_interval_secs);
    if full_sync_due {
        SyncKind::Full
    } else {
        SyncKind::Incremental
    }
}

/// The earliest change a payment integration's sync reads: shortly before the
/// last successful sync started, or for a full sync a fixed window back.
pub(super) fn payment_sync_since(sync_kind: SyncKind, last_success_started_at: Option<i64>) -> i64 {
    match sync_kind {
        SyncKind::Incremental => last_success_started_at
            .unwrap_or(0)
            .saturating_sub(PAYMENT_INCREMENTAL_LOOKBACK_SECS),
        SyncKind::Full => unix_seconds().saturating_sub(PAYMENT_FULL_LOOKBACK_SECS),
    }
}

/// Decrypts a stored credential. `missing` and `invalid` are the errors the
/// provider reports for an absent or undecryptable one.
pub(super) fn decrypt_secret(
    encrypted: Option<&[u8]>,
    missing: SyncError,
    invalid: impl Fn() -> SyncError,
) -> SyncResult<String> {
    let encrypted = encrypted.ok_or(missing)?;
    let decrypted = crypto::decrypt(encrypted).ok_or_else(&invalid)?;
    String::from_utf8(decrypted).map_err(|_| invalid())
}

/// A payment provider integration authenticated by one stored secret.
pub(crate) struct PaymentIntegration {
    pub(super) id: i32,
    pub(super) project_id: String,
    pub(super) encrypted_secret: Option<Vec<u8>>,
    pub(super) credentials_version: i64,
    pub(super) last_success_started_at: Option<i64>,
    pub(super) sync_kind: SyncKind,
}

impl PaymentIntegration {
    pub(super) fn since(&self) -> i64 {
        payment_sync_since(self.sync_kind, self.last_success_started_at)
    }

    /// The external account id the provider's rows are stored under.
    pub(super) fn external_account_id(&self, provider: &str) -> String {
        format!("{provider}_integration_{}", self.id)
    }
}

/// SQL selecting `columns` from the `integration_type` integrations due for a
/// sync, as [`load_due_payment_integrations`] describes. `extra_condition`
/// is ANDed into the filter for providers that need more stored state.
pub(super) fn due_payment_integrations_sql(
    columns: &str,
    integration_type: &'static str,
    extra_condition: &str,
) -> String {
    format!(
        "SELECT {columns}
         FROM integrations
         WHERE disabled_at IS NULL
            AND integration_type = '{integration_type}'::integration_type
            AND encrypted_api_key IS NOT NULL{extra_condition}
            AND (last_sync_completed_at IS NULL
                 OR last_sync_completed_at < NOW() - INTERVAL '{SYNC_THROTTLE_SECS} seconds')
         ORDER BY
            (last_full_sync_success_at IS NULL
                OR last_full_sync_success_at
                    < NOW() - INTERVAL '{PAYMENT_FULL_SYNC_INTERVAL_SECS} seconds') DESC,
            last_sync_completed_at ASC NULLS FIRST
         LIMIT {SYNC_BATCH_SIZE}"
    )
}

/// Loads `integration_type` integrations that are due for a sync: not
/// disabled, and not synced (successfully or not) within the throttle window,
/// those overdue a full sync first. `last_sync_*` columns on `integrations` are
/// denormalized scheduler state maintained by [`complete_integration_sync`],
/// so this is a plain indexed scan with no joins into `integration_syncs`.
pub(super) async fn load_due_payment_integrations(
    server_context: Arc<ServerContext>,
    integration_type: &'static str,
) -> SyncResult<Vec<PaymentIntegration>> {
    let conn = open_main_database_read(server_context).await?;
    let rows = conn
        .query(
            &due_payment_integrations_sql(
                "id,
                 project_id::text AS project_id,
                 encrypted_api_key,
                 EXTRACT(EPOCH FROM last_sync_success_at)::BIGINT,
                 EXTRACT(EPOCH FROM last_full_sync_success_at)::BIGINT,
                 credentials_version",
                integration_type,
                "",
            ),
            &[],
        )
        .await?;

    Ok(rows
        .iter()
        .map(|row| PaymentIntegration {
            id: row.get(0),
            project_id: row.get(1),
            encrypted_secret: row.get(2),
            credentials_version: row.get(5),
            last_success_started_at: row.get(3),
            sync_kind: sync_kind(row.get(4), PAYMENT_FULL_SYNC_INTERVAL_SECS),
        })
        .collect())
}

/// What a successful sync did, and whether it ended up full or incremental.
pub(super) struct SyncOutcome {
    pub(super) counts: SyncCounts,
    pub(super) sync_kind: SyncKind,
}

/// Records a sync of integration `integration_id` in `integration_syncs`, and
/// runs `attempt` until it succeeds, up to [`SYNC_MAX_ATTEMPTS`] times or until
/// shutdown or a credential change, which skips the sync if it comes first.
/// `sync_kind` is recorded when the sync starts and if it fails; a
/// success records the kind its outcome reports. `provider` labels log lines.
/// `credentials_version` must be read together with the credentials `attempt`
/// uses, since a batch can wait minutes for its turn.
pub(super) async fn run_sync_with_retries<F, Fut>(
    server_context: Arc<ServerContext>,
    provider: &str,
    integration_id: i32,
    credentials_version: i64,
    sync_kind: SyncKind,
    mut attempt: F,
) where
    F: FnMut() -> Fut,
    Fut: Future<Output = SyncResult<SyncOutcome>>,
{
    let started_at = match create_integration_sync_row(
        server_context.clone(),
        integration_id,
        credentials_version,
        sync_kind,
    )
    .await
    {
        Ok(Some(started_at)) => started_at,
        Ok(None) => {
            info!("skipping {provider} integration {integration_id} sync: its credentials changed");
            return;
        }
        Err(error) => {
            report_error!(
                &format!("integration_sync.create_row:{integration_id}"),
                "failed to create integration_syncs row for {provider} integration {integration_id}: {error}"
            );
            return;
        }
    };

    let mut last_error = None;
    for attempt_number in 1..=SYNC_MAX_ATTEMPTS {
        match attempt().await {
            Ok(outcome) => {
                if let Err(error) = complete_integration_sync(
                    server_context,
                    integration_id,
                    credentials_version,
                    outcome.sync_kind,
                    started_at,
                    outcome.counts,
                    None,
                )
                .await
                {
                    report_error!(
                        &format!("integration_sync.complete_success:{integration_id}"),
                        "failed to mark {provider} integration {integration_id} sync successful: {error}"
                    );
                }
                return;
            }
            Err(error) => {
                warn!(
                    "{provider} integration {integration_id} sync attempt {attempt_number}/{SYNC_MAX_ATTEMPTS} failed: {error}"
                );
                last_error = Some(error.to_string());
                if attempt_number == SYNC_MAX_ATTEMPTS
                    || sleep_or_shutdown(&server_context, SYNC_RETRY_BACKOFF).await
                    || !credentials_are_current(
                        server_context.clone(),
                        integration_id,
                        credentials_version,
                    )
                    .await
                {
                    break;
                }
            }
        }
    }

    if let Err(error) = complete_integration_sync(
        server_context,
        integration_id,
        credentials_version,
        sync_kind,
        started_at,
        SyncCounts::default(),
        last_error.as_deref(),
    )
    .await
    {
        report_error!(
            &format!("integration_sync.complete_failed:{integration_id}"),
            "failed to mark {provider} integration {integration_id} sync failed: {error}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_failing_sync_stops_retrying_once_the_credentials_change() {
        let (server_context, conn) = crate::test_utils::main_test_db("sync-stop-retry").await;
        let account_id = crate::test_utils::insert_test_account(&conn, "sync_stop_retry").await;
        let project_id: i32 = conn
            .query_one(
                "INSERT INTO projects (account_id, name) VALUES ($1::text::uuid, $2) RETURNING id",
                &[&account_id, &format!("p{}", account_id.replace('-', ""))],
            )
            .await
            .expect("insert project")
            .get(0);
        let integration_id: i32 = conn
            .query_one(
                "INSERT INTO integrations (project_id, integration_type)
                 VALUES ($1, 'stripe') RETURNING id",
                &[&project_id],
            )
            .await
            .expect("insert integration")
            .get(0);
        let attempts = std::cell::Cell::new(0);

        run_sync_with_retries(
            server_context.clone(),
            "stripe",
            integration_id,
            0,
            SyncKind::Full,
            || {
                attempts.set(attempts.get() + 1);
                let conn = &conn;
                async move {
                    conn.execute(
                        "UPDATE integrations SET credentials_version = 1 WHERE id = $1",
                        &[&integration_id],
                    )
                    .await
                    .expect("change credentials");
                    Err(SyncError::CredentialsChanged)
                }
            },
        )
        .await;

        assert_eq!(attempts.get(), 1);
        conn.execute(
            "DELETE FROM accounts WHERE id = $1::text::uuid",
            &[&account_id],
        )
        .await
        .expect("delete account");
    }

    #[test]
    fn a_full_sync_runs_first_and_then_once_per_interval() {
        let now = unix_seconds();
        let interval = PAYMENT_FULL_SYNC_INTERVAL_SECS;
        assert_eq!(sync_kind(None, interval), SyncKind::Full);
        assert_eq!(sync_kind(Some(now - interval), interval), SyncKind::Full);
        assert_eq!(sync_kind(Some(now - 60), interval), SyncKind::Incremental);
    }
}
