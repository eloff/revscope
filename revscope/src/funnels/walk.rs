//! Walks visitors through a funnel's steps in SQLite TEMP tables, each step
//! keeping the visitors who completed it after completing the step before.

use rusqlite::types::ToSql;
use rusqlite::{OptionalExtension, params_from_iter};

use crate::auth::{AuthError, map_row_error};
use crate::db::ProjectConnection;
use crate::metrics::ratio;
use crate::segment_filter::SegmentFilters;
use crate::util::NANOS_PER_SECOND;

use super::types::{FunnelDropoffRow, FunnelStep, FunnelStepDropoff, payment_types_to_text};

/// Event name the tracker emits when a visitor clicks a link that leaves the
/// site (see `tracker.js`). Its `metadata.to` records the outbound destination.
pub(super) const OUTBOUND_CLICK_EVENT_NAME: &str = "revscope-outbound-click";

/// Excludes subscription renewal invoices; only the first successful charge counts.
const EXCLUDE_SUBSCRIPTION_RENEWALS: &str = "NOT (
    payment_type = 'subscription'
    AND EXISTS (
        SELECT 1
        FROM payments prior
        WHERE prior.subscription_id = payments.subscription_id
          AND prior.status = 'ok'
          AND prior.payment_type = 'subscription'
          AND prior.paid_at < payments.paid_at
          AND prior.id != payments.id
    )
)";

pub(super) struct FunnelWalkOptions {
    pub(super) compute_dropoff: bool,
    pub(super) compute_revenue: bool,
    pub(super) outbound_event_id: Option<i64>,
}

pub(super) struct FunnelWalkResult {
    pub(super) counts: Vec<i64>,
    /// Per-step drop-off destinations when `compute_dropoff` was set; `None` for
    /// steps that have no drop-off (final step / payment steps).
    pub(super) dropoffs: Vec<Option<FunnelStepDropoff>>,
    pub(super) total_revenue: Option<i64>,
}

/// Request-scoped TEMP tables for the sliding-window funnel walk. Dropped on
/// success or failure so pooled connections do not leak survivor sets.
struct FunnelTempTables<'a> {
    conn: &'a ProjectConnection,
}

impl<'a> FunnelTempTables<'a> {
    fn create(conn: &'a ProjectConnection) -> Result<Self, AuthError> {
        conn.execute_batch(
            "DROP TABLE IF EXISTS funnel_survivors;
             DROP TABLE IF EXISTS funnel_next_survivors;
             CREATE TEMP TABLE funnel_survivors (
                 session_id INTEGER PRIMARY KEY,
                 completion_ns INTEGER NOT NULL,
                 page_ts INTEGER
             );
             CREATE TEMP TABLE funnel_next_survivors (
                 session_id INTEGER PRIMARY KEY,
                 completion_ns INTEGER NOT NULL,
                 page_ts INTEGER
             );",
        )
        .map_err(map_row_error)?;
        Ok(Self { conn })
    }

    fn swap(&self) -> Result<(), AuthError> {
        self.conn
            .execute_batch(
                "DROP TABLE funnel_survivors;
                 ALTER TABLE funnel_next_survivors RENAME TO funnel_survivors;
                 CREATE TEMP TABLE funnel_next_survivors (
                     session_id INTEGER PRIMARY KEY,
                     completion_ns INTEGER NOT NULL,
                     page_ts INTEGER
                 );",
            )
            .map_err(map_row_error)
    }
}

impl Drop for FunnelTempTables<'_> {
    fn drop(&mut self) {
        let _ = self.conn.execute_batch(
            "DROP TABLE IF EXISTS funnel_next_survivors;
             DROP TABLE IF EXISTS funnel_survivors;",
        );
    }
}

/// Walks a funnel one step at a time in SQLite TEMP tables, keeping only the
/// current and next survivor sets in memory at the database layer.
///
/// A session qualifies for step `i` only if it qualified for step `i - 1` and
/// has an occurrence of step `i` strictly after the occurrence that satisfied
/// step `i - 1`. We always advance using the earliest qualifying occurrence so
/// subsequent steps have the best chance to match.
///
/// Visitor counts come from `changes()` after each `INSERT…SELECT` (one row per
/// session). Drop-off destinations are computed from the adjacent TEMP tables
/// before the window slides; final-payment revenue is summed from the last
/// survivor table when requested.
pub(super) fn walk_funnel(
    conn: &ProjectConnection,
    steps: &[FunnelStep],
    start: i64,
    end: i64,
    segments: &SegmentFilters,
    options: FunnelWalkOptions,
) -> Result<FunnelWalkResult, AuthError> {
    // Read checkouts use query_only; TEMP DDL/DML needs it cleared for this walk.
    // Declared before `temps` so Drop order drops temps while writes are still allowed.
    let _allow_temp_writes = conn
        .allow_temp_writes()
        .map_err(|error| AuthError::Db(error.to_string()))?;
    let temps = FunnelTempTables::create(conn)?;
    let mut counts = Vec::with_capacity(steps.len());
    let mut dropoffs = if options.compute_dropoff {
        vec![None; steps.len()]
    } else {
        Vec::new()
    };

    for (index, step) in steps.iter().enumerate() {
        conn.execute("DELETE FROM funnel_next_survivors", [])
            .map_err(map_row_error)?;
        insert_step_survivors(conn, step, start, end, index > 0, segments)?;
        counts.push(conn.changes() as i64);

        if options.compute_dropoff && index > 0 {
            let prev_step = &steps[index - 1];
            if prev_step.step_type == "pageview" {
                let next_step_path_id = if step.step_type == "pageview" {
                    step.path_id
                } else {
                    None
                };
                dropoffs[index - 1] = Some(query_dropoff_from_temps(
                    conn,
                    next_step_path_id,
                    options.outbound_event_id,
                )?);
            }
        }

        temps.swap()?;
    }

    let total_revenue = if options.compute_revenue {
        let Some(step) = steps.last().filter(|step| step.step_type == "payment") else {
            return Err(AuthError::InvalidRequest(
                "payment step required for revenue",
            ));
        };
        Some(final_payment_revenue_from_temps(
            conn, step, start, end, segments,
        )?)
    } else {
        None
    };

    // Drop temps before returning so a subsequent previous-period walk starts clean.
    // Must run before `_allow_temp_writes` drops (restores query_only).
    drop(temps);

    Ok(FunnelWalkResult {
        counts,
        dropoffs,
        total_revenue,
    })
}

/// Inserts earliest qualifying occurrences for `step` into `funnel_next_survivors`.
/// When `join_previous` is set, only sessions in `funnel_survivors` that have a
/// later occurrence qualify.
fn insert_step_survivors(
    conn: &ProjectConnection,
    step: &FunnelStep,
    start: i64,
    end: i64,
    join_previous: bool,
    segments: &SegmentFilters,
) -> Result<(), AuthError> {
    let is_landing = if step.is_landing { 1 } else { 0 };
    match step.step_type.as_str() {
        "pageview" if step.event_name_id.is_some() => {
            insert_event_step_survivors(conn, step, start, end, is_landing, join_previous, segments)
        }
        "pageview" => insert_pageview_step_survivors(
            conn,
            step,
            start,
            end,
            is_landing,
            join_previous,
            segments,
        ),
        "payment" => insert_payment_step_survivors(conn, step, start, end, join_previous, segments),
        _ => Err(AuthError::InvalidRequest("invalid funnel step type")),
    }
}

fn insert_pageview_step_survivors(
    conn: &ProjectConnection,
    step: &FunnelStep,
    start: i64,
    end: i64,
    is_landing: i64,
    join_previous: bool,
    segments: &SegmentFilters,
) -> Result<(), AuthError> {
    let start_ns = start * NANOS_PER_SECOND;
    let end_ns = end * NANOS_PER_SECOND;
    let country_filter = segments.country_exists_clause("pv.visitor_timestamp_id", 5);
    let session_filter = segments.session_filter_clause("pv.visitor_timestamp_id", None);
    let sql = if join_previous {
        format!(
            "INSERT INTO funnel_next_survivors (session_id, completion_ns, page_ts)
             SELECT prev.session_id, MIN(pv.timestamp_id), MIN(pv.timestamp_id)
             FROM funnel_survivors prev
             JOIN pageviews pv ON pv.visitor_timestamp_id = prev.session_id
             JOIN paths p ON p.id = pv.path_id
             WHERE pv.path_id = ?1
               AND pv.timestamp_id >= ?2
               AND pv.timestamp_id < ?3
               AND pv.timestamp_id > prev.completion_ns
               AND (?4 = 0 OR pv.visitor_timestamp_id = pv.timestamp_id)
               AND p.deleted_at IS NULL{country_filter}{session_filter}
             GROUP BY prev.session_id"
        )
    } else {
        format!(
            "INSERT INTO funnel_next_survivors (session_id, completion_ns, page_ts)
             SELECT pv.visitor_timestamp_id, MIN(pv.timestamp_id), MIN(pv.timestamp_id)
             FROM pageviews pv
             JOIN paths p ON p.id = pv.path_id
             WHERE pv.path_id = ?1
               AND pv.timestamp_id >= ?2
               AND pv.timestamp_id < ?3
               AND (?4 = 0 OR pv.visitor_timestamp_id = pv.timestamp_id)
               AND p.deleted_at IS NULL{country_filter}{session_filter}
             GROUP BY pv.visitor_timestamp_id"
        )
    };
    let mut sql_params: Vec<&dyn ToSql> = vec![&step.path_id, &start_ns, &end_ns, &is_landing];
    segments.append_country_params(&mut sql_params);
    conn.execute(&sql, params_from_iter(sql_params))
        .map_err(map_row_error)?;
    Ok(())
}

fn insert_event_step_survivors(
    conn: &ProjectConnection,
    step: &FunnelStep,
    start: i64,
    end: i64,
    is_landing: i64,
    join_previous: bool,
    segments: &SegmentFilters,
) -> Result<(), AuthError> {
    let start_ns = start * NANOS_PER_SECOND;
    let end_ns = end * NANOS_PER_SECOND;
    let country_filter = segments.country_exists_clause("pv.visitor_timestamp_id", 6);
    let session_filter = segments.session_filter_clause("pv.visitor_timestamp_id", None);
    let sql = if join_previous {
        format!(
            "INSERT INTO funnel_next_survivors (session_id, completion_ns, page_ts)
             SELECT session_id, completion_ns, page_ts
             FROM (
                 SELECT
                     pv.visitor_timestamp_id AS session_id,
                     e.timestamp_id AS completion_ns,
                     e.page_timestamp_id AS page_ts,
                     ROW_NUMBER() OVER (
                         PARTITION BY pv.visitor_timestamp_id
                         ORDER BY e.timestamp_id ASC
                     ) AS rn
                 FROM funnel_survivors prev
                 JOIN pageviews pv ON pv.visitor_timestamp_id = prev.session_id
                 JOIN events e ON e.page_timestamp_id = pv.timestamp_id
                 JOIN event_names en ON en.id = e.event_name_id
                 JOIN paths p ON p.id = pv.path_id
                 WHERE e.event_name_id = ?1
                   AND pv.path_id = ?2
                   AND e.timestamp_id >= ?3
                   AND e.timestamp_id < ?4
                   AND e.timestamp_id > prev.completion_ns
                   AND (?5 = 0 OR pv.visitor_timestamp_id = pv.timestamp_id)
                   AND en.deleted_at IS NULL
                   AND p.deleted_at IS NULL{country_filter}{session_filter}
             ) ranked
             WHERE rn = 1"
        )
    } else {
        format!(
            "INSERT INTO funnel_next_survivors (session_id, completion_ns, page_ts)
             SELECT session_id, completion_ns, page_ts
             FROM (
                 SELECT
                     pv.visitor_timestamp_id AS session_id,
                     e.timestamp_id AS completion_ns,
                     e.page_timestamp_id AS page_ts,
                     ROW_NUMBER() OVER (
                         PARTITION BY pv.visitor_timestamp_id
                         ORDER BY e.timestamp_id ASC
                     ) AS rn
                 FROM events e
                 JOIN pageviews pv ON pv.timestamp_id = e.page_timestamp_id
                 JOIN event_names en ON en.id = e.event_name_id
                 JOIN paths p ON p.id = pv.path_id
                 WHERE e.event_name_id = ?1
                   AND pv.path_id = ?2
                   AND e.timestamp_id >= ?3
                   AND e.timestamp_id < ?4
                   AND (?5 = 0 OR pv.visitor_timestamp_id = pv.timestamp_id)
                   AND en.deleted_at IS NULL
                   AND p.deleted_at IS NULL{country_filter}{session_filter}
             ) ranked
             WHERE rn = 1"
        )
    };
    let mut sql_params: Vec<&dyn ToSql> = vec![
        &step.event_name_id,
        &step.path_id,
        &start_ns,
        &end_ns,
        &is_landing,
    ];
    segments.append_country_params(&mut sql_params);
    conn.execute(&sql, params_from_iter(sql_params))
        .map_err(map_row_error)?;
    Ok(())
}

fn insert_payment_step_survivors(
    conn: &ProjectConnection,
    step: &FunnelStep,
    start: i64,
    end: i64,
    join_previous: bool,
    segments: &SegmentFilters,
) -> Result<(), AuthError> {
    let country_filter = segments.country_exists_clause("payments.visitor_timestamp_id", 4);
    let session_filter = segments.session_filter_clause("payments.visitor_timestamp_id", None);
    let payment_types = payment_types_to_text(&step.payment_types);
    let sql = if join_previous {
        format!(
            "INSERT INTO funnel_next_survivors (session_id, completion_ns, page_ts)
             SELECT
                 prev.session_id,
                 MIN(payments.paid_at) * {NANOS_PER_SECOND},
                 NULL
             FROM funnel_survivors prev
             JOIN payments ON payments.visitor_timestamp_id = prev.session_id
             WHERE payments.status = 'ok'
               AND payments.paid_at >= ?1
               AND payments.paid_at < ?2
               AND instr(',' || ?3 || ',', ',' || payments.payment_type || ',') > 0
               AND (payments.paid_at * {NANOS_PER_SECOND}) > prev.completion_ns
               AND {EXCLUDE_SUBSCRIPTION_RENEWALS}{country_filter}{session_filter}
             GROUP BY prev.session_id"
        )
    } else {
        format!(
            "INSERT INTO funnel_next_survivors (session_id, completion_ns, page_ts)
             SELECT
                 payments.visitor_timestamp_id,
                 MIN(payments.paid_at) * {NANOS_PER_SECOND},
                 NULL
             FROM payments
             WHERE payments.visitor_timestamp_id IS NOT NULL
               AND payments.status = 'ok'
               AND payments.paid_at >= ?1
               AND payments.paid_at < ?2
               AND instr(',' || ?3 || ',', ',' || payments.payment_type || ',') > 0
               AND {EXCLUDE_SUBSCRIPTION_RENEWALS}{country_filter}{session_filter}
             GROUP BY payments.visitor_timestamp_id"
        )
    };
    let mut sql_params: Vec<&dyn ToSql> = vec![&start, &end, &payment_types];
    segments.append_country_params(&mut sql_params);
    conn.execute(&sql, params_from_iter(sql_params))
        .map_err(map_row_error)?;
    Ok(())
}

/// Buckets drop-off sessions (in `funnel_survivors` but not `funnel_next_survivors`)
/// by their next page (or exit / outbound), excluding navigations to the next
/// funnel step's page (expected funnel flow).
fn query_dropoff_from_temps(
    conn: &ProjectConnection,
    next_step_path_id: Option<i64>,
    outbound_event_id: Option<i64>,
) -> Result<FunnelStepDropoff, AuthError> {
    let sql = dropoff_destinations_sql(outbound_event_id.is_some());
    let mut sql_params: Vec<&dyn ToSql> = vec![&next_step_path_id];
    if let Some(outbound_id) = &outbound_event_id {
        sql_params.push(outbound_id);
    }

    let mut transitions = conn
        .with_query(sql, params_from_iter(sql_params), |mut rows| {
            let mut out = Vec::new();
            while let Some(row) = rows.next()? {
                out.push(FunnelDropoffRow {
                    path_id: row.get(0)?,
                    path: row.get(1)?,
                    hostname: row.get(2)?,
                    outbound_to: row.get(4)?,
                    visitors: row.get(3)?,
                    share: 0.0,
                });
            }
            Ok(out)
        })
        .map_err(map_row_error)?;

    let total_visitors: i64 = transitions.iter().map(|row| row.visitors).sum();
    for row in &mut transitions {
        row.share = ratio(row.visitors, total_visitors);
    }
    Ok(FunnelStepDropoff {
        total_visitors,
        transitions,
    })
}

/// Builds the drop-off destinations SQL over the adjacent survivor TEMP tables.
/// Each drop-off page is the `previous_timestamp_id` of at most one later page
/// view, so the destination join is 1:1. Expected funnel flow to the next step's
/// page is excluded.
///
/// When `with_outbound` is set, exits (`path_id IS NULL`) are probed for the
/// first revscope-outbound-click event on the drop-off page and split out by
/// destination. The `CASE` guard means the events lookup only runs for exit rows.
fn dropoff_destinations_sql(with_outbound: bool) -> &'static str {
    if with_outbound {
        "WITH dropoff_pages(page_ts) AS (
            SELECT s.page_ts
            FROM funnel_survivors s
            LEFT JOIN funnel_next_survivors n ON n.session_id = s.session_id
            WHERE n.session_id IS NULL
              AND s.page_ts IS NOT NULL
        ),
        next_destinations AS (
            SELECT
                dropoff_pages.page_ts AS page_ts,
                next.path_id AS path_id
            FROM dropoff_pages
            LEFT JOIN pageviews next
              ON next.previous_timestamp_id = dropoff_pages.page_ts
        ),
        classified AS (
            SELECT
                next_destinations.path_id AS path_id,
                CASE WHEN next_destinations.path_id IS NULL THEN (
                    SELECT jsonb_extract(e.metadata, '$.to')
                    FROM events e
                    WHERE e.page_timestamp_id = next_destinations.page_ts
                      AND e.event_name_id = ?2
                    ORDER BY e.timestamp_id DESC
                    LIMIT 1
                ) END AS outbound_to
            FROM next_destinations
        )
        SELECT
            classified.path_id AS path_id,
            p.path AS path,
            p.hostname AS hostname,
            COUNT(*) AS visitors,
            classified.outbound_to AS outbound_to
        FROM classified
        LEFT JOIN paths p
               ON p.id = classified.path_id AND p.deleted_at IS NULL
        WHERE ?1 IS NULL
           OR classified.path_id IS NULL
           OR classified.path_id <> ?1
        GROUP BY classified.path_id, classified.outbound_to
        ORDER BY visitors DESC, p.path ASC, p.hostname ASC"
    } else {
        "WITH dropoff_pages(page_ts) AS (
            SELECT s.page_ts
            FROM funnel_survivors s
            LEFT JOIN funnel_next_survivors n ON n.session_id = s.session_id
            WHERE n.session_id IS NULL
              AND s.page_ts IS NOT NULL
        ),
        next_destinations AS (
            SELECT next.path_id AS path_id
            FROM dropoff_pages
            LEFT JOIN pageviews next
              ON next.previous_timestamp_id = dropoff_pages.page_ts
        )
        SELECT
            destination.path_id AS path_id,
            p.path AS path,
            p.hostname AS hostname,
            COUNT(*) AS visitors,
            NULL AS outbound_to
        FROM next_destinations destination
        LEFT JOIN paths p
               ON p.id = destination.path_id AND p.deleted_at IS NULL
        WHERE ?1 IS NULL
           OR destination.path_id IS NULL
           OR destination.path_id <> ?1
        GROUP BY destination.path_id
        ORDER BY visitors DESC, p.path ASC, p.hostname ASC"
    }
}

/// Resolves the `revscope-outbound-click` event name id for this project, or `None` when
/// the project never recorded an outbound click (so callers can skip the
/// outbound lookup entirely).
pub(super) fn outbound_event_name_id(conn: &ProjectConnection) -> Result<Option<i64>, AuthError> {
    conn.query_row(
        "SELECT id FROM event_names WHERE name = ?1 AND deleted_at IS NULL",
        [OUTBOUND_CLICK_EVENT_NAME],
        |row| row.get::<_, i64>(0),
    )
    .optional()
    .map_err(map_row_error)
}

/// Sums amounts for the earliest qualifying payment of each final-step survivor,
/// matching the completion timestamp stored in `funnel_survivors`.
fn final_payment_revenue_from_temps(
    conn: &ProjectConnection,
    step: &FunnelStep,
    start: i64,
    end: i64,
    segments: &SegmentFilters,
) -> Result<i64, AuthError> {
    let country_filter = segments.country_exists_clause("payments.visitor_timestamp_id", 4);
    let session_filter = segments.session_filter_clause("payments.visitor_timestamp_id", None);
    let payment_types = payment_types_to_text(&step.payment_types);
    let sql = format!(
        "SELECT COALESCE(SUM(amount), 0)
         FROM (
             SELECT
                 payments.amount AS amount,
                 ROW_NUMBER() OVER (
                     PARTITION BY payments.visitor_timestamp_id
                     ORDER BY payments.paid_at ASC, payments.id ASC
                 ) AS rn
             FROM funnel_survivors s
             JOIN payments ON payments.visitor_timestamp_id = s.session_id
             WHERE payments.status = 'ok'
               AND payments.paid_at >= ?1
               AND payments.paid_at < ?2
               AND instr(',' || ?3 || ',', ',' || payments.payment_type || ',') > 0
               AND (payments.paid_at * {NANOS_PER_SECOND}) = s.completion_ns
               AND {EXCLUDE_SUBSCRIPTION_RENEWALS}{country_filter}{session_filter}
         ) matched
         WHERE rn = 1"
    );
    let mut sql_params: Vec<&dyn ToSql> = vec![&start, &end, &payment_types];
    segments.append_country_params(&mut sql_params);
    conn.query_row(&sql, params_from_iter(sql_params), |row| row.get(0))
        .map_err(map_row_error)
}

/// Counts visitors per step without retaining drop-off or revenue side data.
/// Used by tests to exercise the TEMP-table walk in isolation.
#[cfg(test)]
pub(super) fn compute_funnel_counts(
    conn: &ProjectConnection,
    steps: &[FunnelStep],
    start: i64,
    end: i64,
    segments: &SegmentFilters,
) -> Result<Vec<i64>, AuthError> {
    Ok(walk_funnel(
        conn,
        steps,
        start,
        end,
        segments,
        FunnelWalkOptions {
            compute_dropoff: false,
            compute_revenue: false,
            outbound_event_id: None,
        },
    )?
    .counts)
}
