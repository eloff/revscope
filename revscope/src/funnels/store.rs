//! Saved funnels and the pages and events a step can name.

use rusqlite::{OptionalExtension, Row, params};

use crate::auth::{AuthError, map_row_error};
use crate::db::ProjectConnection;
use crate::util::{seconds_since_epoch, split_list};

use super::request::trimmed_optional;
use super::types::{
    Funnel, FunnelEventOption, FunnelOption, FunnelOptionsResponse, FunnelStep, FunnelsResponse,
    SaveFunnelRequest, payment_types_to_text,
};

/// The `funnel_steps` columns [`read_step`] expects, joined with the event
/// name and path a step names.
const STEP_COLUMNS: &str = "fs.id,
            fs.step_order,
            fs.step_type,
            fs.event_name_id,
            en.name,
            fs.path_id,
            p.path,
            p.hostname,
            fs.is_landing,
            fs.payment_types,
            fs.description";

const STEP_JOINS: &str = "LEFT JOIN event_names en ON en.id = fs.event_name_id
         LEFT JOIN paths p ON p.id = fs.path_id";

/// Reads [`STEP_COLUMNS`] starting at column `first`.
fn read_step(row: &Row<'_>, first: usize) -> rusqlite::Result<FunnelStep> {
    Ok(FunnelStep {
        id: row.get(first)?,
        order: row.get(first + 1)?,
        step_type: row.get(first + 2)?,
        event_name_id: row.get(first + 3)?,
        event_name: row.get(first + 4)?,
        path_id: row.get(first + 5)?,
        path: row.get(first + 6)?,
        hostname: row.get(first + 7)?,
        is_landing: row.get::<_, i64>(first + 8)? != 0,
        payment_types: split_list(
            row.get::<_, Option<String>>(first + 9)?
                .as_deref()
                .unwrap_or(""),
        ),
        description: row.get(first + 10)?,
    })
}

/// Every funnel that is not deleted, oldest first, with its steps in order.
pub(super) fn load_funnels(conn: &ProjectConnection) -> Result<FunnelsResponse, AuthError> {
    let sql = format!(
        "SELECT f.id, f.name, f.description, {STEP_COLUMNS}
         FROM funnels f
         LEFT JOIN funnel_steps fs ON fs.funnel_id = f.id
         {STEP_JOINS}
         WHERE f.deleted_at IS NULL
         ORDER BY f.id ASC, fs.step_order ASC"
    );
    conn.with_query(&sql, [], |mut rows| {
        let mut funnels: Vec<Funnel> = Vec::new();
        while let Some(row) = rows.next()? {
            let funnel_id = row.get::<_, i64>(0)?;
            if !funnels.last().is_some_and(|funnel| funnel.id == funnel_id) {
                funnels.push(Funnel {
                    id: funnel_id,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    steps: Vec::new(),
                });
            }
            // A funnel without steps still comes back once, with NULL step columns.
            if row.get::<_, Option<i64>>(3)?.is_some() {
                let funnel = funnels.last_mut().expect("funnel pushed above");
                funnel.steps.push(read_step(row, 3)?);
            }
        }
        Ok(FunnelsResponse { funnels })
    })
    .map_err(map_row_error)
}

pub(super) fn load_funnel_by_id(
    conn: &ProjectConnection,
    funnel_id: i64,
) -> Result<Funnel, AuthError> {
    load_funnels(conn)?
        .funnels
        .into_iter()
        .find(|funnel| funnel.id == funnel_id)
        .ok_or(AuthError::InvalidRequest("funnel not found"))
}

pub(super) fn load_funnel_steps(
    conn: &ProjectConnection,
    funnel_id: i64,
) -> Result<Vec<FunnelStep>, AuthError> {
    let exists = conn
        .query_row(
            "SELECT 1 FROM funnels WHERE id = ?1 AND deleted_at IS NULL",
            params![funnel_id],
            |_| Ok(()),
        )
        .optional()
        .map_err(map_row_error)?;
    if exists.is_none() {
        return Err(AuthError::InvalidRequest("funnel not found"));
    }
    conn.with_query(
        &format!(
            "SELECT {STEP_COLUMNS}
             FROM funnel_steps fs
             {STEP_JOINS}
             WHERE fs.funnel_id = ?1
             ORDER BY fs.step_order ASC"
        ),
        params![funnel_id],
        |mut rows| {
            let mut steps = Vec::new();
            while let Some(row) = rows.next()? {
                steps.push(read_step(row, 0)?);
            }
            Ok(steps)
        },
    )
    .map_err(map_row_error)
}

/// Saves a normalized request as a new funnel, or over the one it names,
/// replacing its steps. Returns the funnel's id.
pub(super) fn persist_funnel(
    conn: &ProjectConnection,
    request: &SaveFunnelRequest,
) -> Result<i64, AuthError> {
    let now = seconds_since_epoch() as i64;
    let description = request.description.as_deref().and_then(trimmed_optional);
    let funnel_id = if let Some(funnel_id) = request.id {
        let updated = conn
            .execute(
                "UPDATE funnels
                 SET name = ?2, description = ?3, updated_at = ?4
                 WHERE id = ?1 AND deleted_at IS NULL",
                params![
                    funnel_id,
                    request.name.as_str(),
                    description.as_deref(),
                    now
                ],
            )
            .map_err(map_row_error)?;
        if updated == 0 {
            return Err(AuthError::InvalidRequest("funnel not found"));
        }
        conn.execute(
            "DELETE FROM funnel_steps WHERE funnel_id = ?1",
            params![funnel_id],
        )
        .map_err(map_row_error)?;
        funnel_id
    } else {
        conn.execute(
            "INSERT INTO funnels (name, description, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?3)",
            params![request.name.as_str(), description.as_deref(), now],
        )
        .map_err(map_row_error)?;
        conn.last_insert_rowid()
    };

    for (index, step) in request.steps.iter().enumerate() {
        conn.execute(
            "INSERT INTO funnel_steps (
                funnel_id, step_order, step_type, event_name_id, path_id, is_landing,
                payment_types, description
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                funnel_id,
                index as i64,
                step.step_type.as_str(),
                step.event_name_id,
                step.path_id,
                i64::from(step.is_landing),
                payment_types_to_text(&step.payment_types),
                step.description.as_deref().and_then(trimmed_optional),
            ],
        )
        .map_err(map_row_error)?;
    }
    Ok(funnel_id)
}

pub(super) fn soft_delete_funnel(
    conn: &ProjectConnection,
    funnel_id: i64,
) -> Result<(), AuthError> {
    conn.execute(
        "UPDATE funnels
         SET deleted_at = ?2, updated_at = ?2
         WHERE id = ?1 AND deleted_at IS NULL",
        params![funnel_id, seconds_since_epoch() as i64],
    )
    .map_err(map_row_error)?;
    Ok(())
}

/// Every live page, and every event with the pages it has fired on, for the
/// step pickers in the funnel editor.
pub(super) fn load_funnel_options(
    conn: &ProjectConnection,
) -> Result<FunnelOptionsResponse, AuthError> {
    let paths = conn
        .with_query(
            "SELECT id, hostname || path
             FROM paths
             WHERE deleted_at IS NULL
             ORDER BY hostname ASC, path ASC",
            [],
            |mut rows| {
                let mut out = Vec::new();
                while let Some(row) = rows.next()? {
                    out.push(FunnelOption {
                        id: row.get(0)?,
                        name: row.get(1)?,
                    });
                }
                Ok(out)
            },
        )
        .map_err(map_row_error)?;
    let events = conn
        .with_query(
            "SELECT
                en.id,
                en.name,
                pv.path_id
             FROM event_names en
             JOIN events e ON e.event_name_id = en.id
             JOIN pageviews pv ON pv.timestamp_id = e.page_timestamp_id
             JOIN paths p ON p.id = pv.path_id
             WHERE en.deleted_at IS NULL
               AND p.deleted_at IS NULL
             GROUP BY en.id, en.name, pv.path_id
             ORDER BY en.name ASC, en.id ASC, pv.path_id ASC",
            [],
            |mut rows| {
                let mut out: Vec<FunnelEventOption> = Vec::new();
                while let Some(row) = rows.next()? {
                    let id = row.get::<_, i64>(0)?;
                    let path_id = row.get::<_, i64>(2)?;
                    if out.last().is_some_and(|event| event.id == id) {
                        out.last_mut()
                            .expect("event option exists")
                            .path_ids
                            .push(path_id);
                    } else {
                        out.push(FunnelEventOption {
                            id,
                            name: row.get(1)?,
                            path_ids: vec![path_id],
                        });
                    }
                }
                Ok(out)
            },
        )
        .map_err(map_row_error)?;
    Ok(FunnelOptionsResponse { paths, events })
}
