//! Session metadata, and merging the sessions it shows to be one visitor.

use fxhash::FxHashSet;
use rusqlite::OptionalExtension;
use rusqlite::params;
use serde_json::Value;

use crate::db::ProjectConnection;
use crate::ingest::batch::BatchRow;
use crate::ingest::error::PersistError;

pub(super) const NANOS_PER_DAY: i64 = 86_400_000_000_000;
pub(super) const CLIENT_PAGE_IDS_MAP_TTL_DAYS: i64 = 7;
pub(super) const MERGE_METADATA_KEYS: [&str; 3] = ["attribution_key", "user_id", "user_email"];

#[derive(Clone, Copy, Eq, PartialEq, Hash)]
pub(super) struct MetadataMergeCandidate {
    pub(super) visitor_timestamp_id: i64,
    pub(super) key: &'static str,
}

/// One metadata upsert, ordered by enqueue order across pageview and metadata
/// hits, and tagged with the batch row it came from.
pub(super) struct MetadataWrite<'a> {
    pub(super) batch_order: usize,
    pub(super) row: BatchRow,
    pub(super) visitor_timestamp_id: i64,
    pub(super) page_timestamp_id: i64,
    pub(super) metadata_json: &'a str,
}

pub(super) fn upsert_metadata_rows(
    conn: &ProjectConnection,
    visitor_timestamp_id: i64,
    page_timestamp_id: i64,
    metadata_json: Option<&str>,
    merge_candidates: &mut Vec<MetadataMergeCandidate>,
) -> Result<(), PersistError> {
    let Some(metadata_json) = metadata_json else {
        return Ok(());
    };
    let Value::Object(metadata) = serde_json::from_str::<Value>(metadata_json)? else {
        return Ok(());
    };
    for (key, value) in metadata {
        let value = metadata_value_text(&value);
        // Empty string means "delete this key" rather than storing "".
        if value.is_empty() {
            conn.execute(
                "DELETE FROM metadata
                 WHERE visitor_timestamp_id = ?1
                   AND key = ?2",
                params![visitor_timestamp_id, key],
            )?;
            continue;
        }
        if let Some(key) = merge_metadata_key(&key) {
            merge_candidates.push(MetadataMergeCandidate {
                visitor_timestamp_id,
                key,
            });
        }
        conn.execute(
            "INSERT INTO metadata (visitor_timestamp_id, key, value, page_timestamp_id)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(visitor_timestamp_id, key) DO UPDATE SET
                value = excluded.value,
                page_timestamp_id = excluded.page_timestamp_id",
            params![visitor_timestamp_id, key, value, page_timestamp_id],
        )?;
    }
    Ok(())
}

pub(super) fn merge_metadata_key(key: &str) -> Option<&'static str> {
    MERGE_METADATA_KEYS
        .iter()
        .copied()
        .find(|candidate| *candidate == key)
}

pub(super) fn apply_session_merges(
    conn: &ProjectConnection,
    candidates: &[MetadataMergeCandidate],
) -> Result<(), PersistError> {
    if candidates.is_empty() {
        return Ok(());
    }

    let mut seen_candidates: FxHashSet<MetadataMergeCandidate> = FxHashSet::default();
    let mut ordered_candidates = Vec::new();
    for &candidate in candidates {
        if seen_candidates.insert(candidate) {
            ordered_candidates.push(candidate);
        }
    }

    let mut seen_sessions: FxHashSet<i64> = FxHashSet::default();
    let mut sessions = Vec::new();
    for candidate in &ordered_candidates {
        if seen_sessions.insert(candidate.visitor_timestamp_id) {
            sessions.push(candidate.visitor_timestamp_id);
        }
    }

    for session in sessions {
        let Some(current_session) = resolve_current_session(conn, session)? else {
            continue;
        };
        if current_session != session {
            continue;
        }

        for key in MERGE_METADATA_KEYS {
            if !ordered_candidates
                .iter()
                .any(|candidate| candidate.visitor_timestamp_id == session && candidate.key == key)
            {
                continue;
            }
            let Some(value) = session_metadata_value(conn, session, key)? else {
                continue;
            };
            let Some(original_session) =
                find_original_metadata_session(conn, key, &value, session)?
            else {
                continue;
            };
            merge_session(conn, session, original_session)?;
            break;
        }
    }

    Ok(())
}

pub(super) fn resolve_current_session(
    conn: &ProjectConnection,
    timestamp_id: i64,
) -> Result<Option<i64>, rusqlite::Error> {
    conn.query_row(
        "SELECT visitor_timestamp_id
         FROM pageviews
         WHERE timestamp_id = ?1
         LIMIT 1",
        params![timestamp_id],
        |row| row.get::<_, i64>(0),
    )
    .optional()
}

pub(super) fn session_metadata_value(
    conn: &ProjectConnection,
    visitor_timestamp_id: i64,
    key: &str,
) -> Result<Option<String>, rusqlite::Error> {
    Ok(conn
        .query_row(
            "SELECT value
             FROM metadata
             WHERE visitor_timestamp_id = ?1
               AND key = ?2",
            params![visitor_timestamp_id, key],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty()))
}

pub(super) fn find_original_metadata_session(
    conn: &ProjectConnection,
    key: &str,
    value: &str,
    visitor_timestamp_id: i64,
) -> Result<Option<i64>, rusqlite::Error> {
    conn.query_row(
        "SELECT MIN(visitor_timestamp_id)
         FROM metadata
         WHERE key = ?1
           AND value = ?2 COLLATE NOCASE
           AND visitor_timestamp_id < ?3",
        params![key, value, visitor_timestamp_id],
        |row| row.get::<_, Option<i64>>(0),
    )
}

pub(super) fn merge_session(
    conn: &ProjectConnection,
    from_visitor_timestamp_id: i64,
    into_visitor_timestamp_id: i64,
) -> Result<(), PersistError> {
    if from_visitor_timestamp_id == into_visitor_timestamp_id {
        return Ok(());
    }

    merge_metadata_rows(conn, from_visitor_timestamp_id, into_visitor_timestamp_id)?;
    conn.execute(
        "UPDATE pageviews
         SET visitor_timestamp_id = ?2
         WHERE visitor_timestamp_id = ?1",
        params![from_visitor_timestamp_id, into_visitor_timestamp_id],
    )?;
    merge_landing_rows(conn, from_visitor_timestamp_id, into_visitor_timestamp_id)?;
    conn.execute(
        "UPDATE user_sessions
         SET visitor_timestamp_id = ?2
         WHERE visitor_timestamp_id = ?1",
        params![from_visitor_timestamp_id, into_visitor_timestamp_id],
    )?;
    remap_payment_sessions(conn, from_visitor_timestamp_id, into_visitor_timestamp_id)?;
    remap_subscription_sessions(conn, from_visitor_timestamp_id, into_visitor_timestamp_id)?;
    Ok(())
}

pub(super) fn merge_metadata_rows(
    conn: &ProjectConnection,
    from_visitor_timestamp_id: i64,
    into_visitor_timestamp_id: i64,
) -> Result<(), PersistError> {
    let rows = conn.with_query(
        "SELECT key, value, page_timestamp_id
             FROM metadata
             WHERE visitor_timestamp_id = ?1",
        params![from_visitor_timestamp_id],
        |mut rows| {
            let mut metadata_rows = Vec::new();
            while let Some(row) = rows.next()? {
                metadata_rows.push((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ));
            }
            Ok(metadata_rows)
        },
    )?;

    for (key, value, page_timestamp_id) in rows {
        conn.execute(
            "INSERT INTO metadata (visitor_timestamp_id, key, value, page_timestamp_id)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(visitor_timestamp_id, key) DO UPDATE SET
                value = excluded.value,
                page_timestamp_id = excluded.page_timestamp_id",
            params![into_visitor_timestamp_id, key, value, page_timestamp_id],
        )?;
    }
    conn.execute(
        "DELETE FROM metadata WHERE visitor_timestamp_id = ?1",
        params![from_visitor_timestamp_id],
    )?;
    Ok(())
}

pub(super) fn merge_landing_rows(
    conn: &ProjectConnection,
    from_visitor_timestamp_id: i64,
    into_visitor_timestamp_id: i64,
) -> Result<(), PersistError> {
    conn.execute(
        "UPDATE landings
         SET session_page_count = session_page_count + COALESCE(
                (SELECT session_page_count FROM landings WHERE visitor_timestamp_id = ?1),
                0
             ),
             session_duration_seconds = session_duration_seconds + COALESCE(
                (SELECT session_duration_seconds FROM landings WHERE visitor_timestamp_id = ?1),
                0
             )
         WHERE visitor_timestamp_id = ?2",
        params![from_visitor_timestamp_id, into_visitor_timestamp_id],
    )?;
    conn.execute(
        "DELETE FROM landings WHERE visitor_timestamp_id = ?1",
        params![from_visitor_timestamp_id],
    )?;
    Ok(())
}

pub(super) fn remap_payment_sessions(
    conn: &ProjectConnection,
    from_visitor_timestamp_id: i64,
    into_visitor_timestamp_id: i64,
) -> Result<(), PersistError> {
    conn.execute(
        "UPDATE payments
         SET visitor_timestamp_id = ?2
         WHERE visitor_timestamp_id = ?1",
        params![from_visitor_timestamp_id, into_visitor_timestamp_id],
    )?;
    conn.execute(
        "UPDATE payments
         SET attributed_visitor_timestamp_id = ?2
         WHERE attributed_visitor_timestamp_id = ?1",
        params![from_visitor_timestamp_id, into_visitor_timestamp_id],
    )?;
    Ok(())
}

pub(super) fn remap_subscription_sessions(
    conn: &ProjectConnection,
    from_visitor_timestamp_id: i64,
    into_visitor_timestamp_id: i64,
) -> Result<(), PersistError> {
    conn.execute(
        "UPDATE subscriptions
         SET visitor_timestamp_id = ?2
         WHERE visitor_timestamp_id = ?1",
        params![from_visitor_timestamp_id, into_visitor_timestamp_id],
    )?;
    conn.execute(
        "UPDATE subscriptions
         SET attributed_visitor_timestamp_id = ?2
         WHERE attributed_visitor_timestamp_id = ?1",
        params![from_visitor_timestamp_id, into_visitor_timestamp_id],
    )?;
    Ok(())
}

pub(super) fn metadata_value_text(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Array(_) | Value::Object(_) => serde_json::to_string(value).unwrap_or_default(),
        Value::Null => "null".to_string(),
    }
}

pub(super) fn upsert_user_session(
    conn: &ProjectConnection,
    user_hash: &[u8],
    visitor_timestamp_id: i64,
    expires_after_days: i64,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO user_sessions (
            user_hash, visitor_timestamp_id, expires_after_days
        ) VALUES (?1, ?2, ?3)
        ON CONFLICT(user_hash) DO UPDATE SET
            visitor_timestamp_id = excluded.visitor_timestamp_id,
            expires_after_days = excluded.expires_after_days",
        params![user_hash, visitor_timestamp_id, expires_after_days],
    )?;
    Ok(())
}

pub(super) fn purge_stale_user_sessions(
    conn: &ProjectConnection,
    now_timestamp: i64,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "DELETE FROM user_sessions
         WHERE EXISTS (
            SELECT 1
            FROM pageviews
            WHERE pageviews.timestamp_id = user_sessions.visitor_timestamp_id
              AND pageviews.timestamp_id < (?1 - (user_sessions.expires_after_days * ?2))
         )",
        params![now_timestamp, NANOS_PER_DAY],
    )?;
    Ok(())
}

pub(super) fn purge_stale_client_page_ids_map(
    conn: &ProjectConnection,
    now_timestamp: i64,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "DELETE FROM client_page_ids_map
         WHERE visitor_timestamp_id < (?1 - (?2 * ?3))",
        params![now_timestamp, CLIENT_PAGE_IDS_MAP_TTL_DAYS, NANOS_PER_DAY],
    )?;
    Ok(())
}
