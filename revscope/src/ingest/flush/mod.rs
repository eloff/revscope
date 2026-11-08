//! Writing queued ingest batches into project databases: one transaction per
//! batch that resolves lookup ids, inserts pageviews, landings and events, and
//! merges sessions a visitor's identity ties together.

mod forget;
mod lookups;
mod pageviews;
mod scheduler;
mod sessions;

use std::panic::AssertUnwindSafe;

use compact_str::CompactString;
use fxhash::FxHashMap;
use log::warn;
use rusqlite::params;
use serde_json::Value;

use crate::crawlers::{upsert_crawler_path, upsert_tracker_crawler_view};
use crate::db::ProjectConnection;
use crate::sentry_report::report_error;
use crate::untrusted_keys::UntrustedKeyMap;
use crate::util::nanoseconds_since_epoch;

use super::batch::{BatchData, BatchRow};
use super::error::PersistError;
use super::failure::IngestRowFailure;
use super::map::IngestBatch;

use forget::apply_forget_requests;
use lookups::{
    resolve_cached_event_name_id, resolve_cached_referrer_id, resolve_cached_useragent_id,
};
pub(crate) use lookups::{
    resolve_event_name_id, resolve_landing_campaign_id, resolve_path_id, resolve_referrer_id,
};
use pageviews::{
    EngagementAccum, apply_engagement_update, apply_landing_context_update, flush_pageview,
    increment_landing_session_metrics, insert_landing_row, max_option,
};
use scheduler::FlushFailure;
pub(crate) use scheduler::start_ingest_flush_scheduler;

pub(in crate::ingest) use scheduler::schedule_threshold_flush;
pub use scheduler::{drain_ingest_on_shutdown, flush_all_ingest};
use sessions::{
    MetadataWrite, apply_session_merges, purge_stale_client_page_ids_map,
    purge_stale_user_sessions, upsert_metadata_rows,
};

pub(in crate::ingest) const BATCH_FLUSH_THRESHOLD: usize = 1000;
/// How long a batch can sit before the timer flush writes it.
pub(crate) const FLUSH_INTERVAL_SECS: u64 = 5 * 60;

/// A flush failure, with the batch row to blame when the failure can be pinned
/// on a single row.
pub(super) struct FlushError {
    row: Option<BatchRow>,
    error: PersistError,
}

impl From<PersistError> for FlushError {
    fn from(error: PersistError) -> Self {
        Self { row: None, error }
    }
}

impl From<rusqlite::Error> for FlushError {
    fn from(error: rusqlite::Error) -> Self {
        Self::from(PersistError::from(error))
    }
}

impl From<std::num::TryFromIntError> for FlushError {
    fn from(error: std::num::TryFromIntError) -> Self {
        Self::from(PersistError::from(error))
    }
}

/// Blame `row` for a flush failure so the retry can drop just that row.
pub(super) fn blame<E: Into<PersistError>>(row: BatchRow) -> impl Fn(E) -> FlushError {
    move |error| FlushError {
        row: Some(row),
        error: error.into(),
    }
}

/// What a flush does with events still waiting for their pageview.
pub(super) enum OrphanEvents {
    /// Keep the ones still within their grace window for the next flush.
    Carry,
    /// Drop all of them. Used by the drain, which has no next flush.
    Drop,
}

/// Flush a batch's queued rows into the project database, in a single write
/// transaction per attempt. A row the database rejects is dropped and returned
/// so the caller can record it, and the remaining rows are flushed again in a
/// fresh transaction. A [`PersistError::DatabaseBusy`] failure leaves the
/// remaining rows queued, since another connection holding a lock says nothing
/// about the row being written. Otherwise the caller clears the batch
/// afterwards (via `update_flushed_at` or `pop`).
pub(in crate::ingest) fn flush_batch_into(
    conn: &ProjectConnection,
    batch: &mut IngestBatch,
) -> Result<Vec<IngestRowFailure>, FlushFailure> {
    let mut dropped_rows = Vec::new();
    let mut retries = 0;
    while batch.has_rows() {
        let failure = match flush_batch_transaction_catching_panic(conn, batch) {
            Ok(()) => return Ok(dropped_rows),
            Err(failure) => failure,
        };
        // Without a row to blame the failure is not one poisoned row, and
        // retrying the same rows would not make progress. Retries are capped at
        // the batch size for the same reason: past that the batch itself, not
        // any single row, is the problem. Dropping a row for a lock would also
        // fail every later row that references it.
        let blamed_row = failure
            .row
            .filter(|_| retries < BATCH_FLUSH_THRESHOLD && !failure.error.is_busy());
        let Some(row) = blamed_row else {
            return Err(FlushFailure {
                error: failure.error,
                dropped_rows,
            });
        };
        retries += 1;
        dropped_rows.push(dropped_row_failure(batch, row, &failure.error));
        batch.remove_row(row);
    }
    Ok(dropped_rows)
}

/// Report a row that could not be written: an `errors` row carrying the row
/// itself, plus an error-level log line with both the database error and the row.
fn dropped_row_failure(
    batch: &IngestBatch,
    row: BatchRow,
    error: &PersistError,
) -> IngestRowFailure {
    let data = batch.row_json(row);
    let error_text = error.to_string();
    report_error!(
        "ingest_flush.dropped_row",
        "ingest flush dropping row it could not write: {error_text}; row: {data}"
    );
    IngestRowFailure {
        error_type: CompactString::new(error.error_type()),
        error_text,
        data,
    }
}

/// Run one flush attempt, turning a panic into a flush failure rather than
/// letting it unwind into the flush scheduler, which would stop every project's
/// flushes for the life of the process.
///
/// Blamed on no row, so the caller gives up on the batch (and records it)
/// instead of retrying a write that panics. parking_lot mutexes do not poison,
/// so the guarded batch is still readable for that record.
fn flush_batch_transaction_catching_panic(
    conn: &ProjectConnection,
    batch: &IngestBatch,
) -> Result<(), FlushError> {
    let attempt = AssertUnwindSafe(|| flush_batch_transaction(conn, batch));
    match std::panic::catch_unwind(attempt) {
        Ok(result) => result,
        Err(payload) => {
            // The unwind skipped the ROLLBACK below, so without this the
            // connection would go back to the pool still in a transaction.
            if !conn.is_autocommit() {
                let _ = conn.execute_batch("ROLLBACK");
            }
            // Sentry's panic integration has already reported this, so warn!
            // keeps the local log complete without raising a duplicate event.
            let message = format!("ingest flush panicked: {}", panic_message(&*payload));
            warn!("{message}");
            Err(FlushError {
                row: None,
                error: PersistError::Panic(message),
            })
        }
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> &str {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        message
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message
    } else {
        "unknown panic payload"
    }
}

/// Run one flush attempt for the whole batch in its own write transaction.
fn flush_batch_transaction(
    conn: &ProjectConnection,
    batch: &IngestBatch,
) -> Result<(), FlushError> {
    conn.execute_batch("BEGIN IMMEDIATE").map_err(|error| {
        let error = FlushError::from(error);
        if !error.error.is_busy() {
            report_error!(
                "ingest_flush.begin",
                "BEGIN IMMEDIATE failed during ingest flush: {}",
                error.error
            );
        }
        error
    })?;
    match flush_batch_body(conn, batch) {
        Ok(()) => conn.execute_batch("COMMIT").map_err(|error| {
            let error = FlushError::from(error);
            if !error.error.is_busy() {
                report_error!(
                    "ingest_flush.commit",
                    "COMMIT failed during ingest flush: {}",
                    error.error
                );
            }
            // A failed COMMIT can leave the transaction open, and the
            // connection goes back to the pool afterwards.
            if !conn.is_autocommit() {
                let _ = conn.execute_batch("ROLLBACK");
            }
            error
        }),
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

fn flush_batch_body(conn: &ProjectConnection, data: &BatchData) -> Result<(), FlushError> {
    // Aggregate engagements by their target pageview so we can fold them into
    // the pageview insert (same batch) or apply them as one update (already in
    // the database).
    let mut engagement_by_page: FxHashMap<i64, EngagementAccum> = FxHashMap::default();
    for engagement in &data.engagements {
        let entry = engagement_by_page
            .entry(engagement.page_timestamp_id)
            .or_insert(EngagementAccum {
                duration: 0,
                scrolled: None,
            });
        entry.duration += engagement.duration_increment;
        entry.scrolled = max_option(entry.scrolled, engagement.scrolled_percent);
    }

    // Memoized id resolution for repeated paths/referrers/events/useragents in the batch.
    let mut path_ids: UntrustedKeyMap<(CompactString, CompactString), i64> =
        UntrustedKeyMap::default();
    let mut referrer_ids: UntrustedKeyMap<CompactString, i64> = UntrustedKeyMap::default();
    let mut event_name_ids: UntrustedKeyMap<CompactString, i64> = UntrustedKeyMap::default();
    let mut useragent_ids: UntrustedKeyMap<CompactString, i64> = UntrustedKeyMap::default();
    let mut crawler_path_ids: UntrustedKeyMap<(CompactString, CompactString), i64> =
        UntrustedKeyMap::default();
    // visitor_timestamp_id -> last pageview timestamp_id within this flush,
    // seeded from the database the first time a visitor is seen.
    let mut last_pageview_by_visitor: FxHashMap<i64, i64> = FxHashMap::default();
    let mut landing_duration_by_ts: FxHashMap<i64, i64> = FxHashMap::default();
    let mut landing_session_deltas: FxHashMap<i64, (i64, i64)> = FxHashMap::default();
    let mut merge_candidates = Vec::new();

    for (index, page) in data.pageviews.iter().enumerate() {
        let duration_seconds = flush_pageview(
            conn,
            page,
            &mut path_ids,
            &mut last_pageview_by_visitor,
            &mut engagement_by_page,
        )
        .map_err(blame(BatchRow::Pageview(index)))?;
        let duration = duration_seconds.unwrap_or_default();
        if page.visitor_timestamp_id == page.timestamp_id {
            landing_duration_by_ts.insert(page.timestamp_id, duration);
        } else {
            let delta = landing_session_deltas
                .entry(page.visitor_timestamp_id)
                .or_insert((0, 0));
            delta.0 += 1;
            delta.1 += duration;
        }
    }

    let mut campaign_rules = None;
    let mut landing_params: FxHashMap<i64, serde_json::Map<String, Value>> = FxHashMap::default();
    if data
        .landings
        .iter()
        .any(|landing| landing.landing_campaign.is_none())
    {
        for page in &data.pageviews {
            if page.timestamp_id != page.visitor_timestamp_id {
                continue;
            }
            if let Some(json) = page.metadata_json.as_deref()
                && let Ok(Value::Object(obj)) = serde_json::from_str(json)
            {
                landing_params.insert(page.timestamp_id, obj);
            }
        }
    }

    for (index, landing) in data.landings.iter().enumerate() {
        let row = BatchRow::Landing(index);
        let referrer_id = match landing.external_referrer.as_deref() {
            Some(referrer) => Some(
                resolve_cached_referrer_id(conn, &mut referrer_ids, referrer)
                    .map_err(blame(row))?,
            ),
            None => None,
        };
        let campaign_id = match landing.landing_campaign.as_ref() {
            Some(tuple) => resolve_landing_campaign_id(conn, tuple, landing.timestamp_id)
                .map_err(blame(row))?,
            None => {
                if campaign_rules.is_none() {
                    campaign_rules = Some(
                        crate::campaigns::load_active_campaign_rules(conn).map_err(blame(row))?,
                    );
                }
                let rules = campaign_rules.as_deref().unwrap_or(&[]);
                crate::campaigns::matching_campaign_id(
                    rules,
                    landing_params.get(&landing.timestamp_id),
                )
            }
        };
        insert_landing_row(
            conn,
            landing,
            referrer_id,
            campaign_id,
            landing_duration_by_ts
                .get(&landing.timestamp_id)
                .copied()
                .unwrap_or_default(),
        )
        .map_err(blame(row))?;
    }

    for (key, estimated_count) in &data.crawler_views {
        let useragent_id = resolve_cached_useragent_id(conn, &mut useragent_ids, &key.useragent)?;
        let path_key = (key.path.clone(), key.hostname.clone());
        let path_id = match crawler_path_ids.get(&path_key) {
            Some(&path_id) => path_id,
            None => {
                let path_id = upsert_crawler_path(conn, &key.path, &key.hostname)?;
                crawler_path_ids.insert(path_key, path_id);
                path_id
            }
        };
        upsert_tracker_crawler_view(
            conn,
            key.hour,
            useragent_id,
            path_id,
            *estimated_count,
            key.verified,
        )?;
    }

    for (visitor_timestamp_id, (page_count_delta, duration_delta)) in landing_session_deltas {
        increment_landing_session_metrics(
            conn,
            visitor_timestamp_id,
            page_count_delta,
            duration_delta,
        )?;
    }

    // After the landings above, so a context hit batched with its own landing
    // pageview still finds the row it updates.
    for (index, context) in data.context_updates.iter().enumerate() {
        apply_landing_context_update(conn, context)
            .map_err(blame(BatchRow::ContextUpdate(index)))?;
    }

    for (index, event) in data.events.iter().enumerate() {
        let row = BatchRow::Event(index);
        let event_name_id =
            resolve_cached_event_name_id(conn, &mut event_name_ids, &event.event_name)
                .map_err(blame(row))?;
        conn.execute(
            "INSERT INTO events (timestamp_id, event_name_id, page_timestamp_id, metadata)
             VALUES (?1, ?2, ?3, jsonb(?4))",
            params![
                event.timestamp_id,
                event_name_id,
                event.page_timestamp_id,
                event.metadata
            ],
        )
        .map_err(blame(row))?;
    }

    // Engagements whose target pageview was not in this batch (it is already in
    // the database) are applied as updates.
    for (page_timestamp_id, accum) in engagement_by_page {
        apply_engagement_update(conn, page_timestamp_id, &accum)
            .map_err(blame(BatchRow::EngagementsForPage(page_timestamp_id)))?;
    }

    // Pageviews and metadata hits can be interleaved in one client batch. Apply
    // their metadata in enqueue order so later key changes always win.
    let mut metadata_writes =
        Vec::with_capacity(data.pageviews.len() + data.metadata_updates.len());
    for (index, page) in data.pageviews.iter().enumerate() {
        if let Some(metadata_json) = page.metadata_json.as_deref() {
            metadata_writes.push(MetadataWrite {
                batch_order: page.batch_order,
                row: BatchRow::Pageview(index),
                visitor_timestamp_id: page.visitor_timestamp_id,
                page_timestamp_id: page.timestamp_id,
                metadata_json,
            });
        }
    }
    for (index, update) in data.metadata_updates.iter().enumerate() {
        metadata_writes.push(MetadataWrite {
            batch_order: update.batch_order,
            row: BatchRow::MetadataUpdate(index),
            visitor_timestamp_id: update.visitor_timestamp_id,
            page_timestamp_id: update.page_timestamp_id,
            metadata_json: update.metadata_json.as_str(),
        });
    }
    metadata_writes.sort_unstable_by_key(|write| write.batch_order);
    for write in metadata_writes {
        upsert_metadata_rows(
            conn,
            write.visitor_timestamp_id,
            write.page_timestamp_id,
            Some(write.metadata_json),
            &mut merge_candidates,
        )
        .map_err(blame(write.row))?;
    }

    apply_session_merges(conn, &merge_candidates)?;
    apply_forget_requests(conn, &data.forgets)?;
    let now = i64::try_from(nanoseconds_since_epoch())?;
    purge_stale_user_sessions(conn, now)?;
    purge_stale_client_page_ids_map(conn, now)?;
    Ok(())
}

#[cfg(test)]
mod tests;
