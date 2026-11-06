//! Saved funnels: listing and charting them is a read of the project, while
//! saving and deleting them needs the owner's session.

use crate::authorize::{Account, AuthorizedProject, WritableProject};
use crate::funnels::{self, FunnelPreviewRequest, SaveFunnelRequest};

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
    let query = request.query(funnels::parse_funnel_data_query)?;
    project_json(
        funnels::funnel_data(request.context(), project, query),
        "funnel_data",
    )
}

/// Charts an unsaved funnel, so a draft can be seen before it is saved.
pub(super) fn funnel_preview(request: &ApiRequest, project: &AuthorizedProject) -> RouteResult {
    let payload = request.json_body::<FunnelPreviewRequest>("invalid funnel preview payload")?;
    let payload = funnels::normalize_preview_request(payload)
        .map_err(|_| bad_request("invalid funnel preview payload"))?;
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
