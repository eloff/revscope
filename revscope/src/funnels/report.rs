//! Assembling a funnel's report: per-step visitors against the comparison
//! period, drop-off, and revenue per visitor.

use rusqlite::{OptionalExtension, params};

use crate::auth::{AuthError, map_row_error};
use crate::comparison;
use crate::db::ProjectConnection;
use crate::metrics::ratio;
use crate::segment_filter::SegmentFilters;
use crate::util::seconds_since_epoch;

use super::store::{load_funnel_steps, load_funnels};
use super::types::{
    FunnelDataBootstrapResponse, FunnelDataQuery, FunnelDataResponse, FunnelDataRouteResponse,
    FunnelPreviewRequest, FunnelStep, FunnelStepData, SaveFunnelStepRequest,
};
use super::walk::{FunnelWalkOptions, outbound_event_name_id, walk_funnel};

/// The reported range and the one it is compared against.
#[derive(Debug, Clone, Copy)]
struct FunnelPeriods {
    start: i64,
    end: i64,
    previous_start: i64,
    previous_end: i64,
}

/// Reports on the funnel the query names, or with no funnel named, lists the
/// funnels with a report on the first, so the page loads in one request.
pub(super) fn build_funnel_data_route_response(
    conn: &ProjectConnection,
    query: &FunnelDataQuery,
) -> Result<FunnelDataRouteResponse, AuthError> {
    match query.funnel_id {
        Some(funnel_id) => Ok(FunnelDataRouteResponse::Data(build_funnel_data_response(
            conn, query, funnel_id,
        )?)),
        None => {
            let funnels = load_funnels(conn)?.funnels;
            let data = match funnels.first().map(|funnel| funnel.id) {
                Some(funnel_id) => Some(build_funnel_data_response(conn, query, funnel_id)?),
                None => None,
            };
            Ok(FunnelDataRouteResponse::Bootstrap(
                FunnelDataBootstrapResponse { funnels, data },
            ))
        }
    }
}

pub(super) fn build_funnel_preview_response(
    conn: &ProjectConnection,
    request: &FunnelPreviewRequest,
) -> Result<FunnelDataResponse, AuthError> {
    let mut steps = draft_steps_to_funnel_steps(&request.steps);
    enrich_funnel_step_labels(conn, &mut steps)?;
    // Dates are filled in by `normalize_preview_request`.
    let periods = FunnelPeriods {
        start: request.start.expect("start normalized"),
        end: request.end.expect("end normalized"),
        previous_start: request.previous_start.expect("previous_start normalized"),
        previous_end: request.previous_end.expect("previous_end normalized"),
    };
    let mut segments = SegmentFilters::from_country_codes(&request.countries);
    segments.set_sources_from_strings(&request.sources);
    segments.set_pages_from_ids(&request.pages);
    segments.set_landing_pages_from_ids(&request.landing_pages);
    segments.set_events_from_ids(&request.events);
    segments.set_attributes_from_params(&request.attributes);
    // Draft funnels are not persisted, so there is no funnel id to report.
    compute_funnel_data(
        conn,
        0,
        &steps,
        periods,
        &segments,
        seconds_since_epoch() as i64,
    )
}

/// Maps normalized draft step requests onto the internal `FunnelStep` shape used
/// by the calculation. Synthetic ids/order come from the list position; label
/// fields (event_name/path/hostname) are left unset because the editor renders
/// its own labels and only consumes per-step visitor counts.
fn draft_steps_to_funnel_steps(steps: &[SaveFunnelStepRequest]) -> Vec<FunnelStep> {
    steps
        .iter()
        .enumerate()
        .map(|(index, step)| FunnelStep {
            id: index as i64,
            order: index as i64,
            step_type: step.step_type.clone(),
            event_name_id: step.event_name_id,
            event_name: None,
            path_id: step.path_id,
            path: None,
            hostname: None,
            is_landing: step.is_landing,
            payment_types: step.payment_types.clone(),
            description: step.description.clone(),
        })
        .collect()
}

pub(super) fn build_funnel_data_response(
    conn: &ProjectConnection,
    query: &FunnelDataQuery,
    funnel_id: i64,
) -> Result<FunnelDataResponse, AuthError> {
    build_funnel_data_response_at(conn, query, funnel_id, seconds_since_epoch() as i64)
}

pub(super) fn build_funnel_data_response_at(
    conn: &ProjectConnection,
    query: &FunnelDataQuery,
    funnel_id: i64,
    now: i64,
) -> Result<FunnelDataResponse, AuthError> {
    let steps = load_funnel_steps(conn, funnel_id)?;
    let periods = FunnelPeriods {
        start: query.start,
        end: query.end,
        previous_start: query.previous_start,
        previous_end: query.previous_end,
    };
    compute_funnel_data(conn, funnel_id, &steps, periods, &query.segments, now)
}

/// Builds a funnel data response from an already-resolved list of steps. Shared
/// by the persisted `funnel_data` route (steps loaded from the db) and the
/// stateless `funnel_preview` route (steps supplied by the editor), so both
/// produce identical per-step visitor counts, drop-off rates, and destinations.
fn compute_funnel_data(
    conn: &ProjectConnection,
    funnel_id: i64,
    steps: &[FunnelStep],
    periods: FunnelPeriods,
    segments: &SegmentFilters,
    now: i64,
) -> Result<FunnelDataResponse, AuthError> {
    let FunnelPeriods {
        start,
        end,
        previous_start,
        previous_end,
    } = periods;
    let final_payment_step = steps.last().filter(|step| step.step_type == "payment");
    // Resolve once; destination SQL skips the outbound probe when absent.
    let outbound_event_id = outbound_event_name_id(conn)?;
    let current = walk_funnel(
        conn,
        steps,
        start,
        end,
        segments,
        FunnelWalkOptions {
            compute_dropoff: true,
            compute_revenue: final_payment_step.is_some(),
            outbound_event_id,
        },
    )?;
    let previous = walk_funnel(
        conn,
        steps,
        previous_start,
        previous_end,
        segments,
        FunnelWalkOptions {
            compute_dropoff: false,
            compute_revenue: false,
            outbound_event_id: None,
        },
    )?;
    let factor = comparison::normalization_factor(start, end, now);

    let mut data = Vec::with_capacity(steps.len());
    for (index, step) in steps.iter().enumerate() {
        let is_last = index + 1 == steps.len();
        let visitors = current.counts[index];
        let previous_visitors = previous.counts[index];
        let dropoff_rate = if is_last {
            None
        } else {
            Some(ratio(visitors - current.counts[index + 1], visitors))
        };
        data.push(FunnelStepData {
            step_id: step.id,
            order: step.order,
            step_type: step.step_type.clone(),
            name: step_label(step),
            description: step.description.clone(),
            path: step.path.clone(),
            event_name: step.event_name.clone(),
            path_id: transition_path_id_for_step(step)?,
            visitors,
            previous_visitors,
            visitors_change: comparison::Change::absolute(
                visitors as f64,
                previous_visitors as f64,
                factor,
            ),
            dropoff_rate,
            dropoff: current.dropoffs.get(index).cloned().flatten(),
            revenue_per_visitor: current
                .total_revenue
                .map(|revenue| if visitors > 0 { revenue / visitors } else { 0 }),
        });
    }
    Ok(FunnelDataResponse {
        funnel_id,
        steps: data,
        total_revenue: current.total_revenue,
    })
}

/// Returns the source path used to lazy-load page transitions for a funnel
/// step. Pageview steps are anchored to their configured page; payment steps
/// have no pageview anchor.
fn transition_path_id_for_step(step: &FunnelStep) -> Result<Option<i64>, AuthError> {
    match step.step_type.as_str() {
        // Pageview steps (with or without an event) are anchored to their
        // configured page, so we always use it as the hover source.
        "pageview" => Ok(step.path_id),
        "payment" => Ok(None),
        _ => Err(AuthError::InvalidRequest("invalid funnel step type")),
    }
}

fn enrich_funnel_step_labels(
    conn: &ProjectConnection,
    steps: &mut [FunnelStep],
) -> Result<(), AuthError> {
    for step in steps.iter_mut() {
        if step.path.is_none()
            && let Some(path_id) = step.path_id
        {
            step.path = conn
                .query_row(
                    "SELECT path FROM paths WHERE id = ?1 AND deleted_at IS NULL",
                    params![path_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(map_row_error)?;
        }
        if step.event_name.is_none()
            && let Some(event_name_id) = step.event_name_id
        {
            step.event_name = conn
                .query_row(
                    "SELECT name FROM event_names WHERE id = ?1 AND deleted_at IS NULL",
                    params![event_name_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(map_row_error)?;
        }
    }
    Ok(())
}

fn step_label(step: &FunnelStep) -> String {
    match step.step_type.as_str() {
        "pageview" => {
            let path = step
                .path
                .clone()
                .unwrap_or_else(|| "Unknown page".to_string());
            match &step.event_name {
                Some(event) => format!("{path} · {event}"),
                None => path,
            }
        }
        "payment" => "Purchase".to_string(),
        _ => "Unknown step".to_string(),
    }
}
