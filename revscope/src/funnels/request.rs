//! Parsing and validating funnel queries and payloads.

use crate::comparison;
use crate::query_params::parse_report_query;
use crate::segment_filter::parse_segment_filters;

use super::types::{
    DeleteFunnelQuery, FunnelDataQuery, FunnelPreviewRequest, SaveFunnelRequest,
    SaveFunnelStepRequest,
};

pub(crate) fn parse_funnel_data_query(target: &str) -> Result<FunnelDataQuery, &'static str> {
    let mut funnel_id = None;
    let mut previous_start = None;
    let mut previous_end = None;
    let range = parse_report_query(target, |key, value| {
        match key {
            "funnel_id" => funnel_id = value.parse().ok(),
            "previous_start" => previous_start = value.parse().ok(),
            "previous_end" => previous_end = value.parse().ok(),
            _ => {}
        }
        Ok(())
    })?;
    let funnel_id = match funnel_id {
        None => None,
        Some(_) => Some(positive_id(funnel_id, "funnel_id")?),
    };
    let (start, end, previous_start, previous_end) =
        normalize_date_range(range.start, range.end, previous_start, previous_end)?;
    Ok(FunnelDataQuery {
        funnel_id,
        start,
        end,
        previous_start,
        previous_end,
        segments: parse_segment_filters(target),
    })
}

pub(crate) fn parse_delete_funnel_query(target: &str) -> Result<DeleteFunnelQuery, &'static str> {
    let funnel_id =
        crate::http::query_param(target, "funnel_id").and_then(|value| value.parse().ok());
    Ok(DeleteFunnelQuery {
        funnel_id: positive_id(funnel_id, "funnel_id")?,
    })
}

pub(crate) fn normalize_save_request(
    mut request: SaveFunnelRequest,
) -> Result<SaveFunnelRequest, &'static str> {
    request.name = request.name.trim().to_string();
    if request.name.is_empty() {
        return Err("funnel name is required");
    }
    if let Some(description) = request.description.as_mut() {
        *description = trimmed_optional(description).unwrap_or_default();
    }
    if request.steps.is_empty() {
        return Err("at least one funnel step is required");
    }
    normalize_funnel_steps(&mut request.steps)?;
    let (start, end, previous_start, previous_end) = normalize_date_range(
        request.start,
        request.end,
        request.previous_start,
        request.previous_end,
    )?;
    request.start = Some(start);
    request.end = Some(end);
    request.previous_start = Some(previous_start);
    request.previous_end = Some(previous_end);
    Ok(request)
}

/// Validates and canonicalizes funnel step definitions in place. Shared by the
/// save and preview flows so a drafted funnel is evaluated under exactly the
/// same rules it would be saved with.
fn normalize_funnel_steps(steps: &mut [SaveFunnelStepRequest]) -> Result<(), &'static str> {
    if steps.is_empty() {
        return Ok(());
    }
    let last_index = steps.len() - 1;
    for (index, step) in steps.iter_mut().enumerate() {
        step.step_type = step.step_type.trim().to_ascii_lowercase();
        if let Some(description) = step.description.as_mut() {
            *description = trimmed_optional(description).unwrap_or_default();
        }
        match step.step_type.as_str() {
            "pageview" => {
                positive_id(step.path_id, "path_id")?;
                if !step.payment_types.is_empty() {
                    return Err("pageview steps cannot include payment_types");
                }
                // The event is optional; when present it must reference a real event.
                if let Some(event_name_id) = step.event_name_id
                    && event_name_id <= 0
                {
                    return Err("event_name_id");
                }
                if step.is_landing && index != 0 {
                    return Err("only the first step can require sessions to start on its page");
                }
            }
            "payment" => {
                if index != last_index {
                    return Err("the payment step must be the final step");
                }
                if step.event_name_id.is_some() || step.path_id.is_some() || step.is_landing {
                    return Err("payment steps must only include payment_types");
                }
                if step.payment_types.is_empty() {
                    return Err("payment steps require at least one payment_type");
                }
                for payment_type in step.payment_types.iter_mut() {
                    *payment_type = payment_type.trim().to_ascii_lowercase();
                    if !is_allowed_payment_type(payment_type) {
                        return Err(
                            "payment_type must be single, subscription, refund, or chargeback",
                        );
                    }
                }
                step.payment_types.sort();
                step.payment_types.dedup();
            }
            _ => return Err("step_type must be pageview or payment"),
        }
    }
    Ok(())
}

pub(crate) fn normalize_preview_request(
    mut request: FunnelPreviewRequest,
) -> Result<FunnelPreviewRequest, &'static str> {
    if request.steps.is_empty() {
        return Err("at least one funnel step is required");
    }
    normalize_funnel_steps(&mut request.steps)?;
    let (start, end, previous_start, previous_end) = normalize_date_range(
        request.start,
        request.end,
        request.previous_start,
        request.previous_end,
    )?;
    request.start = Some(start);
    request.end = Some(end);
    request.previous_start = Some(previous_start);
    request.previous_end = Some(previous_end);
    Ok(request)
}

fn normalize_date_range(
    start: Option<i64>,
    end: Option<i64>,
    previous_start: Option<i64>,
    previous_end: Option<i64>,
) -> Result<(i64, i64, i64, i64), &'static str> {
    let (start, end) = comparison::validate_date_range(start, end)?;
    let period_seconds = end - start;
    let previous_start = previous_start.unwrap_or(start - period_seconds);
    let previous_end = previous_end.unwrap_or(start);
    if previous_end <= previous_start {
        return Err("previous_end must be after previous_start");
    }
    if previous_end - previous_start > comparison::MAX_RANGE_SECONDS {
        return Err("date range must be at most 366 days");
    }
    Ok((start, end, previous_start, previous_end))
}

fn positive_id(value: Option<i64>, name: &'static str) -> Result<i64, &'static str> {
    let value = value.ok_or(name)?;
    if value > 0 { Ok(value) } else { Err(name) }
}

pub(super) fn trimmed_optional(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn is_allowed_payment_type(payment_type: &str) -> bool {
    matches!(
        payment_type,
        "single" | "subscription" | "refund" | "chargeback"
    )
}
