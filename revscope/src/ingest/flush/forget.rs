//! Deleting a visitor's data when the tracker asks to forget them.

use fxhash::FxHashMap;
use rusqlite::OptionalExtension;
use rusqlite::params;

use crate::db::ProjectConnection;
use crate::ingest::batch::BatchRow;
use crate::ingest::error::PersistError;
use crate::ingest::rows::ForgetRow;

use super::sessions::resolve_current_session;
use super::{FlushError, blame};

pub(super) fn apply_forget_requests(
    conn: &ProjectConnection,
    forgets: &[ForgetRow],
) -> Result<(), FlushError> {
    // Session to erase -> the forget row that asked for it, so a failed erasure
    // is blamed on that row.
    let mut session_ids: FxHashMap<i64, BatchRow> = FxHashMap::default();

    for (index, forget) in forgets.iter().enumerate() {
        let row = BatchRow::Forget(index);
        let user_hash = forget.user_hash.to_be_bytes();
        let user_session = conn
            .query_row(
                "SELECT visitor_timestamp_id
                 FROM user_sessions
                 WHERE user_hash = ?1",
                params![user_hash],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(blame(row))?;
        if let Some(session_id) = user_session {
            session_ids.insert(session_id, row);
        }

        if let Some(client_page_id) = forget.client_page_id.as_ref() {
            let page_session = conn
                .query_row(
                    "SELECT pageviews.visitor_timestamp_id
                     FROM client_page_ids_map
                     JOIN pageviews
                       ON pageviews.timestamp_id = client_page_ids_map.visitor_timestamp_id
                     WHERE client_page_ids_map.page_id = ?1
                     LIMIT 1",
                    params![client_page_id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .map_err(blame(row))?;
            if let Some(session_id) = page_session {
                session_ids.insert(session_id, row);
            }
        }

        if let Some(session_id) = forget.session_id {
            let current_session = resolve_current_session(conn, session_id)
                .map_err(blame(row))?
                .unwrap_or(session_id);
            // Require the visitor fingerprint's user_sessions row to map to the
            // requested session. Bare `s` is enumerable (nanosecond timestamp
            // ids), so it must not be a standalone delete capability.
            if user_session == Some(current_session) {
                session_ids.insert(current_session, row);
            }
        }
    }

    for (session_id, row) in session_ids {
        delete_session_data(conn, session_id).map_err(blame(row))?;
    }
    Ok(())
}

pub(super) fn delete_session_data(
    conn: &ProjectConnection,
    visitor_timestamp_id: i64,
) -> Result<(), PersistError> {
    // Provider records are retained, but must no longer identify or attribute
    // revenue to the erased analytics session.
    for table in ["payments", "subscriptions"] {
        conn.execute(
            &format!(
                "UPDATE {table}
                 SET visitor_timestamp_id = CASE
                        WHEN visitor_timestamp_id = ?1 THEN NULL
                        ELSE visitor_timestamp_id
                     END,
                     attributed_visitor_timestamp_id = CASE
                        WHEN attributed_visitor_timestamp_id = ?1 THEN NULL
                        ELSE attributed_visitor_timestamp_id
                     END
                 WHERE visitor_timestamp_id = ?1
                    OR attributed_visitor_timestamp_id = ?1"
            ),
            params![visitor_timestamp_id],
        )?;
    }

    conn.execute(
        "DELETE FROM events
         WHERE page_timestamp_id IN (
            SELECT timestamp_id
            FROM pageviews
            WHERE visitor_timestamp_id = ?1
         )",
        params![visitor_timestamp_id],
    )?;
    conn.execute(
        "DELETE FROM metadata
         WHERE visitor_timestamp_id = ?1
            OR page_timestamp_id IN (
                SELECT timestamp_id
                FROM pageviews
                WHERE visitor_timestamp_id = ?1
            )",
        params![visitor_timestamp_id],
    )?;
    conn.execute(
        "DELETE FROM client_page_ids_map
         WHERE visitor_timestamp_id IN (
            SELECT timestamp_id
            FROM pageviews
            WHERE visitor_timestamp_id = ?1
         )",
        params![visitor_timestamp_id],
    )?;
    conn.execute(
        "DELETE FROM user_sessions WHERE visitor_timestamp_id = ?1",
        params![visitor_timestamp_id],
    )?;
    conn.execute(
        "DELETE FROM landings WHERE visitor_timestamp_id = ?1",
        params![visitor_timestamp_id],
    )?;
    conn.execute(
        "DELETE FROM pageviews WHERE visitor_timestamp_id = ?1",
        params![visitor_timestamp_id],
    )?;
    Ok(())
}
