//! When batches flush (on the timer, past a size threshold, and at shutdown),
//! and what happens to a batch whose write fails.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use compact_str::CompactString;
use fxhash::FxHashSet;
use log::warn;
use tokio_postgres::types::ToSql;

use crate::db::{
    DbError, open_main_database_write, open_project_database_write, try_open_project_database_write,
};
use crate::ingest::batch::BatchData;
use crate::ingest::error::PersistError;
use crate::ingest::failure::{IngestRowFailure, may_record_error, persist_row_failure_to_main};
use crate::ingest::map::{IngestBatch, IngestBatchGuard, IngestMap};
use crate::sentry_report::report_error;
use crate::server_context::ServerContext;
use crate::util::nanoseconds_since_epoch;

use super::{FLUSH_INTERVAL_SECS, OrphanEvents, flush_batch_into};

pub(super) const FLUSH_INTERVAL: Duration = Duration::from_secs(FLUSH_INTERVAL_SECS);
/// When every due batch is write-busy, wait this long before walking again.
const WRITE_BUSY_RETRY: Duration = Duration::from_millis(100);
/// How long a batch whose database another connection has locked is retried
/// before it is archived in `errors` and discarded. Unlike our own write slot,
/// nothing bounds how long an outside process holds that lock, the batch keeps
/// growing meanwhile, and shutdown waits on the drain.
pub(super) const LOCKED_GIVE_UP: Duration = Duration::from_secs(30);
/// Locks found further apart than this count as separate runs toward
/// [`LOCKED_GIVE_UP`]. Above the 5 second busy timeout an on-disk attempt can
/// spend waiting, plus the retry pause.
const LOCKED_MAX_GAP: Duration = Duration::from_secs(10);

/// How to check out the project SQLite write slot for a flush.
pub(super) enum WriteCheckout {
    /// Non-blocking. Used by the timer and the threshold flush so a held write
    /// (integration sync, etc.) skips this batch and retries later.
    Try,
    /// Wait up to the checkout timeout. Used by shutdown drain so queued rows
    /// are not abandoned while another writer finishes.
    Block,
}

/// Result of attempting to persist a batch.
pub(super) enum FlushOutcome {
    Flushed,
    /// Our write slot was held. Keep the batch and retry.
    WriteBusy,
    /// SQLite reported the database or a table locked by another connection.
    /// Nothing was written; keep the batch and retry.
    Locked(PersistError),
}

/// Result of one timer pass over due batches.
pub(super) enum TimerPass {
    /// Head is not due, or the map is empty.
    Idle,
    /// At least one batch was flushed or evicted.
    Progress,
    /// Due batches exist, but every one was write-busy.
    AllBusy,
}

/// A batch that could not be flushed, along with any rows that were dropped
/// before we gave up (they still have to be recorded).
#[derive(Debug)]
pub(in crate::ingest) struct FlushFailure {
    pub(in crate::ingest) error: PersistError,
    pub(in crate::ingest) dropped_rows: Vec<IngestRowFailure>,
}

/// Start the background task that flushes idle batches after [`FLUSH_INTERVAL`].
/// Idempotent: only the first call per server context spawns the task.
pub(crate) fn start_ingest_flush_scheduler(server_context: Arc<ServerContext>) {
    if server_context
        .flush_scheduler_started
        .swap(true, Ordering::SeqCst)
    {
        return;
    }
    crate::task::spawn(server_context.clone(), async move {
        ingest_flush_scheduler_loop(server_context).await;
    });
}

pub(super) async fn ingest_flush_scheduler_loop(server_context: Arc<ServerContext>) {
    while !server_context.shutdown_requested() {
        // Read the head's deadline, dropping the guard immediately.
        let next_deadline = server_context
            .ingest_map
            .least_recently_flushed()
            .map(|guard| guard.flushed_at() + FLUSH_INTERVAL);
        match next_deadline {
            None => {
                tokio::select! {
                    _ = server_context.ingest_map.wait_for_batch() => {}
                    _ = server_context.wait_for_shutdown() => break,
                }
            }
            Some(deadline) => {
                if deadline > tokio::time::Instant::now() {
                    tokio::select! {
                        _ = tokio::time::sleep_until(deadline) => {}
                        // A newly inserted batch may need an earlier wake; just
                        // recompute the head and deadline.
                        _ = server_context.ingest_map.wait_for_batch() => continue,
                        _ = server_context.wait_for_shutdown() => break,
                    }
                }
                match flush_due_batches(server_context.clone()).await {
                    TimerPass::AllBusy => {
                        tokio::select! {
                            _ = tokio::time::sleep(WRITE_BUSY_RETRY) => {}
                            _ = server_context.wait_for_shutdown() => break,
                        }
                    }
                    TimerPass::Idle | TimerPass::Progress => {}
                }
            }
        }
    }

    // Drain remaining batches on shutdown so queued rows are not lost.
    drain_flush_all(server_context).await;
}

/// Flush every due batch that can acquire a write connection. Write-busy
/// batches stay in the map (in place) and are retried on a later pass.
pub(super) async fn flush_due_batches(server_context: Arc<ServerContext>) -> TimerPass {
    let sc = server_context.clone();
    match crate::task::spawn_blocking(sc.clone(), move || flush_due_batches_sync(&sc)).await {
        Ok(pass) => pass,
        // A panic inside a batch flush is caught and the batch recorded, so
        // anything reaching here escaped the walk over the map itself. Ending
        // the scheduler would stop every project's flushes until the next
        // restart, so back off and walk again instead.
        Err(join_error) => {
            warn!("ingest timer flush task failed: {join_error}");
            TimerPass::AllBusy
        }
    }
}

pub(super) fn flush_due_batches_sync(server_context: &Arc<ServerContext>) -> TimerPass {
    let mut skipped = FxHashSet::default();
    let mut flushed_any = false;
    let mut saw_due = false;
    loop {
        let Some(mut guard) = first_due_not_skipped(&server_context.ingest_map, &skipped) else {
            break;
        };
        saw_due = true;
        if guard.is_empty() {
            server_context.ingest_map.pop(guard);
            flushed_any = true;
            continue;
        }
        let project_id = guard.project_id().to_string();
        match flush_project_batch_sync(
            server_context,
            &project_id,
            &mut guard,
            WriteCheckout::Try,
            OrphanEvents::Carry,
        ) {
            Ok(FlushOutcome::Flushed) => {
                // Carried orphan events need the batch to stay for their
                // pageview, and the next timer pass to drop them if it never comes.
                if guard.has_orphan_events() {
                    server_context.ingest_map.update_flushed_at(guard);
                } else {
                    server_context.ingest_map.pop(guard);
                }
                flushed_any = true;
            }
            Ok(FlushOutcome::WriteBusy | FlushOutcome::Locked(_)) => {
                skipped.insert(guard.project_id());
                drop(guard);
            }
            Err(error) => {
                report_error!(
                    &format!("ingest_flush.timer:{project_id}"),
                    "ingest timer flush failed for project {project_id}: {error:?}"
                );
                server_context.ingest_map.pop(guard);
                flushed_any = true;
            }
        }
    }
    if flushed_any {
        TimerPass::Progress
    } else if saw_due {
        TimerPass::AllBusy
    } else {
        TimerPass::Idle
    }
}

/// Oldest due batch that is not in `skipped`, or `None` if the remaining list
/// is not yet due.
pub(super) fn first_due_not_skipped(
    ingest_map: &IngestMap,
    skipped: &FxHashSet<i32>,
) -> Option<IngestBatchGuard> {
    let mut guard = ingest_map.least_recently_flushed()?;
    loop {
        if guard.flushed_at() + FLUSH_INTERVAL > tokio::time::Instant::now() {
            return None;
        }
        if !skipped.contains(&guard.project_id()) {
            return Some(guard);
        }
        let idx = guard.self_idx();
        drop(guard);
        guard = ingest_map.next_after(idx)?;
    }
}

/// Flush every buffered ingest batch. Safe to call while the server is still
/// accepting requests: enqueue happens synchronously before the HTTP response,
/// so "response received, then flush" is deterministic for tests.
pub async fn flush_all_ingest(server_context: Arc<ServerContext>) {
    drain_flush_all(server_context).await;
}

/// Flush every buffered ingest batch. Call after in-flight request handlers have
/// finished so concurrent enqueues cannot race the drain loop.
pub async fn drain_ingest_on_shutdown(server_context: Arc<ServerContext>) {
    flush_all_ingest(server_context.clone()).await;
    // `projects_last_ingest` rows are written on spawned tasks after each flush;
    // wait so shutdown does not tear down the runtime before those inserts finish.
    while server_context.active_task_count() > 0 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

pub(super) async fn drain_flush_all(server_context: Arc<ServerContext>) {
    let sc = server_context.clone();
    let drain = tokio::task::spawn_blocking(move || {
        drain_flush_all_sync(&sc);
    })
    .await;
    // This runs on the shutdown path, where losing whatever is still queued
    // beats aborting the rest of the shutdown.
    if let Err(join_error) = drain {
        warn!("ingest drain flush task failed: {join_error}");
    }
}

pub(super) fn drain_flush_all_sync(server_context: &Arc<ServerContext>) {
    while let Some(mut guard) = server_context.ingest_map.least_recently_flushed() {
        if guard.is_empty() {
            server_context.ingest_map.pop(guard);
            continue;
        }
        let project_id = guard.project_id().to_string();
        match flush_project_batch_sync(
            server_context,
            &project_id,
            &mut guard,
            WriteCheckout::Block,
            OrphanEvents::Drop,
        ) {
            Ok(FlushOutcome::Flushed) => {
                server_context.ingest_map.pop(guard);
            }
            Ok(FlushOutcome::WriteBusy | FlushOutcome::Locked(_)) => {
                // The write slot or the database was busy. Keep the batch and
                // retry; dropping the guard lets us wait again on the same
                // head. A lock that outlasts LOCKED_GIVE_UP comes back as Err.
                drop(guard);
                std::thread::sleep(WRITE_BUSY_RETRY);
            }
            Err(error) => {
                report_error!(
                    &format!("ingest_flush.drain:{project_id}"),
                    "ingest drain flush failed for project {project_id}: {error:?}"
                );
                server_context.ingest_map.pop(guard);
            }
        }
    }
}

/// Hand a batch that has crossed [`BATCH_FLUSH_THRESHOLD`](super::BATCH_FLUSH_THRESHOLD) to a background
/// flush, so the ingest request that crossed it does not wait on the write.
///
/// At most one flush is queued per batch generation (popping the batch resets
/// the flag): the flag is set under the batch lock here and cleared under it
/// when the task starts, so hits arriving while the
/// task waits for the lock just join the batch it is about to write. Once the
/// task has started, a later hit that still finds the batch over the threshold
/// (the write slot was busy) queues the next one.
pub(in crate::ingest) fn schedule_threshold_flush(
    server_context: &Arc<ServerContext>,
    mut guard: IngestBatchGuard,
) {
    if !guard.at_flush_threshold() || !guard.schedule_flush() {
        return;
    }
    let project_key = guard.project_id();
    // Release the batch before spawning so the task does not start by waiting
    // on a lock this thread still holds.
    drop(guard);
    let sc = server_context.clone();
    crate::task::spawn_blocking(server_context.clone(), move || {
        run_threshold_flush(&sc, project_key);
    });
}

/// The background half of [`schedule_threshold_flush`].
pub(super) fn run_threshold_flush(server_context: &Arc<ServerContext>, project_key: i32) {
    // The timer or the drain may have flushed and popped the batch meanwhile.
    let Some(mut guard) = server_context.ingest_map.get_existing(project_key) else {
        return;
    };
    // Not ours to run if the slot was popped and recreated for this project
    // since, or the timer already flushed the batch below the threshold.
    if !guard.take_scheduled_flush() || !guard.at_flush_threshold() {
        return;
    }
    flush_enqueue_batch(server_context, &project_key.to_string(), guard);
}

/// Flush a batch that crossed the threshold, keeping it in the map for the
/// next hits.
pub(super) fn flush_enqueue_batch(
    server_context: &Arc<ServerContext>,
    project_id: &str,
    mut guard: IngestBatchGuard,
) {
    if guard.is_empty() {
        return;
    }
    match flush_project_batch_sync(
        server_context,
        project_id,
        &mut guard,
        WriteCheckout::Try,
        OrphanEvents::Carry,
    ) {
        Ok(FlushOutcome::Flushed) => {
            server_context.ingest_map.update_flushed_at(guard);
        }
        Ok(FlushOutcome::WriteBusy | FlushOutcome::Locked(_)) => {
            // Keep the queued rows. The next ingest that leaves the batch at
            // or above the threshold queues another flush; the timer also retries.
        }
        Err(error) => {
            report_error!(
                &format!("ingest_flush.batch:{project_id}"),
                "ingest batch flush failed for project {project_id}: {error:?}"
            );
            server_context.ingest_map.update_flushed_at(guard);
        }
    }
}

/// Open a write connection and flush the batch. A batch the database has been
/// locked against for [`LOCKED_GIVE_UP`] is archived in `errors` and returned
/// as an error, so the caller discards it like any other failed batch.
pub(super) fn flush_project_batch_sync(
    server_context: &Arc<ServerContext>,
    project_id: &str,
    batch: &mut IngestBatch,
    checkout: WriteCheckout,
    orphan_events: OrphanEvents,
) -> Result<FlushOutcome, PersistError> {
    let outcome =
        flush_project_batch_once(server_context, project_id, batch, checkout, orphan_events)?;
    let FlushOutcome::Locked(error) = outcome else {
        if matches!(outcome, FlushOutcome::WriteBusy) {
            batch.clear_locked();
        }
        return Ok(outcome);
    };
    let (locked_for, new_run) = batch.mark_locked(LOCKED_MAX_GAP);
    if new_run {
        warn!("ingest flush for project {project_id} found the database locked, retrying: {error}");
    }
    if locked_for < LOCKED_GIVE_UP {
        return Ok(FlushOutcome::Locked(error));
    }
    let error = PersistError::DatabaseBusy(format!(
        "gave up after the database stayed locked for {}s: {error}",
        locked_for.as_secs()
    ));
    record_failed_batch(server_context, project_id, batch, &error);
    Err(error)
}

pub(super) fn flush_project_batch_once(
    server_context: &Arc<ServerContext>,
    project_id: &str,
    batch: &mut IngestBatch,
    checkout: WriteCheckout,
    orphan_events: OrphanEvents,
) -> Result<FlushOutcome, PersistError> {
    if batch.is_empty() {
        return Ok(FlushOutcome::Flushed);
    }
    let started_at = i64::try_from(nanoseconds_since_epoch())?;
    let expired_orphans = batch.take_expired_orphan_events(
        started_at,
        matches!(orphan_events, OrphanEvents::Carry),
        |client_ip| may_record_error(server_context, client_ip),
    );
    record_dropped_rows(server_context, project_id, expired_orphans);
    if !batch.has_rows() {
        return Ok(FlushOutcome::Flushed);
    }
    let project_key = batch.project_id();
    crate::db::require_project(server_context.clone(), project_key).map_err(|error| {
        report_error!(
            &format!("ingest_flush.project_cache:{project_id}"),
            "project cache lookup failed for {project_id}: {error}"
        );
        PersistError::from(error)
    })?;

    let conn = match checkout {
        WriteCheckout::Try => match try_open_project_database_write(server_context, project_id) {
            Ok(Some(conn)) => conn,
            Ok(None) => return Ok(FlushOutcome::WriteBusy),
            Err(error) => return open_failure_outcome(project_id, error),
        },
        WriteCheckout::Block => match open_project_database_write(server_context, project_id) {
            Ok(conn) => conn,
            Err(DbError::ProjectDbCheckoutTimeout) => return Ok(FlushOutcome::WriteBusy),
            Err(error) => return open_failure_outcome(project_id, error),
        },
    };

    let dropped_rows = {
        match flush_batch_into(&conn, batch) {
            Ok(dropped_rows) => dropped_rows,
            Err(failure) if failure.error.is_busy() => {
                drop(conn);
                record_dropped_rows(server_context, project_id, failure.dropped_rows);
                return Ok(FlushOutcome::Locked(failure.error));
            }
            Err(failure) => {
                record_dropped_rows(server_context, project_id, failure.dropped_rows);
                // Every caller discards the batch after a failure, so this is
                // the last chance to preserve the rows still queued on it.
                record_failed_batch(server_context, project_id, batch, &failure.error);
                return Err(failure.error);
            }
        }
    }; // release the locked project write connection before touching main postgres
    drop(conn);
    record_dropped_rows(server_context, project_id, dropped_rows);

    // Counted after the flush so dropped rows are not reported as inserted.
    let events_inserted =
        i32::try_from(batch.pageviews.len() + batch.events.len()).unwrap_or(i32::MAX);
    let finished_at = i64::try_from(nanoseconds_since_epoch()).unwrap_or(started_at);
    let server_context = server_context.clone();
    crate::task::spawn(server_context.clone(), async move {
        match insert_projects_last_ingest(
            server_context,
            project_key,
            started_at,
            finished_at,
            events_inserted,
        )
        .await
        {
            Ok(()) => {}
            // Tests often run without DATABASE_URL; skip quietly there.
            Err(crate::db::DbError::MissingMainPostgresUrlEnv(_)) => {}
            Err(error) => {
                warn!("failed to record projects_last_ingest for project {project_key}: {error}");
            }
        }
    });
    Ok(FlushOutcome::Flushed)
}

/// The flush outcome when the project database could not be opened for writing.
pub(super) fn open_failure_outcome(
    project_id: &str,
    error: DbError,
) -> Result<FlushOutcome, PersistError> {
    let error = PersistError::from(error);
    if error.is_busy() {
        return Ok(FlushOutcome::Locked(error));
    }
    report_error!(
        &format!("ingest_flush.open_db:{project_id}"),
        "open project database for flush failed for {project_id}: {error}"
    );
    Err(error)
}

/// Record rows the project database rejected in the main `errors` table, so the
/// dropped data can be inspected. Spawned because the flush runs on a blocking
/// thread while postgres work belongs on the runtime.
pub(super) fn record_dropped_rows(
    server_context: &Arc<ServerContext>,
    project_id: &str,
    dropped_rows: Vec<IngestRowFailure>,
) {
    if dropped_rows.is_empty() {
        return;
    }
    let project_id = project_id.to_string();
    let server_context = server_context.clone();
    crate::task::spawn(server_context.clone(), async move {
        for failure in &dropped_rows {
            if let Err(error) =
                persist_row_failure_to_main(server_context.clone(), &project_id, failure).await
            {
                warn!("failed to record dropped ingest row for project {project_id}: {error}");
            }
        }
    });
}

/// Record the rows a flush could not write in the main `errors` table, so the
/// batch can be recovered instead of vanishing when the caller discards it.
///
/// The batch is serialized here, on the flush thread, because the guard cannot
/// outlive this call; the postgres write is spawned, like the dropped-row
/// records above. Used for both ordinary flush errors and panics.
pub(super) fn record_failed_batch(
    server_context: &Arc<ServerContext>,
    project_id: &str,
    batch: &IngestBatch,
    error: &PersistError,
) {
    let project_key = batch.project_id();
    let data: &BatchData = batch;
    let data = serde_json::to_string(data).unwrap_or_else(|serialize_error| {
        report_error!(
            &format!("ingest_flush.archive_serialize:{project_key}"),
            "could not serialize the abandoned ingest batch for project {project_key}: {serialize_error}"
        );
        "null".to_string()
    });
    let failure = IngestRowFailure {
        error_type: CompactString::new(error.error_type()),
        error_text: error.to_string(),
        data,
    };
    let project_id = project_id.to_string();
    let server_context = server_context.clone();
    crate::task::spawn(server_context.clone(), async move {
        if let Err(error) = persist_row_failure_to_main(server_context, &project_id, &failure).await
        {
            warn!("failed to record the abandoned ingest batch for project {project_id}: {error}");
        }
    });
}

pub(super) async fn insert_projects_last_ingest(
    server_context: Arc<ServerContext>,
    project_id: i32,
    started_at: i64,
    finished_at: i64,
    events_inserted: i32,
) -> Result<(), crate::db::DbError> {
    let conn = open_main_database_write(server_context).await?;
    let params: [&(dyn ToSql + Sync); 4] =
        [&project_id, &started_at, &finished_at, &events_inserted];
    conn.execute(
        "INSERT INTO projects_last_ingest (project_id, started_at, finished_at, events_inserted)
         VALUES ($1, $2, $3, $4)",
        &params,
    )
    .await
    .map_err(crate::db::DbError::main_postgres_query)?;
    Ok(())
}
