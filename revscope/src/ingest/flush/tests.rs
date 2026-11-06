use std::sync::Arc;
use std::time::Duration;

use rusqlite::OptionalExtension;

use crate::db::ClientPageId;
use crate::ingest::rows::{EngagementRow, EventRow, ForgetRow, PageView};
use crate::ingest::test_support::{sample_landing_pageview, sample_pageview};
use crate::server_context::ServerContext;
use crate::test_utils::open_migrated_db;

use super::scheduler::{FLUSH_INTERVAL, LOCKED_GIVE_UP};
use super::scheduler::{
    FlushOutcome, TimerPass, WriteCheckout, drain_flush_all_sync, flush_due_batches_sync,
    flush_enqueue_batch, flush_project_batch_sync, run_threshold_flush,
};
use super::sessions::{
    CLIENT_PAGE_IDS_MAP_TTL_DAYS, NANOS_PER_DAY, purge_stale_client_page_ids_map,
};
use super::*;

fn insert_page_map(conn: &ProjectConnection, timestamp_id: i64, page_id_seed: u8) {
    let mut page_id = [0_u8; 12];
    page_id[0] = page_id_seed;
    conn.execute(
        "INSERT INTO pageviews (
            timestamp_id, visitor_timestamp_id, previous_timestamp_id, path_id,
            duration_seconds, scrolled_percent
         ) VALUES (?1, ?1, NULL, 1, 1, NULL)",
        params![timestamp_id],
    )
    .expect("insert pageview");
    conn.execute(
        "INSERT INTO client_page_ids_map (visitor_timestamp_id, page_id)
         VALUES (?1, ?2)",
        params![
            timestamp_id,
            ClientPageId::from_bytes(page_id).expect("page id")
        ],
    )
    .expect("insert page map");
}

#[test]
fn purges_client_page_ids_map_older_than_one_week() {
    let conn = open_migrated_db();
    let now = 1_000_000_000_000_000_000_i64;
    let week = CLIENT_PAGE_IDS_MAP_TTL_DAYS * NANOS_PER_DAY;
    let stale = now - week - 1;
    let fresh = now - week + 1;

    insert_page_map(&conn, stale, 1);
    insert_page_map(&conn, fresh, 2);

    purge_stale_client_page_ids_map(&conn, now).expect("purge");

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM client_page_ids_map", [], |row| {
            row.get(0)
        })
        .expect("count");
    assert_eq!(count, 1);

    let remaining: i64 = conn
        .query_row(
            "SELECT visitor_timestamp_id FROM client_page_ids_map",
            [],
            |row| row.get(0),
        )
        .expect("remaining");
    assert_eq!(remaining, fresh);
}

fn batch_of(pageviews: i64) -> IngestBatch {
    let mut batch = IngestBatch::new(0, 0);
    for n in 1..=pageviews {
        let mut page = sample_pageview(n * 1_000, n * 1_000, 1);
        page.path = CompactString::new(format!("/p{n}"));
        batch.queue_test_pageview(page);
    }
    batch
}

fn count_pageviews(conn: &ProjectConnection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM pageviews", [], |row| row.get(0))
        .expect("count pageviews")
}

#[test]
fn a_failure_no_row_can_be_blamed_for_gives_up_without_dropping_rows() {
    let conn = open_migrated_db();
    // BEGIN IMMEDIATE fails inside a transaction someone else left open.
    conn.execute_batch("BEGIN").expect("begin");
    let mut batch = batch_of(2);

    let failure = flush_batch_into(&conn, &mut batch).expect_err("flush fails");

    assert!(failure.dropped_rows.is_empty());
    assert_eq!(batch.pageviews.len(), 2, "the caller still has every row");
    conn.execute_batch("ROLLBACK").expect("rollback");
    assert_eq!(count_pageviews(&conn), 0);
}

#[test]
fn a_batch_that_rejects_every_row_stops_retrying_at_the_flush_threshold() {
    let conn = open_migrated_db();
    conn.execute_batch(
        "CREATE TRIGGER reject_every_path BEFORE INSERT ON paths
         BEGIN SELECT RAISE(ABORT, 'rejected'); END;",
    )
    .expect("create trigger");
    let mut batch = batch_of(i64::try_from(BATCH_FLUSH_THRESHOLD).unwrap() + 1);

    let failure = flush_batch_into(&conn, &mut batch).expect_err("flush gives up");

    assert_eq!(failure.dropped_rows.len(), BATCH_FLUSH_THRESHOLD);
    assert!(failure.dropped_rows[0].error_text.contains("rejected"));
    assert!(failure.dropped_rows[0].data.contains("/p1"));
    assert_eq!(
        batch.pageviews.len(),
        1,
        "the last row is left for the caller"
    );
    assert_eq!(count_pageviews(&conn), 0);
}

const ACCOUNT_ID: &str = "00000000-0000-0000-0000-0000000000f1";

/// An in-memory context with projects `1` and `2` cached and their
/// databases created. The pooled connections keep the shared-memory
/// databases alive between checkouts.
fn flush_context(namespace: &str) -> Arc<ServerContext> {
    let server_context = Arc::new(ServerContext::new_in_memory(namespace));
    for (project_id, name) in [(1, "flush_one"), (2, "flush_two")] {
        server_context
            .cache_project(project_id, name, ACCOUNT_ID)
            .expect("cache project");
        drop(
            crate::db::open_project_database_write(&server_context, &project_id.to_string())
                .expect("create project database"),
        );
    }
    server_context
}

/// Queue `pageviews` more pageviews, numbered after the ones already
/// stored or queued so repeated calls do not reuse timestamp ids.
fn queue(server_context: &ServerContext, project_id: i32, pageviews: i64) {
    let first = stored(server_context, project_id)
        + queued(server_context, project_id).map_or(0, |n| i64::try_from(n).unwrap())
        + 1;
    let mut guard = server_context.ingest_map.get(project_id);
    let data: &mut BatchData = &mut guard;
    for n in first..first + pageviews {
        data.queue_test_pageview(sample_pageview(n * 1_000, n * 1_000, 1));
    }
}

fn queued(server_context: &ServerContext, project_id: i32) -> Option<usize> {
    server_context
        .ingest_map
        .get_existing(project_id)
        .map(|guard| guard.pageviews.len())
}

fn stored(server_context: &ServerContext, project_id: i32) -> i64 {
    let conn = crate::db::open_project_database_read(server_context, &project_id.to_string())
        .expect("open project database");
    count_pageviews(&conn)
}

#[tokio::test(start_paused = true)]
async fn the_timer_leaves_a_batch_that_is_not_due_yet() {
    let server_context = flush_context("flush-timer-not-due");
    queue(&server_context, 1, 2);
    tokio::time::advance(FLUSH_INTERVAL - Duration::from_secs(1)).await;

    assert!(matches!(
        flush_due_batches_sync(&server_context),
        TimerPass::Idle
    ));
    assert_eq!(queued(&server_context, 1), Some(2));
    assert_eq!(stored(&server_context, 1), 0);
}

#[tokio::test(start_paused = true)]
async fn the_timer_keeps_a_write_busy_batch_and_flushes_it_once_the_slot_is_free() {
    let server_context = flush_context("flush-timer-busy");
    queue(&server_context, 1, 2);
    tokio::time::advance(FLUSH_INTERVAL).await;
    let held =
        crate::db::open_project_database_write(&server_context, "1").expect("hold the write slot");

    assert!(matches!(
        flush_due_batches_sync(&server_context),
        TimerPass::AllBusy
    ));
    assert_eq!(queued(&server_context, 1), Some(2));

    drop(held);
    assert!(matches!(
        flush_due_batches_sync(&server_context),
        TimerPass::Progress
    ));
    assert_eq!(stored(&server_context, 1), 2);
    assert_eq!(
        queued(&server_context, 1),
        None,
        "a flushed batch leaves the map"
    );
}

#[tokio::test(start_paused = true)]
async fn the_timer_flushes_the_projects_behind_a_write_busy_one() {
    let server_context = flush_context("flush-timer-skip");
    queue(&server_context, 1, 2);
    queue(&server_context, 2, 3);
    tokio::time::advance(FLUSH_INTERVAL).await;
    let held =
        crate::db::open_project_database_write(&server_context, "1").expect("hold the write slot");

    assert!(matches!(
        flush_due_batches_sync(&server_context),
        TimerPass::Progress
    ));
    assert_eq!(stored(&server_context, 2), 3);
    assert_eq!(queued(&server_context, 2), None);
    assert_eq!(queued(&server_context, 1), Some(2));
    drop(held);
}

#[tokio::test(start_paused = true)]
async fn a_threshold_flush_that_finds_the_slot_busy_keeps_its_rows() {
    let server_context = flush_context("flush-enqueue-busy");
    queue(&server_context, 1, 2);
    let held =
        crate::db::open_project_database_write(&server_context, "1").expect("hold the write slot");

    let guard = server_context.ingest_map.get(1);
    flush_enqueue_batch(&server_context, "1", guard);
    assert_eq!(queued(&server_context, 1), Some(2));

    drop(held);
    let guard = server_context.ingest_map.get(1);
    flush_enqueue_batch(&server_context, "1", guard);
    assert_eq!(stored(&server_context, 1), 2);
    // The batch stays in the map, emptied, for the next hits.
    assert_eq!(queued(&server_context, 1), Some(0));
}

/// Wait until no background threshold flush is queued or running for
/// `project_id`. The flag is cleared under the batch lock and the task keeps
/// that lock until its flush is done, so holding the lock with the flag
/// clear means the task has finished.
async fn wait_for_threshold_flush(server_context: &ServerContext, project_id: i32) {
    for _ in 0..500 {
        match server_context.ingest_map.get_existing(project_id) {
            Some(guard) if guard.flush_scheduled() => {}
            _ => return,
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the threshold flush never ran");
}

fn threshold() -> i64 {
    i64::try_from(BATCH_FLUSH_THRESHOLD).unwrap()
}

#[tokio::test]
async fn crossing_the_threshold_flushes_on_a_background_task() {
    let server_context = flush_context("flush-threshold-background");
    queue(&server_context, 1, threshold());

    schedule_threshold_flush(&server_context, server_context.ingest_map.get(1));
    wait_for_threshold_flush(&server_context, 1).await;

    assert_eq!(stored(&server_context, 1), threshold());
    // The batch stays in the map, emptied, for the next hits.
    assert_eq!(queued(&server_context, 1), Some(0));
}

#[tokio::test]
async fn a_batch_below_the_threshold_queues_no_flush() {
    let server_context = flush_context("flush-threshold-below");
    queue(&server_context, 1, threshold() - 1);
    let tasks = server_context.active_task_count();

    schedule_threshold_flush(&server_context, server_context.ingest_map.get(1));

    assert_eq!(server_context.active_task_count(), tasks, "no task spawned");
    assert_eq!(queued(&server_context, 1), Some(BATCH_FLUSH_THRESHOLD - 1));
}

#[tokio::test]
async fn hits_arriving_before_the_flush_starts_queue_no_second_flush() {
    let server_context = flush_context("flush-threshold-once");
    queue(&server_context, 1, threshold());
    // A flush is queued and its task has not taken the lock yet.
    assert!(server_context.ingest_map.get(1).schedule_flush());
    let tasks = server_context.active_task_count();

    queue(&server_context, 1, 1);
    schedule_threshold_flush(&server_context, server_context.ingest_map.get(1));

    assert_eq!(
        server_context.active_task_count(),
        tasks,
        "no second task spawned"
    );
    // The queued task writes the hits that joined the batch meanwhile.
    run_threshold_flush(&server_context, 1);
    assert_eq!(stored(&server_context, 1), threshold() + 1);
}

#[tokio::test]
async fn a_flush_blocked_by_a_table_lock_keeps_every_row_for_the_next_flush() {
    let server_context = flush_context("flush-threshold-locked");
    queue(&server_context, 1, threshold());
    // In a shared-cache database a read transaction holds a lock on each
    // table it read, so inserting into pageviews fails with SQLITE_LOCKED.
    let reader = crate::db::open_project_database_read(&server_context, "1").expect("open reader");
    reader.execute_batch("BEGIN").expect("begin read");
    assert_eq!(count_pageviews(&reader), 0);

    assert!(server_context.ingest_map.get(1).schedule_flush());
    run_threshold_flush(&server_context, 1);
    assert_eq!(
        queued(&server_context, 1),
        Some(BATCH_FLUSH_THRESHOLD),
        "no row is dropped for another connection's lock"
    );

    reader.execute_batch("COMMIT").expect("end read");
    drop(reader);
    assert!(server_context.ingest_map.get(1).schedule_flush());
    run_threshold_flush(&server_context, 1);
    assert_eq!(stored(&server_context, 1), threshold());
}

#[tokio::test]
async fn a_batch_locked_out_for_the_give_up_time_is_archived_and_discarded() {
    let server_context = flush_context("flush-locked-give-up");
    queue(&server_context, 1, 2);
    queue(&server_context, 2, 2);
    let reader = crate::db::open_project_database_read(&server_context, "1").expect("open reader");
    reader.execute_batch("BEGIN").expect("begin read");
    assert_eq!(count_pageviews(&reader), 0);

    let mut guard = server_context.ingest_map.get(1);
    let first = flush_project_batch_sync(
        &server_context,
        "1",
        &mut guard,
        WriteCheckout::Try,
        OrphanEvents::Carry,
    );
    assert!(matches!(first, Ok(FlushOutcome::Locked(_))));
    assert_eq!(guard.pageviews.len(), 2, "a locked flush keeps its rows");
    guard.set_locked_since(tokio::time::Instant::now() - LOCKED_GIVE_UP);
    drop(guard);

    // The drain retries a locked batch, so returning at all means it gave
    // up on project 1 rather than waiting on the reader.
    drain_flush_all_sync(&server_context);

    assert_eq!(queued(&server_context, 1), None);
    assert_eq!(stored(&server_context, 2), 2, "other projects still drain");
    reader.execute_batch("COMMIT").expect("end read");
    assert_eq!(count_pageviews(&reader), 0);
}

#[tokio::test]
async fn a_queued_flush_leaves_alone_a_batch_recreated_after_the_timer_popped_its_own() {
    let server_context = flush_context("flush-threshold-timer-won");
    queue(&server_context, 1, threshold());
    assert!(server_context.ingest_map.get(1).schedule_flush());

    // The timer flushes and pops the batch before the task gets the lock,
    // and new hits fill a fresh batch for the project past the threshold
    // without queueing a flush of their own.
    let mut guard = server_context.ingest_map.get(1);
    let flushed = flush_project_batch_sync(
        &server_context,
        "1",
        &mut guard,
        WriteCheckout::Try,
        OrphanEvents::Carry,
    )
    .expect("timer flush");
    assert!(matches!(flushed, FlushOutcome::Flushed));
    server_context.ingest_map.pop(guard);
    queue(&server_context, 1, threshold());

    run_threshold_flush(&server_context, 1);
    assert_eq!(stored(&server_context, 1), threshold());
    assert_eq!(
        queued(&server_context, 1),
        Some(BATCH_FLUSH_THRESHOLD),
        "the stale task does not flush the new batch"
    );
}

#[tokio::test]
async fn a_threshold_flush_that_found_the_slot_busy_is_queued_again_by_the_next_hit() {
    let server_context = flush_context("flush-threshold-busy-retry");
    queue(&server_context, 1, threshold());
    let held =
        crate::db::open_project_database_write(&server_context, "1").expect("hold the write slot");

    schedule_threshold_flush(&server_context, server_context.ingest_map.get(1));
    wait_for_threshold_flush(&server_context, 1).await;
    assert_eq!(queued(&server_context, 1), Some(BATCH_FLUSH_THRESHOLD));
    drop(held);

    queue(&server_context, 1, 1);
    schedule_threshold_flush(&server_context, server_context.ingest_map.get(1));
    wait_for_threshold_flush(&server_context, 1).await;
    assert_eq!(stored(&server_context, 1), threshold() + 1);
}

fn sample_pageview_with_metadata(
    timestamp_id: i64,
    visitor_timestamp_id: i64,
    duration_seconds: i64,
    metadata_json: &str,
) -> PageView {
    let mut page = sample_pageview(timestamp_id, visitor_timestamp_id, duration_seconds);
    page.metadata_json = Some(metadata_json.to_string());
    page
}

#[test]
fn flushing_pageviews_and_engagement_maintains_landing_session_metrics() {
    let conn = open_migrated_db();

    let mut batch = IngestBatch::new(0, 0);
    {
        let data: &mut BatchData = &mut batch;
        data.pageviews.push(sample_pageview(1_000, 1_000, 3));
        data.landings.push(sample_landing_pageview(1_000));
        data.pageview_idx_by_ts.insert(1_000, 0);
        data.pageviews.push(sample_pageview(1_100, 1_000, 5));
        data.pageview_idx_by_ts.insert(1_100, 1);
        data.engagements.push(EngagementRow {
            page_timestamp_id: 1_100,
            duration_increment: 7,
            scrolled_percent: Some(80),
        });
        data.total_events = 3;
    }
    flush_batch_into(&conn, &mut batch).expect("flush");

    let (session_duration_seconds, session_page_count): (i64, i64) = conn
        .query_row(
            "SELECT session_duration_seconds, session_page_count
             FROM landings
             WHERE visitor_timestamp_id = ?1",
            params![1_000_i64],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("landing metrics");
    assert_eq!(session_duration_seconds, 3 + 5 + 7);
    assert_eq!(session_page_count, 2);

    let second_duration: i64 = conn
        .query_row(
            "SELECT duration_seconds
             FROM pageviews
             WHERE timestamp_id = ?1",
            params![1_100_i64],
            |row| row.get(0),
        )
        .expect("page duration");
    assert_eq!(second_duration, 5 + 7);

    let second_scroll: Option<i64> = conn
        .query_row(
            "SELECT scrolled_percent
             FROM pageviews
             WHERE timestamp_id = ?1",
            params![1_100_i64],
            |row| row.get(0),
        )
        .expect("page scroll");
    assert_eq!(second_scroll, Some(80));
}

/// Overflowing the engagement accumulator is the shape of bug the flush has
/// to survive: it aborts the attempt from inside the write transaction. Only
/// a debug build traps the overflow, so the test is gated accordingly.
#[cfg(debug_assertions)]
#[test]
fn flush_reports_a_panic_and_leaves_the_batch_intact_for_archiving() {
    let conn = open_migrated_db();

    let mut batch = IngestBatch::new(0, 0);
    {
        let data: &mut BatchData = &mut batch;
        data.pageviews.push(sample_pageview(1_000, 1_000, 3));
        data.landings.push(sample_landing_pageview(1_000));
        for _ in 0..2 {
            data.engagements.push(EngagementRow {
                page_timestamp_id: 1_000,
                duration_increment: i64::MAX,
                scrolled_percent: None,
            });
        }
        data.total_events = 3;
    }

    let error = flush_batch_into(&conn, &mut batch).expect_err("flush should fail");

    assert!(
        matches!(error.error, PersistError::Panic(_)),
        "{:?}",
        error.error
    );
    assert_eq!(error.error.error_type(), "panic");
    // The caller serializes these rows into errors.data, so they have to
    // still be there.
    assert_eq!(batch.pageviews.len(), 1);
    assert_eq!(batch.engagements.len(), 2);
    // The unwind skipped the rollback, so the connection must have been
    // rolled back before it goes back to the pool.
    assert!(conn.is_autocommit());
}

#[test]
fn flush_drops_a_pageview_the_database_rejects_and_keeps_the_rest() {
    let conn = open_migrated_db();

    let mut batch = IngestBatch::new(0, 0);
    {
        let data: &mut BatchData = &mut batch;
        data.pageviews.push(sample_pageview(1_000, 1_000, 3));
        data.landings.push(sample_landing_pageview(1_000));
        let mut poisoned = sample_pageview(1_100, 1_100, 5);
        poisoned.scrolled_percent = Some(500);
        data.pageviews.push(poisoned);
        data.landings.push(sample_landing_pageview(1_100));
        data.total_events = 2;
    }

    let dropped = flush_batch_into(&conn, &mut batch).expect("flush");

    assert_eq!(dropped.len(), 1);
    assert_eq!(dropped[0].error_type, "database");
    assert!(
        dropped[0].error_text.contains("CHECK constraint failed"),
        "{}",
        dropped[0].error_text
    );
    assert!(
        dropped[0].data.contains(r#""timestamp_id":1100"#),
        "{}",
        dropped[0].data
    );
    assert!(dropped[0].data.contains(r#""scrolled_percent":500"#));
    assert_eq!(batch.pageviews.len(), 1);

    let pageview_ids = conn
        .with_query(
            "SELECT timestamp_id FROM pageviews ORDER BY timestamp_id",
            [],
            |mut rows| {
                let mut ids = Vec::new();
                while let Some(row) = rows.next()? {
                    ids.push(row.get::<_, i64>(0)?);
                }
                Ok(ids)
            },
        )
        .expect("pageview ids");
    assert_eq!(pageview_ids, vec![1_000]);

    // The landing row keys off the dropped pageview, so it goes with it.
    let landing_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM landings", [], |row| row.get(0))
        .expect("landing count");
    assert_eq!(landing_count, 1);
}

#[test]
fn flush_drops_an_event_the_database_rejects_and_keeps_the_rest() {
    let conn = open_migrated_db();

    let mut batch = IngestBatch::new(0, 0);
    {
        let data: &mut BatchData = &mut batch;
        data.pageviews.push(sample_pageview(2_000, 2_000, 1));
        data.landings.push(sample_landing_pageview(2_000));
        data.events.push(EventRow {
            timestamp_id: 2_050,
            event_name: CompactString::new("signup"),
            page_timestamp_id: 2_000,
            // jsonb() rejects malformed JSON.
            metadata: Some(r#"{"broken""#.to_string()),
        });
        data.total_events = 2;
    }

    let dropped = flush_batch_into(&conn, &mut batch).expect("flush");

    assert_eq!(dropped.len(), 1);
    assert!(
        dropped[0].data.contains(r#""row":"event""#),
        "{}",
        dropped[0].data
    );
    assert!(dropped[0].data.contains("signup"), "{}", dropped[0].data);
    assert!(batch.events.is_empty());

    let pageview_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM pageviews", [], |row| row.get(0))
        .expect("pageview count");
    assert_eq!(pageview_count, 1);
    let event_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
        .expect("event count");
    assert_eq!(event_count, 0);
}

#[test]
fn flushing_forget_erases_sessions_resolved_by_hash_and_page() {
    let conn = open_migrated_db();
    let first = i64::try_from(nanoseconds_since_epoch()).expect("current timestamp");
    let sessions = [first, first + 10, first + 20, first + 30];
    let mut pages = sessions
        .iter()
        .map(|&session_id| {
            sample_pageview_with_metadata(session_id, session_id, 1, r#"{"account_id":"erase-me"}"#)
        })
        .collect::<Vec<_>>();
    let second_page_id = pages[1].client_page_id.clone();
    let first_user_hash = pages[0].user_hash;

    let mut batch = IngestBatch::new(0, 0);
    {
        let data: &mut BatchData = &mut batch;
        for (index, page) in pages.drain(..).enumerate() {
            data.landings
                .push(sample_landing_pageview(page.timestamp_id));
            data.events.push(EventRow {
                timestamp_id: first + 100 + index as i64,
                event_name: CompactString::new("test-event"),
                page_timestamp_id: page.timestamp_id,
                metadata: None,
            });
            data.pageviews.push(page);
        }
        // Payload session sessions[2] must not erase: user_hash maps to
        // sessions[0], not sessions[2].
        data.forgets.push(ForgetRow {
            user_hash: first_user_hash,
            client_page_id: Some(second_page_id),
            session_id: Some(sessions[2]),
        });
        data.total_events = data.pageviews.len() + data.events.len() + data.forgets.len();
    }
    flush_batch_into(&conn, &mut batch).expect("flush");

    let mut remaining: Vec<i64> = conn
        .prepare("SELECT visitor_timestamp_id FROM pageviews ORDER BY visitor_timestamp_id")
        .expect("prepare")
        .query_map([], |row| row.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("rows");
    remaining.sort_unstable();
    assert_eq!(remaining, vec![sessions[2], sessions[3]]);
}

#[test]
fn forget_payload_session_requires_matching_user_session() {
    let conn = open_migrated_db();
    let session_id = i64::try_from(nanoseconds_since_epoch()).expect("current timestamp");
    let mut initial = IngestBatch::new(0, 0);
    {
        let data: &mut BatchData = &mut initial;
        data.pageviews
            .push(sample_pageview(session_id, session_id, 1));
        data.landings.push(sample_landing_pageview(session_id));
        data.total_events = 1;
    }
    flush_batch_into(&conn, &mut initial).expect("flush session");

    let mut mismatched = IngestBatch::new(0, 0);
    {
        let data: &mut BatchData = &mut mismatched;
        data.forgets.push(ForgetRow {
            user_hash: u128::MAX - 1,
            client_page_id: None,
            session_id: Some(session_id),
        });
        data.total_events = 1;
    }
    flush_batch_into(&conn, &mut mismatched).expect("flush mismatched forget");

    let remaining: i64 = conn
        .query_row("SELECT COUNT(*) FROM pageviews", [], |row| row.get(0))
        .expect("pageview count");
    assert_eq!(remaining, 1);

    let mut matched = IngestBatch::new(0, 0);
    {
        let data: &mut BatchData = &mut matched;
        data.forgets.push(ForgetRow {
            user_hash: session_id as u128,
            client_page_id: None,
            session_id: Some(session_id),
        });
        data.total_events = 1;
    }
    flush_batch_into(&conn, &mut matched).expect("flush matched forget");

    let remaining: i64 = conn
        .query_row("SELECT COUNT(*) FROM pageviews", [], |row| row.get(0))
        .expect("pageview count");
    assert_eq!(remaining, 0);
}

#[test]
fn forgetting_session_detaches_provider_records() {
    let conn = open_migrated_db();
    let session_id = i64::try_from(nanoseconds_since_epoch()).expect("current timestamp");
    let mut initial = IngestBatch::new(0, 0);
    {
        let data: &mut BatchData = &mut initial;
        data.pageviews
            .push(sample_pageview(session_id, session_id, 1));
        data.landings.push(sample_landing_pageview(session_id));
        data.total_events = 1;
    }
    flush_batch_into(&conn, &mut initial).expect("flush session");

    conn.execute(
        "INSERT INTO payments (
            provider, provider_payment_id, visitor_timestamp_id,
            attributed_visitor_timestamp_id, payment_type, status, amount, currency,
            paid_at, created_at
         ) VALUES (
            'stripe', 'pay_forget', ?1, ?1, 'single', 'ok', 1000, 'USD', 5, 5
         )",
        params![session_id],
    )
    .expect("insert payment");
    conn.execute(
        "INSERT INTO subscriptions (
            provider, provider_subscription_id, visitor_timestamp_id,
            attributed_visitor_timestamp_id, currency, interval_unit, interval_count
         ) VALUES (
            'stripe', 'sub_forget', ?1, ?1, 'USD', 'month', 1
         )",
        params![session_id],
    )
    .expect("insert subscription");

    let mut forget = IngestBatch::new(0, 0);
    {
        let data: &mut BatchData = &mut forget;
        data.forgets.push(ForgetRow {
            user_hash: session_id as u128,
            client_page_id: None,
            session_id: Some(session_id),
        });
        data.total_events = 1;
    }
    flush_batch_into(&conn, &mut forget).expect("flush forget");

    let payment_sessions: (Option<i64>, Option<i64>) = conn
        .query_row(
            "SELECT visitor_timestamp_id, attributed_visitor_timestamp_id
             FROM payments
             WHERE provider_payment_id = 'pay_forget'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("payment");
    assert_eq!(payment_sessions, (None, None));
    let subscription_sessions: (Option<i64>, Option<i64>) = conn
        .query_row(
            "SELECT visitor_timestamp_id, attributed_visitor_timestamp_id
             FROM subscriptions
             WHERE provider_subscription_id = 'sub_forget'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("subscription");
    assert_eq!(subscription_sessions, (None, None));
}

#[test]
fn flushing_folds_engagement_for_existing_pageview_via_update() {
    let conn = open_migrated_db();

    // Pre-existing pageview + landing already in the database (from a prior batch).
    let mut prior = IngestBatch::new(0, 0);
    {
        let data: &mut BatchData = &mut prior;
        data.pageviews.push(sample_pageview(2_000, 2_000, 4));
        data.landings.push(sample_landing_pageview(2_000));
        data.pageview_idx_by_ts.insert(2_000, 0);
        data.total_events = 1;
    }
    flush_batch_into(&conn, &mut prior).expect("flush prior");

    // A later batch only carries an engagement targeting that pageview.
    let mut batch = IngestBatch::new(0, 0);
    {
        let data: &mut BatchData = &mut batch;
        data.engagements.push(EngagementRow {
            page_timestamp_id: 2_000,
            duration_increment: 9,
            scrolled_percent: Some(55),
        });
        data.total_events = 1;
    }
    flush_batch_into(&conn, &mut batch).expect("flush engagement");

    let (duration, scroll): (i64, Option<i64>) = conn
        .query_row(
            "SELECT duration_seconds, scrolled_percent
             FROM pageviews
             WHERE timestamp_id = ?1",
            params![2_000_i64],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("page");
    assert_eq!(duration, 4 + 9);
    assert_eq!(scroll, Some(55));

    let session_duration: i64 = conn
        .query_row(
            "SELECT session_duration_seconds FROM landings WHERE visitor_timestamp_id = ?1",
            params![2_000_i64],
            |row| row.get(0),
        )
        .expect("landing");
    assert_eq!(session_duration, 4 + 9);
}

#[test]
fn flushing_merges_identity_metadata_sessions_and_related_rows() {
    let conn = open_migrated_db();

    let mut original = IngestBatch::new(0, 0);
    {
        let data: &mut BatchData = &mut original;
        data.pageviews.push(sample_pageview_with_metadata(
            3_000,
            3_000,
            3,
            r#"{"user_email":"buyer@example.com","account_id":"acct_123"}"#,
        ));
        data.landings.push(sample_landing_pageview(3_000));
        data.pageview_idx_by_ts.insert(3_000, 0);
        data.total_events = 1;
    }
    flush_batch_into(&conn, &mut original).expect("flush original");

    conn.execute(
        "INSERT INTO payments (
            provider, provider_payment_id, visitor_timestamp_id,
            attributed_visitor_timestamp_id, payment_type, status, amount, currency,
            paid_at, created_at
         ) VALUES (
            'stripe', 'pay_merge', 4_000, 4_000, 'single', 'ok', 1000, 'USD', 5, 5
         )",
        [],
    )
    .expect("insert payment");
    conn.execute(
        "INSERT INTO subscriptions (
            provider, provider_subscription_id, visitor_timestamp_id,
            attributed_visitor_timestamp_id, currency, interval_unit, interval_count
         ) VALUES (
            'stripe', 'sub_merge', 4_000, 4_000, 'USD', 'month', 1
         )",
        [],
    )
    .expect("insert subscription");
    conn.execute(
        "INSERT INTO user_sessions (user_hash, visitor_timestamp_id, expires_after_days)
         VALUES (?1, 4_000, 1000000)",
        params![[4_u8; 16]],
    )
    .expect("insert user session");

    let mut duplicate = IngestBatch::new(0, 0);
    {
        let data: &mut BatchData = &mut duplicate;
        data.pageviews.push(sample_pageview_with_metadata(
            4_000,
            4_000,
            4,
            r#"{"user_email":"BUYER@example.com","account_id":"acct_123"}"#,
        ));
        data.landings.push(sample_landing_pageview(4_000));
        data.pageview_idx_by_ts.insert(4_000, 0);
        data.pageviews.push(sample_pageview(4_100, 4_000, 5));
        data.pageview_idx_by_ts.insert(4_100, 1);
        data.total_events = 2;
    }
    flush_batch_into(&conn, &mut duplicate).expect("flush duplicate");

    let remapped_pageviews: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pageviews WHERE visitor_timestamp_id = 3_000",
            [],
            |row| row.get(0),
        )
        .expect("remapped pageviews");
    assert_eq!(remapped_pageviews, 3);
    let duplicate_landing_exists: Option<i64> = conn
        .query_row(
            "SELECT visitor_timestamp_id FROM landings WHERE visitor_timestamp_id = 4_000",
            [],
            |row| row.get(0),
        )
        .optional()
        .expect("duplicate landing query");
    assert_eq!(duplicate_landing_exists, None);
    let original_metrics: (i64, i64) = conn
        .query_row(
            "SELECT session_duration_seconds, session_page_count
             FROM landings
             WHERE visitor_timestamp_id = 3_000",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("merged landing metrics");
    assert_eq!(original_metrics, (3 + 4 + 5, 3));
    let metadata_value: String = conn
        .query_row(
            "SELECT value
             FROM metadata
             WHERE visitor_timestamp_id = 3_000
               AND key = 'user_email'",
            [],
            |row| row.get(0),
        )
        .expect("merged metadata");
    assert_eq!(metadata_value, "BUYER@example.com");
    let user_session_visitor: i64 = conn
        .query_row(
            "SELECT visitor_timestamp_id FROM user_sessions WHERE user_hash = ?1",
            params![[4_u8; 16]],
            |row| row.get(0),
        )
        .expect("remapped user session");
    assert_eq!(user_session_visitor, 3_000);
    let payment_visitors: (Option<i64>, Option<i64>) = conn
        .query_row(
            "SELECT visitor_timestamp_id, attributed_visitor_timestamp_id
             FROM payments
             WHERE provider_payment_id = 'pay_merge'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("remapped payment");
    assert_eq!(payment_visitors, (Some(3_000), Some(3_000)));
    let subscription_visitors: (Option<i64>, Option<i64>) = conn
        .query_row(
            "SELECT visitor_timestamp_id, attributed_visitor_timestamp_id
             FROM subscriptions
             WHERE provider_subscription_id = 'sub_merge'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("remapped subscription");
    assert_eq!(subscription_visitors, (Some(3_000), Some(3_000)));

    let mut engagement = IngestBatch::new(0, 0);
    {
        let data: &mut BatchData = &mut engagement;
        data.engagements.push(EngagementRow {
            page_timestamp_id: 4_100,
            duration_increment: 6,
            scrolled_percent: None,
        });
        data.total_events = 1;
    }
    flush_batch_into(&conn, &mut engagement).expect("flush engagement");
    let session_duration_after_engagement: i64 = conn
        .query_row(
            "SELECT session_duration_seconds
             FROM landings
             WHERE visitor_timestamp_id = 3_000",
            [],
            |row| row.get(0),
        )
        .expect("engagement landing duration");
    assert_eq!(session_duration_after_engagement, 3 + 4 + 5 + 6);
}

#[test]
fn flushing_does_not_merge_on_account_id_only() {
    let conn = open_migrated_db();
    let mut batch = IngestBatch::new(0, 0);
    {
        let data: &mut BatchData = &mut batch;
        data.pageviews.push(sample_pageview_with_metadata(
            5_000,
            5_000,
            3,
            r#"{"account_id":"acct_123"}"#,
        ));
        data.landings.push(sample_landing_pageview(5_000));
        data.pageview_idx_by_ts.insert(5_000, 0);
        data.pageviews.push(sample_pageview_with_metadata(
            6_000,
            6_000,
            4,
            r#"{"account_id":"ACCT_123"}"#,
        ));
        data.landings.push(sample_landing_pageview(6_000));
        data.pageview_idx_by_ts.insert(6_000, 1);
        data.total_events = 2;
    }
    flush_batch_into(&conn, &mut batch).expect("flush");

    let distinct_sessions: i64 = conn
        .query_row("SELECT COUNT(*) FROM landings", [], |row| row.get(0))
        .expect("landing count");
    assert_eq!(distinct_sessions, 2);
}

#[test]
fn session_merge_uses_metadata_key_priority() {
    let conn = open_migrated_db();
    let mut batch = IngestBatch::new(0, 0);
    {
        let data: &mut BatchData = &mut batch;
        data.pageviews.push(sample_pageview_with_metadata(
            7_000,
            7_000,
            3,
            r#"{"user_id":"user_123"}"#,
        ));
        data.landings.push(sample_landing_pageview(7_000));
        data.pageview_idx_by_ts.insert(7_000, 0);
        data.pageviews.push(sample_pageview_with_metadata(
            8_000,
            8_000,
            3,
            r#"{"attribution_key":"key_123"}"#,
        ));
        data.landings.push(sample_landing_pageview(8_000));
        data.pageview_idx_by_ts.insert(8_000, 1);
        data.pageviews.push(sample_pageview_with_metadata(
            9_000,
            9_000,
            3,
            r#"{"attribution_key":"KEY_123","user_id":"user_123"}"#,
        ));
        data.landings.push(sample_landing_pageview(9_000));
        data.pageview_idx_by_ts.insert(9_000, 2);
        data.total_events = 3;
    }
    flush_batch_into(&conn, &mut batch).expect("flush");

    let merged_target: i64 = conn
        .query_row(
            "SELECT visitor_timestamp_id FROM pageviews WHERE timestamp_id = 9_000",
            [],
            |row| row.get(0),
        )
        .expect("merged visitor");
    assert_eq!(merged_target, 8_000);
}
