//! Saved funnels: listing and charting them is a read of the project, while
//! saving and deleting them needs the owner's session.

use crate::authorize::{Account, AuthorizedProject, WritableProject};
use crate::funnels::{self, FunnelPreviewRequest, SaveFunnelRequest};
use crate::plan_retention::{retained_comparison, retained_range};

use super::request::{ApiRequest, RouteResult, project_json};
use super::responses::bad_request;

const CONTEXT: &str = "funnels";

pub(super) fn funnels(request: &ApiRequest, project: &AuthorizedProject) -> RouteResult {
    project_json(funnels::funnels(request.context(), project), CONTEXT)
}

pub(super) fn save_funnel(
    request: &ApiRequest,
    _account: &Account,
    project: &WritableProject,
) -> RouteResult {
    let payload = request.json_body::<SaveFunnelRequest>("invalid funnel payload")?;
    let payload = funnels::normalize_save_request(payload)
        .map_err(|_| bad_request("invalid funnel payload"))?;
    project_json(
        funnels::save_funnel(request.context(), project, payload),
        CONTEXT,
    )
}

pub(super) fn delete_funnel(
    request: &ApiRequest,
    _account: &Account,
    project: &WritableProject,
) -> RouteResult {
    let query = request.query(funnels::parse_delete_funnel_query)?;
    project_json(
        funnels::delete_funnel(request.context(), project, query),
        CONTEXT,
    )
}

pub(super) fn funnel_data(request: &ApiRequest, project: &AuthorizedProject) -> RouteResult {
    let mut query = request.retained_query(project, funnels::parse_funnel_data_query)?;
    // The comparison window is the caller's to choose here, so it's held to retention too.
    (query.previous_start, query.previous_end) = retained_comparison(
        query.previous_start,
        query.previous_end,
        request.oldest_retained_second(project),
    );
    project_json(
        funnels::funnel_data(request.context(), project, query),
        "funnel_data",
    )
}

/// Charts an unsaved funnel, so a draft can be seen before it is saved.
pub(super) fn funnel_preview(request: &ApiRequest, project: &AuthorizedProject) -> RouteResult {
    let payload = request.json_body::<FunnelPreviewRequest>("invalid funnel preview payload")?;
    let mut payload = funnels::normalize_preview_request(payload)
        .map_err(|_| bad_request("invalid funnel preview payload"))?;
    // Normalizing filled every bound in.
    let oldest = request.oldest_retained_second(project);
    if let (Some(start), Some(end)) = (payload.start, payload.end) {
        let (start, _) = retained_range(start, end, oldest).map_err(bad_request)?;
        payload.start = Some(start);
    }
    if let (Some(start), Some(end)) = (payload.previous_start, payload.previous_end) {
        (payload.previous_start, payload.previous_end) = {
            let (start, end) = retained_comparison(start, end, oldest);
            (Some(start), Some(end))
        };
    }
    project_json(
        funnels::funnel_preview(request.context(), project, payload),
        "funnel_preview",
    )
}

pub(super) fn funnel_options(request: &ApiRequest, project: &AuthorizedProject) -> RouteResult {
    project_json(
        funnels::funnel_options(request.context(), project),
        "funnel_options",
    )
}
