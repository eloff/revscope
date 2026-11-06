use fxhash::FxHashSet;

use super::batch::BatchData;
use super::error::PersistError;
use super::lookup::get_read_connection;
use super::map::IngestBatch;
use crate::db::ProjectConnection;
use crate::server_context::ServerContext;
use crate::util::nanoseconds_since_epoch;

pub(super) const NANOS_PER_MILLI: i64 = 1_000_000;

/// Subtracted from every assigned timestamp id to cover request flight time and
/// the parsing we did before reading the clock.
pub(super) const INGEST_LATENCY_ALLOWANCE_NANOS: i64 = 100 * NANOS_PER_MILLI;

/// Matches `batchFlushDelay` (30 seconds) in `tracker/tracker.js` — the longest a
/// hit normally waits in the client queue before send. Used as the empty-table
/// floor so a fresh project can still backdate within one flush window.
const TRACKER_BATCH_FLUSH_DELAY_NANOS: i64 = 30_000 * NANOS_PER_MILLI;

/// Assign a unique pageview timestamp id, backdated towards when the pageview
/// happened (see [`backdated_timestamp_id`]).
pub(super) fn assign_pageview_timestamp(
    server_context: &ServerContext,
    project_id: &str,
    batch: &mut IngestBatch,
    read_conn: &mut Option<ProjectConnection>,
    client_age_ms: i64,
) -> Result<i64, PersistError> {
    let received_nanos = i64::try_from(nanoseconds_since_epoch())?;
    if batch.db_max_pageview_ts.is_none() {
        let conn = get_read_connection(server_context, project_id, read_conn)?;
        let empty_floor = received_nanos.saturating_sub(TRACKER_BATCH_FLUSH_DELAY_NANOS);
        batch.db_max_pageview_ts = Some(max_pageview_timestamp_id(conn, empty_floor)?);
    }

    let data: &mut BatchData = &mut *batch;
    let db_max = data.db_max_pageview_ts.expect("db max read above");
    Ok(backdated_timestamp_id(
        received_nanos,
        client_age_ms,
        db_max,
        &mut data.queued_pageview_ts,
    ))
}

/// Assign a unique event timestamp id, backdated towards when the event
/// happened (see [`backdated_timestamp_id`]).
///
/// Pageview and event ids are floored against their own tables, so a pageview
/// held back by its floor could otherwise end up newer than an event fired on
/// it. `page_timestamp_id` keeps the event after the page it belongs to.
pub(super) fn assign_event_timestamp(
    server_context: &ServerContext,
    project_id: &str,
    batch: &mut IngestBatch,
    read_conn: &mut Option<ProjectConnection>,
    received_nanos: i64,
    client_age_ms: i64,
    page_timestamp_id: i64,
) -> Result<i64, PersistError> {
    if batch.db_max_event_ts.is_none() {
        let conn = get_read_connection(server_context, project_id, read_conn)?;
        let empty_floor = received_nanos.saturating_sub(TRACKER_BATCH_FLUSH_DELAY_NANOS);
        batch.db_max_event_ts = Some(max_event_timestamp_id(conn, empty_floor)?);
    }

    let data: &mut BatchData = &mut *batch;
    let floor = data
        .db_max_event_ts
        .expect("db max read above")
        .max(page_timestamp_id);
    Ok(backdated_timestamp_id(
        received_nanos,
        client_age_ms,
        floor,
        &mut data.queued_event_ts,
    ))
}

/// Turn the server receive time into an id that sits as close as we can get to
/// when the hit happened: back off the age the client measured for it and a
/// fixed allowance for flight and processing time.
///
/// Two rules keep the result usable as a primary key. It stays above `floor`,
/// which covers the ids already written for the project, so backdating can never
/// reach a row on disk. And it is bumped past any id already handed out to this
/// batch, since backdating can put two hits on the same nanosecond.
fn backdated_timestamp_id(
    received_nanos: i64,
    client_age_ms: i64,
    floor: i64,
    queued: &mut FxHashSet<i64>,
) -> i64 {
    let mut timestamp_id = received_nanos
        .saturating_sub(client_age_ms.saturating_mul(NANOS_PER_MILLI))
        .saturating_sub(INGEST_LATENCY_ALLOWANCE_NANOS)
        .max(floor.saturating_add(1));
    while !queued.insert(timestamp_id) {
        timestamp_id += 1;
    }
    timestamp_id
}

/// Highest pageview timestamp id already on disk. When the table is empty,
/// returns `empty_floor` (server receive time minus the tracker batch flush
/// delay) so backdating still has a sensible lower bound.
fn max_pageview_timestamp_id(
    conn: &ProjectConnection,
    empty_floor: i64,
) -> Result<i64, PersistError> {
    conn.query_row("SELECT MAX(timestamp_id) FROM pageviews", [], |row| {
        row.get::<_, Option<i64>>(0)
    })
    .map(|max_id| max_id.unwrap_or(empty_floor))
    .map_err(PersistError::from)
}

/// Highest event timestamp id already on disk. When the table is empty, returns
/// `empty_floor` (server receive time minus the tracker batch flush delay).
fn max_event_timestamp_id(conn: &ProjectConnection, empty_floor: i64) -> Result<i64, PersistError> {
    conn.query_row("SELECT MAX(timestamp_id) FROM events", [], |row| {
        row.get::<_, Option<i64>>(0)
    })
    .map(|max_id| max_id.unwrap_or(empty_floor))
    .map_err(PersistError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backdated_timestamp_id_backs_off_client_age_and_latency() {
        let received = 1_700_000_000_000_000_000_i64;
        let mut queued = FxHashSet::default();

        assert_eq!(
            backdated_timestamp_id(received, 5_000, 0, &mut queued),
            received - 5_000 * NANOS_PER_MILLI - INGEST_LATENCY_ALLOWANCE_NANOS
        );
    }

    #[test]
    fn backdated_timestamp_id_stays_above_rows_already_written() {
        let received = 1_700_000_000_000_000_000_i64;
        let db_max = received - 1_000 * NANOS_PER_MILLI;
        let mut queued = FxHashSet::default();

        assert_eq!(
            backdated_timestamp_id(received, 30_000, db_max, &mut queued),
            db_max + 1
        );
    }

    #[test]
    fn empty_table_floor_matches_tracker_batch_flush_delay() {
        // When MAX(timestamp_id) is NULL, assign_* uses receive time minus this
        // window as the floor so a fresh project can still backdate within one
        // client flush cycle (batchFlushDelay in tracker.js).
        assert_eq!(TRACKER_BATCH_FLUSH_DELAY_NANOS, 30_000 * NANOS_PER_MILLI);

        let received = 1_700_000_000_000_000_000_i64;
        let empty_floor = received - TRACKER_BATCH_FLUSH_DELAY_NANOS;
        let mut queued = FxHashSet::default();
        assert_eq!(
            backdated_timestamp_id(received, 30_000, empty_floor, &mut queued),
            empty_floor + 1
        );
    }

    #[test]
    fn backdated_timestamp_id_bumps_ids_that_repeat_within_the_batch() {
        let received = 1_700_000_000_000_000_000_i64;
        let mut queued = FxHashSet::default();

        let first = backdated_timestamp_id(received, 1_000, 0, &mut queued);
        let second = backdated_timestamp_id(received, 1_000, 0, &mut queued);
        let third = backdated_timestamp_id(received, 1_000, 0, &mut queued);

        assert_eq!(second, first + 1);
        assert_eq!(third, first + 2);
    }
}
