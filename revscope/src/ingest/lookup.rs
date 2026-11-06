use rusqlite::{OptionalExtension, params};

use super::error::PersistError;
use super::map::IngestBatch;
use super::normalize::positive_timestamp_id;
use super::wire::TrackRequest;
use crate::db::{ClientPageId, ProjectConnection, open_project_database_read};
use crate::server_context::ServerContext;
use crate::util::non_empty;

/// Lazily open (and reuse) a read-only project connection for enqueue-time
/// lookups that miss the in-memory batch maps.
pub(super) fn get_read_connection<'a>(
    server_context: &ServerContext,
    project_id: &str,
    slot: &'a mut Option<ProjectConnection>,
) -> Result<&'a ProjectConnection, PersistError> {
    if slot.is_none() {
        let conn = open_project_database_read(server_context, project_id)?;
        *slot = Some(conn);
    }
    Ok(slot.as_ref().expect("connection just stored"))
}

/// Resolve the visitor (session) timestamp id for a pageview, preferring the
/// in-batch maps before querying the database.
///
/// A stale or unknown session cookie (`s`) must not reject the pageview: if `s`
/// is missing from both the current batch and the project DB, fall back to the
/// user-hash session or mint a new landing session. Otherwise the client keeps
/// a `pageId` the server never recorded and later events become orphans.
pub(super) fn resolve_enqueue_visitor_timestamp(
    server_context: &ServerContext,
    project_id: &str,
    batch: &IngestBatch,
    read_conn: &mut Option<ProjectConnection>,
    wire: &TrackRequest,
    user_hash: u128,
    timestamp_id: i64,
) -> Result<i64, PersistError> {
    if let Some(session_id) = positive_timestamp_id(wire.session_id)?
        && batch.pageview_idx_by_ts.contains_key(&session_id)
    {
        return Ok(session_id);
    }
    // Fast path when there is no session cookie: reuse this user's session from
    // the current batch before opening a DB connection.
    if wire.session_id.is_none()
        && let Some(&visitor_timestamp_id) = batch.visitor_by_user_hash.get(&user_hash)
    {
        return Ok(visitor_timestamp_id);
    }

    let conn = get_read_connection(server_context, project_id, read_conn)?;
    // Always load the user-hash session so a stale/unknown `s` can fall back to
    // it (batch first, then DB) instead of InvalidPayload.
    let existing_session = match batch.visitor_by_user_hash.get(&user_hash) {
        Some(&visitor_timestamp_id) => Some(UserSessionState {
            visitor_timestamp_id,
        }),
        None => fetch_user_session_state(conn, &user_hash.to_be_bytes())?,
    };
    resolve_visitor_timestamp_id(
        conn,
        wire.session_id,
        existing_session.as_ref(),
        timestamp_id,
    )
}

/// Resolve the pageview a non-pageview hit refers to via client `pg`.
/// Prefer the in-batch maps before querying the database.
pub(super) fn resolve_enqueue_pageview_ref(
    server_context: &ServerContext,
    project_id: &str,
    batch: &IngestBatch,
    read_conn: &mut Option<ProjectConnection>,
    wire: &TrackRequest,
) -> Result<Option<(i64, i64)>, PersistError> {
    let Some(page_id_hex) = non_empty(wire.page_id.as_deref()) else {
        return Ok(None);
    };
    let client_page_id =
        ClientPageId::from_hex(page_id_hex).map_err(|_| PersistError::InvalidPayload)?;
    if let Some(&reference) = batch.pageview_idx_by_client_id.get(&client_page_id) {
        return Ok(Some(batch.pageview_ids(reference)));
    }
    let conn = get_read_connection(server_context, project_id, read_conn)?;
    Ok(fetch_pageview_session_target(conn, &client_page_id)?
        .map(|target| (target.timestamp_id, target.visitor_timestamp_id)))
}

#[derive(Debug)]
struct UserSessionState {
    visitor_timestamp_id: i64,
}

#[derive(Debug)]
struct PageviewSessionTarget {
    timestamp_id: i64,
    visitor_timestamp_id: i64,
}

fn fetch_user_session_state(
    conn: &ProjectConnection,
    user_hash: &[u8],
) -> Result<Option<UserSessionState>, rusqlite::Error> {
    conn.query_row(
        "SELECT visitor_timestamp_id FROM user_sessions WHERE user_hash = ?1",
        params![user_hash],
        |row| {
            Ok(UserSessionState {
                visitor_timestamp_id: row.get::<_, i64>(0)?,
            })
        },
    )
    .optional()
}

fn fetch_pageview_session_target(
    conn: &ProjectConnection,
    page_id: &ClientPageId,
) -> Result<Option<PageviewSessionTarget>, rusqlite::Error> {
    conn.query_row(
        "SELECT pageviews.timestamp_id, pageviews.visitor_timestamp_id
         FROM client_page_ids_map
         JOIN pageviews ON pageviews.timestamp_id = client_page_ids_map.visitor_timestamp_id
         WHERE client_page_ids_map.page_id = ?1
         LIMIT 1",
        params![page_id],
        |row| {
            Ok(PageviewSessionTarget {
                timestamp_id: row.get::<_, i64>(0)?,
                visitor_timestamp_id: row.get::<_, i64>(1)?,
            })
        },
    )
    .optional()
}

fn resolve_visitor_timestamp_id(
    conn: &ProjectConnection,
    payload_session_id: Option<i64>,
    existing_session: Option<&UserSessionState>,
    current_timestamp_id: i64,
) -> Result<i64, PersistError> {
    if let Some(session_id) = positive_timestamp_id(payload_session_id)?
        && let Some(resolved) = resolve_timestamp_id(conn, session_id)?
    {
        return Ok(resolved);
    }
    // Missing, stale, or unknown session id: reuse the user-hash session or mint
    // a new landing session rather than rejecting the pageview.
    if let Some(existing_session) = existing_session {
        return Ok(existing_session.visitor_timestamp_id);
    }
    Ok(current_timestamp_id)
}

fn resolve_timestamp_id(
    conn: &ProjectConnection,
    timestamp_id: i64,
) -> Result<Option<i64>, rusqlite::Error> {
    conn.query_row(
        "SELECT visitor_timestamp_id FROM pageviews WHERE timestamp_id = ?1 LIMIT 1",
        params![timestamp_id],
        |row| row.get::<_, i64>(0),
    )
    .optional()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::project::in_memory_test_connection;
    use crate::test_utils::open_migrated_db;

    #[test]
    fn resolves_visitor_timestamp_id_in_priority_order() {
        let conn = open_migrated_db();
        conn.execute(
            "INSERT INTO pageviews (
                timestamp_id, visitor_timestamp_id, previous_timestamp_id, path_id,
                duration_seconds, scrolled_percent
             ) VALUES (2_222, 2_222, NULL, 1, NULL, NULL)",
            [],
        )
        .expect("insert pageview");
        conn.execute(
            "INSERT INTO client_page_ids_map (visitor_timestamp_id, page_id)
             VALUES (?1, ?2)",
            params![
                2_222_i64,
                ClientPageId::from_hex("222222222222222222222222").expect("mapped page")
            ],
        )
        .expect("insert map");
        let session = UserSessionState {
            visitor_timestamp_id: 1_111,
        };
        let current_timestamp_id = 3_333;

        assert_eq!(
            resolve_visitor_timestamp_id(&conn, Some(2_222), Some(&session), current_timestamp_id)
                .expect("payload session"),
            2_222
        );
        assert_eq!(
            resolve_visitor_timestamp_id(&conn, None, Some(&session), current_timestamp_id)
                .expect("session fallback"),
            1_111
        );
        assert_eq!(
            resolve_visitor_timestamp_id(&conn, None, None, current_timestamp_id)
                .expect("current page fallback"),
            current_timestamp_id
        );
        // Unknown/stale session cookie falls back to user-hash session.
        assert_eq!(
            resolve_visitor_timestamp_id(&conn, Some(9_999), Some(&session), current_timestamp_id)
                .expect("stale session -> user hash"),
            1_111
        );
        // Unknown/stale session with no user-hash session mints a landing.
        assert_eq!(
            resolve_visitor_timestamp_id(&conn, Some(9_999), None, current_timestamp_id)
                .expect("stale session -> mint landing"),
            current_timestamp_id
        );
    }

    #[test]
    fn payload_session_resolves_to_merged_session() {
        let conn = open_migrated_db();
        conn.execute(
            "INSERT INTO pageviews (
                timestamp_id, visitor_timestamp_id, previous_timestamp_id, path_id,
                duration_seconds, scrolled_percent
             ) VALUES (10_000, 9_000, NULL, 1, NULL, NULL)",
            [],
        )
        .expect("insert merged pageview");

        assert_eq!(
            resolve_visitor_timestamp_id(&conn, Some(10_000), None, 11_000)
                .expect("payload session"),
            9_000
        );
    }

    #[test]
    fn rejects_invalid_session_id_value() {
        let conn = in_memory_test_connection();
        assert!(matches!(
            resolve_visitor_timestamp_id(&conn, Some(0), None, 1),
            Err(PersistError::InvalidPayload)
        ));
    }
}
