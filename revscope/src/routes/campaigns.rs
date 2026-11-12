//! Ad campaigns: reports on them are a read of the project, while saving
//! campaigns and their spend needs the owner's session.

use serde_json::json;

use crate::authorize::{Account, AuthorizedProject, WritableProject};
use crate::campaign_spend::{self, SaveCampaignSpendRequest};
use crate::campaigns::{self, CampaignPreviewRequest, SaveCampaignRequest};
use crate::http::Response;

use super::request::{ApiRequest, RouteResult, project_json};
use super::responses::{bad_request, map_project_auth_error};

const CONTEXT: &str = "campaigns";

pub(super) fn campaigns(request: &ApiRequest, project: &AuthorizedProject) -> RouteResult {
    let query = request.retained_query(project, campaigns::parse_campaigns_query)?;
    project_json(
        campaigns::campaigns(request.context(), project, query),
        CONTEXT,
    )
}

pub(super) fn save_campaign(
    request: &ApiRequest,
    _account: &Account,
    project: &WritableProject,
) -> RouteResult {
    let payload = request.json_body::<SaveCampaignRequest>("invalid campaign payload")?;
    let payload = campaigns::normalize_save_request(payload).map_err(bad_request)?;
    project_json(
        campaigns::save_campaign(request.context(), project, payload),
        CONTEXT,
    )
}

pub(super) fn delete_campaign(
    request: &ApiRequest,
    _account: &Account,
    project: &WritableProject,
) -> RouteResult {
    let query = request.query(campaigns::parse_delete_campaign_query)?;
    campaigns::delete_campaign(request.context(), project, query)
        .map_err(|error| map_project_auth_error(error, CONTEXT))?;
    Ok(Response::ok_json(json!({ "ok": true })))
}

/// Reports on unsaved campaign rules, so a draft can be seen before it is saved.
pub(super) fn campaign_preview(request: &ApiRequest, project: &AuthorizedProject) -> RouteResult {
    let query = request.retained_query(project, campaigns::parse_campaigns_query)?;
    let payload =
        request.json_body::<CampaignPreviewRequest>("invalid campaign preview payload")?;
    let payload = campaigns::normalize_preview_request(payload).map_err(bad_request)?;
    project_json(
        campaigns::campaign_preview(request.context(), project, payload, &query),
        "campaign_preview",
    )
}

pub(super) fn campaign_spend(
    request: &ApiRequest,
    _account: &Account,
    project: &WritableProject,
) -> RouteResult {
    let payload =
        request.json_body::<SaveCampaignSpendRequest>("invalid campaign spend payload")?;
    let payload = campaign_spend::normalize_save_request(payload).map_err(bad_request)?;
    project_json(
        campaign_spend::save_campaign_spend(request.context(), project, payload),
        "campaign_spend",
    )
}
