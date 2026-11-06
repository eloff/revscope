use std::sync::Arc;

use rusqlite::{OptionalExtension, Row, ToSql, params, params_from_iter};
use serde_json::Value;

use super::sync::SyncResult;
use crate::db::{
    ClientPageId, ProjectCache, ProjectConnection, open_main_database_read,
    open_project_database_write_for,
};
use crate::server_context::ServerContext;
use crate::util::{
    NANOS_PER_SECOND, VISITOR_TIMESTAMP_ID_HEX_LEN, parse_visitor_timestamp_id_hex,
    seconds_since_epoch,
};

const FULL_SYNC_LOOKBACK_SECONDS: i64 = 90 * 24 * 60 * 60;
const SUCCESS_PATH_WINDOW_NANOS: i64 = 15 * 60 * NANOS_PER_SECOND;

pub(super) async fn attribute_full_sync(
    server_context: Arc<ServerContext>,
    project: &Arc<ProjectCache>,
    provider: &str,
) -> SyncResult<()> {
    let payment_success_path =
        load_payment_success_path(server_context.clone(), project.id()).await?;
    let cutoff = i64::from(seconds_since_epoch()).saturating_sub(FULL_SYNC_LOOKBACK_SECONDS);
    let conn = open_project_database_write_for(&server_context, project)?;

    for payment_id in unattributed_payment_ids(&conn, provider, cutoff)? {
        let Some(row) = load_payment_attribution_row(&conn, payment_id)? else {
            continue;
        };
        if attribute_entity(&conn, &row, row.paid_at, SessionLookup::ByKeys)? {
            continue;
        }
        if let Some(path) = payment_success_path.as_deref()
            && matches!(row.payment_type.as_deref(), Some("single" | "subscription"))
            && attribute_entity(&conn, &row, row.paid_at, SessionLookup::BySuccessPath(path))?
        {
            continue;
        }
    }

    for subscription_id in unattributed_subscription_ids(&conn, provider, cutoff)? {
        let Some(row) = load_subscription_attribution_row(&conn, subscription_id)? else {
            continue;
        };
        if attribute_entity(&conn, &row, row.started_at, SessionLookup::ByKeys)? {
            continue;
        }
        if let Some(path) = payment_success_path.as_deref()
            && attribute_entity(
                &conn,
                &row,
                row.started_at,
                SessionLookup::BySuccessPath(path),
            )?
        {
            continue;
        }
    }

    Ok(())
}

pub(super) fn attribute_payment_by_provider_id(
    conn: &ProjectConnection,
    provider: &str,
    provider_payment_id: &str,
) -> SyncResult<bool> {
    let Some(payment_id) = conn
        .query_row(
            "SELECT id FROM payments WHERE provider = ?1 AND provider_payment_id = ?2",
            params![provider, provider_payment_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
    else {
        return Ok(false);
    };
    let Some(row) = load_payment_attribution_row(conn, payment_id)? else {
        return Ok(false);
    };
    attribute_entity(conn, &row, row.paid_at, SessionLookup::ByKeys)
}

pub(super) fn attribute_subscription_by_id(
    conn: &ProjectConnection,
    subscription_id: i64,
) -> SyncResult<bool> {
    let Some(row) = load_subscription_attribution_row(conn, subscription_id)? else {
        return Ok(false);
    };
    attribute_entity(conn, &row, row.started_at, SessionLookup::ByKeys)
}

enum SessionLookup<'a> {
    ByKeys,
    BySuccessPath(&'a str),
}

fn attribute_entity(
    conn: &ProjectConnection,
    row: &AttributionFields,
    event_at: Option<i64>,
    lookup: SessionLookup<'_>,
) -> SyncResult<bool> {
    if let Some(visitor_timestamp_id) = row.visitor_timestamp_id {
        return apply_attribution(conn, row, visitor_timestamp_id);
    }
    let Some(event_at) = event_at else {
        return Ok(false);
    };
    let visitor_timestamp_id = match lookup {
        SessionLookup::ByKeys => {
            let metadata_candidates = metadata_id_candidates(row);
            match find_landing_by_metadata_ids(conn, &metadata_candidates, event_at)? {
                Some(visitor_timestamp_id) => Some(visitor_timestamp_id),
                None => {
                    let candidates = attribution_candidates(row);
                    find_landing_by_keys(conn, &candidates, event_at)?
                }
            }
        }
        SessionLookup::BySuccessPath(path) => find_success_path_session(conn, path, event_at)?,
    };
    let Some(visitor_timestamp_id) = visitor_timestamp_id else {
        return Ok(false);
    };
    apply_attribution(conn, row, visitor_timestamp_id)
}

async fn load_payment_success_path(
    server_context: Arc<ServerContext>,
    project_id: i32,
) -> SyncResult<Option<String>> {
    let conn = open_main_database_read(server_context).await?;
    let row = conn
        .query_opt(
            "SELECT payment_success_path FROM projects WHERE id = $1",
            &[&project_id],
        )
        .await?;
    Ok(row.and_then(|row| row.get::<_, Option<String>>(0)))
}

fn unattributed_payment_ids(
    conn: &ProjectConnection,
    provider: &str,
    cutoff: i64,
) -> SyncResult<Vec<i64>> {
    conn.with_query(
        "SELECT id
         FROM payments
         WHERE provider = ?1
           AND visitor_timestamp_id IS NULL
           AND paid_at >= ?2
         ORDER BY paid_at ASC, id ASC",
        params![provider, cutoff],
        |mut rows| {
            let mut ids = Vec::new();
            while let Some(row) = rows.next()? {
                ids.push(row.get::<_, i64>(0)?);
            }
            Ok(ids)
        },
    )
    .map_err(Into::into)
}

fn unattributed_subscription_ids(
    conn: &ProjectConnection,
    provider: &str,
    cutoff: i64,
) -> SyncResult<Vec<i64>> {
    conn.with_query(
        "SELECT id
         FROM subscriptions
         WHERE provider = ?1
           AND visitor_timestamp_id IS NULL
           AND started_at >= ?2
         ORDER BY started_at ASC, id ASC",
        params![provider, cutoff],
        |mut rows| {
            let mut ids = Vec::new();
            while let Some(row) = rows.next()? {
                ids.push(row.get::<_, i64>(0)?);
            }
            Ok(ids)
        },
    )
    .map_err(Into::into)
}

#[derive(Debug)]
struct AttributionFields {
    payment_id: Option<i64>,
    subscription_id: Option<i64>,
    visitor_timestamp_id: Option<i64>,
    attributed_visitor_timestamp_id: Option<i64>,
    customer_email: Option<String>,
    order_email: Option<String>,
    provider_checkout_id: Option<String>,
    provider_order_id: Option<String>,
    provider_payment_id: Option<String>,
    provider_subscription_id: Option<String>,
    payment_type: Option<String>,
    paid_at: Option<i64>,
    started_at: Option<i64>,
    payment_metadata: Option<String>,
    subscription_metadata: Option<String>,
}

enum AttributionRowSource {
    Payment(i64),
    Subscription(i64),
}

fn load_payment_attribution_row(
    conn: &ProjectConnection,
    payment_id: i64,
) -> SyncResult<Option<AttributionFields>> {
    load_attribution_row(conn, AttributionRowSource::Payment(payment_id))
}

fn load_subscription_attribution_row(
    conn: &ProjectConnection,
    subscription_id: i64,
) -> SyncResult<Option<AttributionFields>> {
    load_attribution_row(conn, AttributionRowSource::Subscription(subscription_id))
}

fn load_attribution_row(
    conn: &ProjectConnection,
    source: AttributionRowSource,
) -> SyncResult<Option<AttributionFields>> {
    const PAYMENT_SQL: &str = "SELECT
            p.id,
            COALESCE(p.subscription_id, s.id),
            COALESCE(p.visitor_timestamp_id, s.visitor_timestamp_id),
            COALESCE(p.attributed_visitor_timestamp_id, s.attributed_visitor_timestamp_id),
            COALESCE(p.customer_email, s.customer_email),
            COALESCE(p.order_email, s.order_email),
            COALESCE(p.provider_checkout_id, s.provider_checkout_id),
            COALESCE(p.provider_order_id, s.provider_order_id),
            p.provider_payment_id,
            s.provider_subscription_id,
            p.payment_type,
            p.paid_at,
            s.started_at,
            p.metadata,
            s.metadata
         FROM payments p
         LEFT JOIN subscriptions s ON s.id = p.subscription_id
         WHERE p.id = ?1";
    const SUBSCRIPTION_SQL: &str = "SELECT
            p.id,
            COALESCE(p.subscription_id, s.id),
            COALESCE(p.visitor_timestamp_id, s.visitor_timestamp_id),
            COALESCE(p.attributed_visitor_timestamp_id, s.attributed_visitor_timestamp_id),
            COALESCE(p.customer_email, s.customer_email),
            COALESCE(p.order_email, s.order_email),
            COALESCE(p.provider_checkout_id, s.provider_checkout_id),
            COALESCE(p.provider_order_id, s.provider_order_id),
            p.provider_payment_id,
            s.provider_subscription_id,
            p.payment_type,
            p.paid_at,
            s.started_at,
            p.metadata,
            s.metadata
         FROM subscriptions s
         LEFT JOIN payments p ON p.id = (
            SELECT first_payment.id
            FROM payments first_payment
            WHERE first_payment.subscription_id = s.id
            ORDER BY first_payment.paid_at ASC, first_payment.id ASC
            LIMIT 1
         )
         WHERE s.id = ?1";
    let (id, sql) = match source {
        AttributionRowSource::Payment(payment_id) => (payment_id, PAYMENT_SQL),
        AttributionRowSource::Subscription(subscription_id) => (subscription_id, SUBSCRIPTION_SQL),
    };
    Ok(conn
        .query_row(sql, params![id], attribution_fields_from_row)
        .optional()?)
}

fn attribution_fields_from_row(row: &Row<'_>) -> rusqlite::Result<AttributionFields> {
    Ok(AttributionFields {
        payment_id: row.get(0)?,
        subscription_id: row.get(1)?,
        visitor_timestamp_id: row.get(2)?,
        attributed_visitor_timestamp_id: row.get(3)?,
        customer_email: row.get(4)?,
        order_email: row.get(5)?,
        provider_checkout_id: row.get(6)?,
        provider_order_id: row.get(7)?,
        provider_payment_id: row.get(8)?,
        provider_subscription_id: row.get(9)?,
        payment_type: row.get(10)?,
        paid_at: row.get(11)?,
        started_at: row.get(12)?,
        payment_metadata: row.get(13)?,
        subscription_metadata: row.get(14)?,
    })
}

fn attribution_candidates(row: &AttributionFields) -> Vec<String> {
    let mut candidates = Vec::new();
    push_candidate(&mut candidates, row.customer_email.as_deref());
    push_candidate(&mut candidates, row.order_email.as_deref());
    push_candidate(&mut candidates, row.provider_checkout_id.as_deref());
    push_candidate(&mut candidates, row.provider_order_id.as_deref());
    push_candidate(&mut candidates, row.provider_payment_id.as_deref());
    push_candidate(&mut candidates, row.provider_subscription_id.as_deref());
    push_metadata_candidates(&mut candidates, row.payment_metadata.as_deref());
    push_metadata_candidates(&mut candidates, row.subscription_metadata.as_deref());
    candidates
}

fn metadata_id_candidates(row: &AttributionFields) -> Vec<String> {
    let mut candidates = Vec::new();
    push_metadata_string_candidates(&mut candidates, row.payment_metadata.as_deref());
    push_metadata_string_candidates(&mut candidates, row.subscription_metadata.as_deref());
    candidates
}

fn push_candidate(candidates: &mut Vec<String>, value: Option<&str>) {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    let value = value.to_ascii_lowercase();
    // This is O(N^2) but the N is finite and small (see attribution_candidates)
    if !candidates.iter().any(|candidate| candidate == &value) {
        candidates.push(value);
    }
}

fn push_metadata_candidates(candidates: &mut Vec<String>, metadata: Option<&str>) {
    let Some(metadata) = metadata else {
        return;
    };
    let Ok(value) = serde_json::from_str::<Value>(metadata) else {
        return;
    };
    push_json_scalar_candidates(candidates, &value);
}

fn push_metadata_string_candidates(candidates: &mut Vec<String>, metadata: Option<&str>) {
    let Some(metadata) = metadata else {
        return;
    };
    let Ok(value) = serde_json::from_str::<Value>(metadata) else {
        return;
    };
    push_json_string_candidates(candidates, &value);
}

fn push_json_string_candidates(candidates: &mut Vec<String>, value: &Value) {
    match value {
        Value::String(value) => push_candidate(candidates, Some(value)),
        Value::Array(values) => {
            for value in values {
                push_json_string_candidates(candidates, value);
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                push_json_string_candidates(candidates, value);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn push_json_scalar_candidates(candidates: &mut Vec<String>, value: &Value) {
    match value {
        Value::String(value) => push_candidate(candidates, Some(value)),
        Value::Number(value) => push_candidate(candidates, Some(&value.to_string())),
        Value::Bool(value) => {
            push_candidate(candidates, Some(if *value { "true" } else { "false" }))
        }
        Value::Array(values) => {
            for value in values {
                push_json_scalar_candidates(candidates, value);
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                push_json_scalar_candidates(candidates, value);
            }
        }
        Value::Null => {}
    }
}

fn find_landing_by_metadata_ids(
    conn: &ProjectConnection,
    candidates: &[String],
    event_at: i64,
) -> SyncResult<Option<i64>> {
    let event_timestamp_id = event_timestamp_id(event_at);
    let mut latest_visitor_timestamp_id = None;

    for candidate in candidates {
        let visitor_timestamp_id = match candidate.len() {
            VISITOR_TIMESTAMP_ID_HEX_LEN => {
                let Ok(visitor_timestamp_id) = parse_visitor_timestamp_id_hex(candidate) else {
                    continue;
                };
                conn.query_row(
                    "SELECT visitor_timestamp_id
                     FROM landings
                     WHERE visitor_timestamp_id = ?1
                       AND visitor_timestamp_id < ?2",
                    params![visitor_timestamp_id, event_timestamp_id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?
            }
            ClientPageId::HEX_LEN => {
                let Ok(page_id) = ClientPageId::from_hex(candidate) else {
                    continue;
                };
                conn.query_row(
                    "SELECT pv.visitor_timestamp_id
                     FROM client_page_ids_map page_map
                     JOIN pageviews pv ON pv.timestamp_id = page_map.visitor_timestamp_id
                     JOIN landings l ON l.visitor_timestamp_id = pv.visitor_timestamp_id
                     WHERE page_map.page_id = ?1
                       AND page_map.visitor_timestamp_id < ?2",
                    params![page_id, event_timestamp_id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?
            }
            _ => continue,
        };

        if let Some(visitor_timestamp_id) = visitor_timestamp_id
            && latest_visitor_timestamp_id.is_none_or(|latest| visitor_timestamp_id > latest)
        {
            latest_visitor_timestamp_id = Some(visitor_timestamp_id);
        }
    }

    Ok(latest_visitor_timestamp_id)
}

fn find_landing_by_keys(
    conn: &ProjectConnection,
    candidates: &[String],
    event_at: i64,
) -> SyncResult<Option<i64>> {
    if candidates.is_empty() {
        return Ok(None);
    }
    let event_timestamp_id = event_timestamp_id(event_at);
    let placeholders = (1..=candidates.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let event_param = candidates.len() + 1;
    // `value COLLATE NOCASE IN (...)` (not `IN (...) COLLATE NOCASE`) is required
    // for case-insensitive membership and to use the metadata_value index.
    let sql = format!(
        "SELECT visitor_timestamp_id
         FROM metadata
         WHERE value COLLATE NOCASE IN ({placeholders})
           AND visitor_timestamp_id < ?{event_param}
         ORDER BY visitor_timestamp_id DESC
         LIMIT 1"
    );
    let mut params: Vec<&dyn ToSql> = candidates
        .iter()
        .map(|candidate| candidate as &dyn ToSql)
        .collect();
    params.push(&event_timestamp_id);
    Ok(conn
        .query_row(&sql, params_from_iter(params), |row| row.get::<_, i64>(0))
        .optional()?)
}

fn find_success_path_session(
    conn: &ProjectConnection,
    payment_success_path: &str,
    event_at: i64,
) -> SyncResult<Option<i64>> {
    let event_timestamp_id = event_timestamp_id(event_at);
    let start = event_timestamp_id.saturating_sub(SUCCESS_PATH_WINDOW_NANOS);
    let end = event_timestamp_id.saturating_add(SUCCESS_PATH_WINDOW_NANOS);
    Ok(conn
        .query_row(
            "SELECT pv.visitor_timestamp_id
             FROM pageviews pv
             JOIN paths path ON path.id = pv.path_id
             WHERE path.path = ?1
               AND pv.timestamp_id BETWEEN ?2 AND ?3
               AND NOT EXISTS (
                    SELECT 1 FROM payments used_payment
                    WHERE used_payment.visitor_timestamp_id = pv.visitor_timestamp_id
                      AND used_payment.payment_type IN ('single', 'subscription')
               )
               AND NOT EXISTS (
                    SELECT 1 FROM subscriptions used_subscription
                    WHERE used_subscription.visitor_timestamp_id = pv.visitor_timestamp_id
               )
             ORDER BY
                CASE WHEN ?4 <= pv.timestamp_id THEN 0 ELSE 1 END,
                ABS(pv.timestamp_id - ?4)
             LIMIT 1",
            params![payment_success_path, start, end, event_timestamp_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?)
}

fn apply_attribution(
    conn: &ProjectConnection,
    row: &AttributionFields,
    visitor_timestamp_id: i64,
) -> SyncResult<bool> {
    let attributed_visitor_timestamp_id = match row.attributed_visitor_timestamp_id {
        Some(attributed_visitor_timestamp_id) => attributed_visitor_timestamp_id,
        None => resolve_attributed_session(conn, visitor_timestamp_id)?,
    };
    let mut updated = set_payment_attribution(
        conn,
        row,
        visitor_timestamp_id,
        attributed_visitor_timestamp_id,
    )?;
    if let Some(subscription_id) = row.subscription_id {
        updated |= set_subscription_attribution(
            conn,
            subscription_id,
            visitor_timestamp_id,
            attributed_visitor_timestamp_id,
        )?;
        let payment_ids = subscription_payment_ids_without_attribution(conn, subscription_id)?;
        for payment_id in payment_ids {
            if Some(payment_id) == row.payment_id {
                continue;
            }
            if let Some(payment) = load_payment_attribution_row(conn, payment_id)? {
                updated |= set_payment_attribution(
                    conn,
                    &payment,
                    visitor_timestamp_id,
                    attributed_visitor_timestamp_id,
                )?;
            }
        }
    }
    Ok(updated)
}

fn resolve_attributed_session(
    conn: &ProjectConnection,
    visitor_timestamp_id: i64,
) -> SyncResult<i64> {
    let Some(account_id) = conn
        .query_row(
            "SELECT value
             FROM metadata
             WHERE visitor_timestamp_id = ?1
               AND key = 'account_id'",
            params![visitor_timestamp_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        return Ok(visitor_timestamp_id);
    };

    Ok(conn
        .query_row(
            "SELECT MIN(visitor_timestamp_id)
             FROM metadata
             WHERE key = 'account_id'
               AND value = ?1 COLLATE NOCASE",
            params![account_id],
            |row| row.get::<_, Option<i64>>(0),
        )?
        .unwrap_or(visitor_timestamp_id))
}

fn set_payment_attribution(
    conn: &ProjectConnection,
    row: &AttributionFields,
    visitor_timestamp_id: i64,
    attributed_visitor_timestamp_id: i64,
) -> SyncResult<bool> {
    let (Some(payment_id), Some(paid_at)) = (row.payment_id, row.paid_at) else {
        return Ok(false);
    };
    let (session_duration_seconds, session_page_count) =
        session_metrics_at(conn, visitor_timestamp_id, paid_at)?;
    let updated = conn.execute(
        "UPDATE payments
         SET visitor_timestamp_id = COALESCE(visitor_timestamp_id, ?2),
             attributed_visitor_timestamp_id = COALESCE(attributed_visitor_timestamp_id, ?3),
             session_duration_seconds = ?4,
             session_page_count = ?5
         WHERE id = ?1
           AND (visitor_timestamp_id IS NULL OR attributed_visitor_timestamp_id IS NULL)",
        params![
            payment_id,
            visitor_timestamp_id,
            attributed_visitor_timestamp_id,
            session_duration_seconds,
            session_page_count
        ],
    )?;
    Ok(updated > 0)
}

fn set_subscription_attribution(
    conn: &ProjectConnection,
    subscription_id: i64,
    visitor_timestamp_id: i64,
    attributed_visitor_timestamp_id: i64,
) -> SyncResult<bool> {
    let updated = conn.execute(
        "UPDATE subscriptions
         SET visitor_timestamp_id = COALESCE(visitor_timestamp_id, ?2),
             attributed_visitor_timestamp_id = COALESCE(attributed_visitor_timestamp_id, ?3)
         WHERE id = ?1
           AND (visitor_timestamp_id IS NULL OR attributed_visitor_timestamp_id IS NULL)",
        params![
            subscription_id,
            visitor_timestamp_id,
            attributed_visitor_timestamp_id
        ],
    )?;
    Ok(updated > 0)
}

pub(super) fn propagate_linked_subscription_payment_fields(
    conn: &ProjectConnection,
    subscription_id: i64,
) -> SyncResult<()> {
    conn.execute(
        "UPDATE subscriptions AS s
         SET visitor_timestamp_id = COALESCE(s.visitor_timestamp_id, p.visitor_timestamp_id),
             attributed_visitor_timestamp_id = COALESCE(
                s.attributed_visitor_timestamp_id,
                p.attributed_visitor_timestamp_id
             ),
             provider_checkout_id = COALESCE(s.provider_checkout_id, p.provider_checkout_id),
             provider_order_id = COALESCE(s.provider_order_id, p.provider_order_id),
             provider_customer_id = COALESCE(s.provider_customer_id, p.provider_customer_id),
             customer_email = COALESCE(s.customer_email, p.customer_email),
             order_email = COALESCE(s.order_email, p.order_email)
         FROM (
            SELECT visitor_timestamp_id,
                   attributed_visitor_timestamp_id,
                   provider_checkout_id,
                   provider_order_id,
                   provider_customer_id,
                   customer_email,
                   order_email
            FROM payments
            WHERE subscription_id = ?1
            ORDER BY paid_at ASC, id ASC
            LIMIT 1
         ) AS p
         WHERE s.id = ?1",
        params![subscription_id],
    )?;

    let visitor_timestamp_id = conn
        .query_row(
            "SELECT visitor_timestamp_id FROM subscriptions WHERE id = ?1",
            params![subscription_id],
            |row| row.get::<_, Option<i64>>(0),
        )
        .optional()?
        .flatten();
    if let Some(visitor_timestamp_id) = visitor_timestamp_id
        && let Some(row) = load_subscription_attribution_row(conn, subscription_id)?
    {
        apply_attribution(conn, &row, visitor_timestamp_id)?;
    }

    conn.execute(
        "UPDATE payments
         SET attributed_visitor_timestamp_id = COALESCE(
                attributed_visitor_timestamp_id,
                (SELECT s.attributed_visitor_timestamp_id FROM subscriptions s WHERE s.id = payments.subscription_id)
             ),
             provider_checkout_id = COALESCE(
                provider_checkout_id,
                (SELECT s.provider_checkout_id FROM subscriptions s WHERE s.id = payments.subscription_id)
             ),
             provider_order_id = COALESCE(
                provider_order_id,
                (SELECT s.provider_order_id FROM subscriptions s WHERE s.id = payments.subscription_id)
             ),
             provider_customer_id = COALESCE(
                provider_customer_id,
                (SELECT s.provider_customer_id FROM subscriptions s WHERE s.id = payments.subscription_id)
             ),
             customer_email = COALESCE(
                customer_email,
                (SELECT s.customer_email FROM subscriptions s WHERE s.id = payments.subscription_id)
             ),
             order_email = COALESCE(
                order_email,
                (SELECT s.order_email FROM subscriptions s WHERE s.id = payments.subscription_id)
             )
         WHERE subscription_id = ?1",
        params![subscription_id],
    )?;

    Ok(())
}

fn subscription_payment_ids_without_attribution(
    conn: &ProjectConnection,
    subscription_id: i64,
) -> SyncResult<Vec<i64>> {
    conn.with_query(
        "SELECT id
         FROM payments
         WHERE subscription_id = ?1
           AND visitor_timestamp_id IS NULL
           AND payment_type IN ('single', 'subscription')
         ORDER BY paid_at ASC, id ASC",
        params![subscription_id],
        |mut rows| {
            let mut ids = Vec::new();
            while let Some(row) = rows.next()? {
                ids.push(row.get::<_, i64>(0)?);
            }
            Ok(ids)
        },
    )
    .map_err(Into::into)
}

fn session_metrics_at(
    conn: &ProjectConnection,
    visitor_timestamp_id: i64,
    paid_at: i64,
) -> SyncResult<(i64, i64)> {
    let upper_bound = paid_at.saturating_mul(NANOS_PER_SECOND);
    let metrics = conn.query_row(
        "SELECT
            COALESCE(SUM(COALESCE(duration_seconds, 0)), 0),
            COUNT(*)
         FROM pageviews
         WHERE visitor_timestamp_id = ?1
           AND timestamp_id <= ?2",
        params![visitor_timestamp_id, upper_bound],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
    )?;
    if metrics.1 > 0 {
        return Ok(metrics);
    }
    Ok(conn.query_row(
        "SELECT session_duration_seconds, session_page_count
         FROM landings
         WHERE visitor_timestamp_id = ?1",
        params![visitor_timestamp_id],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
    )?)
}

fn event_timestamp_id(event_at: i64) -> i64 {
    event_at.saturating_mul(NANOS_PER_SECOND)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::open_migrated_db;

    const VISITOR_ONE: i64 = 100 * NANOS_PER_SECOND + 1;
    const VISITOR_PURCHASER: i64 = 100 * NANOS_PER_SECOND + 500_000_000 + 1;
    const VISITOR_TWO: i64 = 110 * NANOS_PER_SECOND + 1;
    const SUCCESS_PAGE: i64 = 130 * NANOS_PER_SECOND + 1;

    fn insert_path(conn: &ProjectConnection, path: &str) -> i64 {
        conn.execute(
            "INSERT INTO paths (path, hostname) VALUES (?1, 'example.test')",
            params![path],
        )
        .expect("insert path");
        conn.query_row(
            "SELECT id FROM paths WHERE path = ?1",
            params![path],
            |row| row.get(0),
        )
        .expect("path id")
    }

    fn insert_landing(conn: &ProjectConnection, visitor_timestamp_id: i64, path_id: i64) {
        conn.execute(
            "INSERT INTO pageviews (
                timestamp_id, visitor_timestamp_id, previous_timestamp_id, path_id,
                duration_seconds, scrolled_percent
             ) VALUES (?1, ?1, NULL, ?2, 7, NULL)",
            params![visitor_timestamp_id, path_id],
        )
        .expect("insert landing pageview");
        conn.execute(
            "INSERT INTO landings (
                visitor_timestamp_id, session_duration_seconds, session_page_count,
                is_google, tracker_version, device
             ) VALUES (?1, 7, 1, 0, 1.0, 'desktop')",
            params![visitor_timestamp_id],
        )
        .expect("insert landing");
    }

    fn insert_pageview(
        conn: &ProjectConnection,
        timestamp_id: i64,
        visitor_timestamp_id: i64,
        previous_timestamp_id: i64,
        path_id: i64,
    ) {
        conn.execute(
            "INSERT INTO pageviews (
                timestamp_id, visitor_timestamp_id, previous_timestamp_id, path_id,
                duration_seconds, scrolled_percent
             ) VALUES (?1, ?2, ?3, ?4, 3, NULL)",
            params![
                timestamp_id,
                visitor_timestamp_id,
                previous_timestamp_id,
                path_id
            ],
        )
        .expect("insert pageview");
        conn.execute(
            "UPDATE landings
             SET session_duration_seconds = session_duration_seconds + 3,
                 session_page_count = session_page_count + 1
             WHERE visitor_timestamp_id = ?1",
            params![visitor_timestamp_id],
        )
        .expect("update landing metrics");
    }

    fn insert_subscription(conn: &ProjectConnection) -> i64 {
        conn.execute(
            "INSERT INTO subscriptions (
                provider, provider_subscription_id, customer_email, status, currency,
                interval_unit, interval_count, started_at, metadata
             ) VALUES (
                'stripe', 'sub_123', 'buyer@example.com', 'active', 'USD',
                'month', 1, 101, json('{\"checkout\":\"checkout_123\"}')
             )",
            [],
        )
        .expect("insert subscription");
        conn.query_row(
            "SELECT id FROM subscriptions WHERE provider_subscription_id = 'sub_123'",
            [],
            |row| row.get(0),
        )
        .expect("subscription id")
    }

    fn insert_payment(conn: &ProjectConnection, subscription_id: Option<i64>) -> i64 {
        conn.execute(
            "INSERT INTO payments (
                provider, subscription_id, provider_payment_id, provider_checkout_id,
                customer_email, payment_type, status, amount, currency, paid_at, created_at,
                metadata
             ) VALUES (
                'stripe', ?1, 'pay_123', 'checkout_123',
                'BUYER@example.com', 'subscription', 'ok', 5000, 'USD', 101, 101,
                json('{\"order\":\"order_123\"}')
             )",
            params![subscription_id],
        )
        .expect("insert payment");
        conn.query_row(
            "SELECT id FROM payments WHERE provider_payment_id = 'pay_123'",
            [],
            |row| row.get(0),
        )
        .expect("payment id")
    }

    #[test]
    fn attributes_payment_and_subscription_from_attribution_key_metadata() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/");
        insert_landing(&conn, VISITOR_ONE, home);
        conn.execute(
            "INSERT INTO metadata (visitor_timestamp_id, key, value, page_timestamp_id)
             VALUES (?1, 'attribution_key', 'Buyer@Example.Com', ?1)",
            params![VISITOR_ONE],
        )
        .expect("insert attribution key metadata");
        insert_pageview(
            &conn,
            VISITOR_ONE + 5 * NANOS_PER_SECOND,
            VISITOR_ONE,
            VISITOR_ONE,
            home,
        );
        let subscription_id = insert_subscription(&conn);
        let payment_id = insert_payment(&conn, Some(subscription_id));

        assert!(
            attribute_payment_by_provider_id(&conn, "stripe", "pay_123")
                .expect("attribute payment")
        );

        let payment: (Option<i64>, Option<i64>, Option<i64>, Option<i64>) = conn
            .query_row(
                "SELECT visitor_timestamp_id, attributed_visitor_timestamp_id,
                        session_duration_seconds, session_page_count
                 FROM payments
                 WHERE id = ?1",
                params![payment_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("payment");
        assert_eq!(
            payment,
            (Some(VISITOR_ONE), Some(VISITOR_ONE), Some(7), Some(1))
        );

        let subscription_visitor: (Option<i64>, Option<i64>) = conn
            .query_row(
                "SELECT visitor_timestamp_id, attributed_visitor_timestamp_id
                 FROM subscriptions
                 WHERE id = ?1",
                params![subscription_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("subscription visitor");
        assert_eq!(subscription_visitor, (Some(VISITOR_ONE), Some(VISITOR_ONE)));
    }

    #[test]
    fn account_id_redirects_revenue_attribution_to_original_session() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/");
        insert_landing(&conn, VISITOR_ONE, home);
        insert_landing(&conn, VISITOR_PURCHASER, home);
        conn.execute(
            "INSERT INTO metadata (visitor_timestamp_id, key, value, page_timestamp_id)
             VALUES (?1, 'account_id', 'acct_123', ?1)",
            params![VISITOR_ONE],
        )
        .expect("insert original account metadata");
        conn.execute(
            "INSERT INTO metadata (visitor_timestamp_id, key, value, page_timestamp_id)
             VALUES
                (?1, 'account_id', 'ACCT_123', ?1),
                (?1, 'user_email', 'buyer@example.com', ?1)",
            params![VISITOR_PURCHASER],
        )
        .expect("insert purchaser metadata");
        let subscription_id = insert_subscription(&conn);
        let payment_id = insert_payment(&conn, Some(subscription_id));

        assert!(
            attribute_payment_by_provider_id(&conn, "stripe", "pay_123")
                .expect("attribute payment")
        );

        let linked_visitors: (Option<i64>, Option<i64>, Option<i64>, Option<i64>) = conn
            .query_row(
                "SELECT p.visitor_timestamp_id, p.attributed_visitor_timestamp_id,
                        s.visitor_timestamp_id, s.attributed_visitor_timestamp_id
                 FROM payments p
                 JOIN subscriptions s ON s.id = p.subscription_id
                 WHERE p.id = ?1",
                params![payment_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("linked visitors");
        assert_eq!(
            linked_visitors,
            (
                Some(VISITOR_PURCHASER),
                Some(VISITOR_ONE),
                Some(VISITOR_PURCHASER),
                Some(VISITOR_ONE)
            )
        );
    }

    #[test]
    fn attributes_payment_from_metadata_value() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/");
        insert_landing(&conn, VISITOR_ONE, home);
        conn.execute(
            "INSERT INTO metadata (visitor_timestamp_id, key, value, page_timestamp_id)
             VALUES (?1, 'customer_email', 'buyer@example.com', ?1)",
            params![VISITOR_ONE],
        )
        .expect("insert metadata");
        let subscription_id = insert_subscription(&conn);
        let payment_id = insert_payment(&conn, Some(subscription_id));

        assert!(
            attribute_payment_by_provider_id(&conn, "stripe", "pay_123")
                .expect("attribute payment")
        );

        let payment_visitor: Option<i64> = conn
            .query_row(
                "SELECT visitor_timestamp_id FROM payments WHERE id = ?1",
                params![payment_id],
                |row| row.get(0),
            )
            .expect("payment visitor");
        assert_eq!(payment_visitor, Some(VISITOR_ONE));
    }

    #[test]
    fn attributes_payment_from_visitor_timestamp_id_in_payment_metadata() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/");
        insert_landing(&conn, VISITOR_ONE, home);
        let subscription_id = insert_subscription(&conn);
        let payment_id = insert_payment(&conn, Some(subscription_id));
        let visitor_id = crate::util::encode_visitor_timestamp_id(VISITOR_ONE);
        conn.execute(
            "UPDATE payments
             SET metadata = json_object('visitor_id', ?2)
             WHERE id = ?1",
            params![payment_id, visitor_id],
        )
        .expect("set visitor id metadata");

        assert!(
            attribute_payment_by_provider_id(&conn, "stripe", "pay_123")
                .expect("attribute payment")
        );

        let payment_visitor: Option<i64> = conn
            .query_row(
                "SELECT visitor_timestamp_id FROM payments WHERE id = ?1",
                params![payment_id],
                |row| row.get(0),
            )
            .expect("payment visitor");
        assert_eq!(payment_visitor, Some(VISITOR_ONE));
    }

    #[test]
    fn attributes_payment_from_client_page_id_in_subscription_metadata() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/");
        insert_landing(&conn, VISITOR_ONE, home);
        let page_timestamp_id = VISITOR_ONE + NANOS_PER_SECOND / 2;
        insert_pageview(&conn, page_timestamp_id, VISITOR_ONE, VISITOR_ONE, home);
        let page_id_hex = "0123456789abcdef01234567";
        let page_id = ClientPageId::from_hex(page_id_hex).expect("client page id");
        conn.execute(
            "INSERT INTO client_page_ids_map (visitor_timestamp_id, page_id)
             VALUES (?1, ?2)",
            params![page_timestamp_id, page_id],
        )
        .expect("insert client page id map");
        let subscription_id = insert_subscription(&conn);
        conn.execute(
            "UPDATE subscriptions
             SET metadata = json_object('page_id', ?2)
             WHERE id = ?1",
            params![subscription_id, page_id_hex],
        )
        .expect("set page id metadata");
        let payment_id = insert_payment(&conn, Some(subscription_id));

        assert!(
            attribute_payment_by_provider_id(&conn, "stripe", "pay_123")
                .expect("attribute payment")
        );

        let payment_visitor: Option<i64> = conn
            .query_row(
                "SELECT visitor_timestamp_id FROM payments WHERE id = ?1",
                params![payment_id],
                |row| row.get(0),
            )
            .expect("payment visitor");
        assert_eq!(payment_visitor, Some(VISITOR_ONE));
    }

    #[test]
    fn metadata_landing_ignores_sessions_at_or_after_paid_at() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/");
        insert_landing(&conn, VISITOR_TWO, home);
        conn.execute(
            "INSERT INTO metadata (visitor_timestamp_id, key, value, page_timestamp_id)
             VALUES (?1, 'customer_email', 'buyer@example.com', ?1)",
            params![VISITOR_TWO],
        )
        .expect("insert post-payment metadata");
        let subscription_id = insert_subscription(&conn);
        let payment_id = insert_payment(&conn, Some(subscription_id));

        assert!(
            !attribute_payment_by_provider_id(&conn, "stripe", "pay_123")
                .expect("attribute payment")
        );

        let payment_visitor: Option<i64> = conn
            .query_row(
                "SELECT visitor_timestamp_id FROM payments WHERE id = ?1",
                params![payment_id],
                |row| row.get(0),
            )
            .expect("payment visitor");
        assert_eq!(payment_visitor, None);
    }

    #[test]
    fn metadata_landing_prefers_closest_session_strictly_before_paid_at() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/");
        insert_landing(&conn, VISITOR_ONE, home);
        insert_landing(&conn, VISITOR_TWO, home);
        conn.execute(
            "INSERT INTO metadata (visitor_timestamp_id, key, value, page_timestamp_id)
             VALUES
                (?1, 'customer_email', 'buyer@example.com', ?1),
                (?2, 'customer_email', 'buyer@example.com', ?2)",
            params![VISITOR_ONE, VISITOR_TWO],
        )
        .expect("insert before and after metadata");
        let subscription_id = insert_subscription(&conn);
        let payment_id = insert_payment(&conn, Some(subscription_id));

        assert!(
            attribute_payment_by_provider_id(&conn, "stripe", "pay_123")
                .expect("attribute payment")
        );

        let payment_visitor: Option<i64> = conn
            .query_row(
                "SELECT visitor_timestamp_id FROM payments WHERE id = ?1",
                params![payment_id],
                |row| row.get(0),
            )
            .expect("payment visitor");
        assert_eq!(payment_visitor, Some(VISITOR_ONE));
    }

    #[test]
    fn payment_attribution_uses_existing_subscription_session_before_lookup() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/");
        insert_landing(&conn, VISITOR_ONE, home);
        conn.execute(
            "INSERT INTO metadata (visitor_timestamp_id, key, value, page_timestamp_id)
             VALUES (?1, 'attribution_key', 'buyer@example.com', ?1)",
            params![VISITOR_ONE],
        )
        .expect("insert attribution key metadata");
        insert_landing(&conn, VISITOR_TWO, home);
        let subscription_id = insert_subscription(&conn);
        let payment_id = insert_payment(&conn, Some(subscription_id));
        conn.execute(
            "UPDATE subscriptions
             SET visitor_timestamp_id = ?2
             WHERE id = ?1",
            params![subscription_id, VISITOR_TWO],
        )
        .expect("pre-attribute subscription");

        assert!(
            attribute_payment_by_provider_id(&conn, "stripe", "pay_123")
                .expect("attribute payment")
        );

        let linked_visitors: (Option<i64>, Option<i64>) = conn
            .query_row(
                "SELECT p.visitor_timestamp_id, s.visitor_timestamp_id
                 FROM payments p
                 JOIN subscriptions s ON s.id = p.subscription_id
                 WHERE p.id = ?1",
                params![payment_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("linked visitors");
        assert_eq!(linked_visitors, (Some(VISITOR_TWO), Some(VISITOR_TWO)));
    }

    #[test]
    fn payment_attribution_propagates_subscription_session_to_payment() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/");
        insert_landing(&conn, VISITOR_ONE, home);
        let subscription_id = insert_subscription(&conn);
        let payment_id = insert_payment(&conn, Some(subscription_id));
        conn.execute(
            "UPDATE subscriptions
             SET visitor_timestamp_id = ?2
             WHERE id = ?1",
            params![subscription_id, VISITOR_ONE],
        )
        .expect("pre-attribute subscription");

        assert!(
            attribute_payment_by_provider_id(&conn, "stripe", "pay_123")
                .expect("attribute payment")
        );

        let payment: (Option<i64>, Option<i64>, Option<i64>) = conn
            .query_row(
                "SELECT visitor_timestamp_id, session_duration_seconds, session_page_count
                 FROM payments
                 WHERE id = ?1",
                params![payment_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("payment");
        assert_eq!(payment, (Some(VISITOR_ONE), Some(7), Some(1)));
    }

    #[test]
    fn subscription_attribution_propagates_first_payment_session_to_subscription() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/");
        insert_landing(&conn, VISITOR_ONE, home);
        let subscription_id = insert_subscription(&conn);
        let payment_id = insert_payment(&conn, Some(subscription_id));
        conn.execute(
            "UPDATE payments
             SET visitor_timestamp_id = ?2
             WHERE id = ?1",
            params![payment_id, VISITOR_ONE],
        )
        .expect("pre-attribute payment");

        assert!(attribute_subscription_by_id(&conn, subscription_id).expect("attribute sub"));

        let subscription_visitor: Option<i64> = conn
            .query_row(
                "SELECT visitor_timestamp_id FROM subscriptions WHERE id = ?1",
                params![subscription_id],
                |row| row.get(0),
            )
            .expect("subscription visitor");
        assert_eq!(subscription_visitor, Some(VISITOR_ONE));
    }

    #[test]
    fn propagates_missing_fields_between_linked_subscription_and_payments() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/");
        insert_landing(&conn, VISITOR_ONE, home);

        conn.execute(
            "INSERT INTO subscriptions (
                provider, provider_subscription_id, visitor_timestamp_id,
                provider_customer_id, status, currency, interval_unit, interval_count, started_at
             ) VALUES (
                'shopify', 'sub_linked', ?1,
                'cus_sub', 'active', 'USD', 'month', 1, 100
             )",
            params![VISITOR_ONE],
        )
        .expect("insert linked subscription");
        let subscription_id: i64 = conn
            .query_row(
                "SELECT id FROM subscriptions WHERE provider_subscription_id = 'sub_linked'",
                [],
                |row| row.get(0),
            )
            .expect("subscription id");

        conn.execute(
            "INSERT INTO payments (
                provider, subscription_id, provider_payment_id, provider_checkout_id,
                provider_order_id, customer_email, order_email, payment_type, status,
                amount, currency, paid_at, created_at
             ) VALUES (
                'shopify', ?1, 'pay_linked', 'checkout_linked',
                'order_linked', 'buyer@example.com', 'order@example.com', 'subscription', 'ok',
                5000, 'USD', 101, 101
             )",
            params![subscription_id],
        )
        .expect("insert linked payment");

        propagate_linked_subscription_payment_fields(&conn, subscription_id)
            .expect("propagate linked fields");

        let subscription: (
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        ) = conn
            .query_row(
                "SELECT provider_checkout_id, provider_order_id, customer_email, order_email
                 FROM subscriptions
                 WHERE id = ?1",
                params![subscription_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("subscription fields");
        assert_eq!(
            subscription,
            (
                Some("checkout_linked".to_string()),
                Some("order_linked".to_string()),
                Some("buyer@example.com".to_string()),
                Some("order@example.com".to_string())
            )
        );

        let payment: (Option<i64>, Option<i64>, Option<i64>, Option<String>) = conn
            .query_row(
                "SELECT visitor_timestamp_id, session_duration_seconds,
                        session_page_count, provider_customer_id
                 FROM payments
                 WHERE provider_payment_id = 'pay_linked'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("payment fields");
        assert_eq!(
            payment,
            (
                Some(VISITOR_ONE),
                Some(7),
                Some(1),
                Some("cus_sub".to_string())
            )
        );
    }

    #[test]
    fn link_propagation_does_not_overwrite_existing_subscription_session() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/");
        insert_landing(&conn, VISITOR_ONE, home);
        insert_landing(&conn, VISITOR_TWO, home);

        conn.execute(
            "INSERT INTO subscriptions (
                provider, provider_subscription_id, visitor_timestamp_id,
                status, currency, interval_unit, interval_count, started_at
             ) VALUES (
                'shopify', 'sub_conflict', ?1,
                'active', 'USD', 'month', 1, 100
             )",
            params![VISITOR_TWO],
        )
        .expect("insert linked subscription");
        let subscription_id: i64 = conn
            .query_row(
                "SELECT id FROM subscriptions WHERE provider_subscription_id = 'sub_conflict'",
                [],
                |row| row.get(0),
            )
            .expect("subscription id");

        conn.execute(
            "INSERT INTO payments (
                provider, subscription_id, provider_payment_id, visitor_timestamp_id,
                payment_type, status, amount, currency, paid_at, created_at
             ) VALUES (
                'shopify', ?1, 'pay_conflict', ?2,
                'subscription', 'ok', 5000, 'USD', 101, 101
             )",
            params![subscription_id, VISITOR_ONE],
        )
        .expect("insert linked payment");

        propagate_linked_subscription_payment_fields(&conn, subscription_id)
            .expect("propagate linked fields");

        let linked_visitors: (Option<i64>, Option<i64>) = conn
            .query_row(
                "SELECT p.visitor_timestamp_id, s.visitor_timestamp_id
                 FROM payments p
                 JOIN subscriptions s ON s.id = p.subscription_id
                 WHERE p.provider_payment_id = 'pay_conflict'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("linked visitors");
        assert_eq!(linked_visitors, (Some(VISITOR_ONE), Some(VISITOR_TWO)));
    }

    #[test]
    fn success_path_fallback_uses_closest_unused_session() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/");
        let success = insert_path(&conn, "/checkout/success");
        insert_landing(&conn, VISITOR_ONE, home);
        insert_pageview(&conn, SUCCESS_PAGE, VISITOR_ONE, VISITOR_ONE, success);
        insert_landing(&conn, VISITOR_TWO, home);
        insert_pageview(
            &conn,
            SUCCESS_PAGE + NANOS_PER_SECOND,
            VISITOR_TWO,
            VISITOR_TWO,
            success,
        );
        let payment_id = insert_payment(&conn, None);
        let row = load_payment_attribution_row(&conn, payment_id)
            .expect("load payment")
            .expect("payment row");

        assert!(
            attribute_entity(
                &conn,
                &row,
                row.paid_at,
                SessionLookup::BySuccessPath("/checkout/success"),
            )
            .expect("attribute by success path")
        );

        let visitor_timestamp_id: Option<i64> = conn
            .query_row(
                "SELECT visitor_timestamp_id FROM payments WHERE id = ?1",
                params![payment_id],
                |row| row.get(0),
            )
            .expect("payment visitor");
        assert_eq!(visitor_timestamp_id, Some(VISITOR_ONE));
    }
}
