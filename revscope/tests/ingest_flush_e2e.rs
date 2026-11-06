//! The ingest flush when it cannot simply write: a row the project database
//! rejects, and a write slot someone else is holding.

mod common;

use std::io;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use common::{
    DEMO_PROJECT_ID, DEMO_PROJECT_NAME, TestServer, TrackHit, batch_form, count_rows, page_id,
    query_i64,
};

#[tokio::test]
async fn a_row_the_database_rejects_is_dropped_and_recorded_and_the_rest_persist() -> io::Result<()>
{
    let mut server = TestServer::spawn("flush-poisoned-row").await?;
    // Tests share the demo project's `errors` rows in one postgres, so the
    // rejection carries a marker only this run produces.
    let marker = format!(
        "poisoned-{:x}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after the unix epoch")
            .as_nanos()
    );
    {
        let conn =
            revscope::db::open_project_database_write(server.context.as_ref(), DEMO_PROJECT_ID)
                .map_err(io::Error::other)?;
        conn.execute_batch(&format!(
            "CREATE TRIGGER reject_poison BEFORE INSERT ON paths
             WHEN NEW.path = '/poison'
             BEGIN SELECT RAISE(ABORT, '{marker}'); END;"
        ))
        .map_err(io::Error::other)?;
    }

    let items = [
        TrackHit::pageview(DEMO_PROJECT_NAME, "/first", &page_id(1)),
        TrackHit::pageview(DEMO_PROJECT_NAME, "/poison", &page_id(2)),
        TrackHit::pageview(DEMO_PROJECT_NAME, "/last", &page_id(3)),
    ];
    let response = server
        .post_ingest(&batch_form(DEMO_PROJECT_NAME, &items), &[])
        .await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    server.flush().await;

    {
        let conn = server.project_db(DEMO_PROJECT_ID);
        assert_eq!(
            count_rows(&conn, "pageviews"),
            2,
            "one bad row must not take its batch down with it"
        );
        assert_eq!(
            query_i64(
                &conn,
                "SELECT COUNT(*) FROM paths WHERE path IN ('/first', '/last')"
            ),
            2
        );
        assert_eq!(
            query_i64(&conn, "SELECT COUNT(*) FROM paths WHERE path = '/poison'"),
            0
        );
    }

    // The dropped row is kept in the main `errors` table so it can be
    // inspected. It is written on a spawned task, so poll for it.
    let main = revscope::db::open_main_database_read(server.context.clone())
        .await
        .map_err(io::Error::other)?;
    let project_id: i32 = DEMO_PROJECT_ID.parse().expect("demo project id");
    let pattern = format!("%{marker}%");
    let mut recorded = None;
    for _ in 0..100 {
        recorded = main
            .query_opt(
                "SELECT data FROM errors WHERE project_id = $1 AND error_text LIKE $2",
                &[&project_id, &pattern],
            )
            .await
            .map_err(io::Error::other)?
            .map(|row| row.get::<_, String>(0));
        if recorded.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let data = recorded.expect("the dropped row is recorded in errors");
    assert!(
        data.contains("/poison"),
        "the record carries the row: {data}"
    );
    Ok(())
}

#[tokio::test]
async fn a_held_write_slot_defers_the_flush_without_losing_rows() -> io::Result<()> {
    let mut server = TestServer::spawn("flush-write-busy").await?;
    let held = revscope::db::open_project_database_write(server.context.as_ref(), DEMO_PROJECT_ID)
        .map_err(io::Error::other)?;

    // 1000 pageviews reach the flush threshold. With the write slot held the
    // threshold flush finds it busy and keeps the rows queued.
    for batch_idx in 0..50 {
        let items: Vec<TrackHit> = (0..20)
            .map(|item_idx| {
                let n = batch_idx * 20 + item_idx + 1;
                TrackHit::pageview(DEMO_PROJECT_NAME, &format!("/p{n}"), &format!("{n:024x}"))
            })
            .collect();
        let response = server
            .post_ingest(&batch_form(DEMO_PROJECT_NAME, &items), &[])
            .await?;
        assert_eq!(response.status_code(), 200, "{}", response.body_str());
    }
    {
        let conn = server.project_db(DEMO_PROJECT_ID);
        assert_eq!(
            count_rows(&conn, "pageviews"),
            0,
            "nothing is written while another writer holds the slot"
        );
    }

    // The drain waits for the slot instead of skipping, so releasing it a
    // little later still gets every row written, once.
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        drop(held);
    });
    server.flush().await;
    releaser.join().expect("release the write slot");

    let conn = server.project_db(DEMO_PROJECT_ID);
    assert_eq!(count_rows(&conn, "pageviews"), 1000);
    assert_eq!(
        query_i64(&conn, "SELECT COUNT(DISTINCT timestamp_id) FROM pageviews"),
        1000
    );
    Ok(())
}
