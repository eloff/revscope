//! The Google Analytics import section of Settings, admin only, plus what the
//! dashboard reads to say when imported data is left out.

use crate::authorize::{AdminProject, AuthorizedProject};
use crate::email_verification::require_verified;
use crate::ga_import::{self, ConnectRequest, ImportedDataSettings, StartImportRequest};

use super::request::{ApiRequest, RouteResult, project_json};

const CONTEXT: &str = "ga_import";

pub(super) fn list(request: &ApiRequest, project: &AdminProject) -> RouteResult {
    request.session_json(
        project.admin(),
        ga_import::list_imports(request.context(), project),
        CONTEXT,
    )
}

pub(super) fn start(request: &ApiRequest, project: &AdminProject) -> RouteResult {
    let payload = request.json_body::<StartImportRequest>("invalid import payload")?;
    request.session_json(
        project.admin(),
        require_verified(project.admin().user())
            .and_then(|()| ga_import::start_import(request.context(), project, payload)),
        CONTEXT,
    )
}

pub(super) fn delete(request: &ApiRequest, project: &AdminProject) -> RouteResult {
    let import_id = request.query(ga_import::parse_import_id)?;
    request.session_json(
        project.admin(),
        ga_import::delete_import(request.context(), project, import_id),
        CONTEXT,
    )
}

pub(super) fn cancel(request: &ApiRequest, project: &AdminProject) -> RouteResult {
    let import_id = request.query(ga_import::parse_import_id)?;
    request.session_json(
        project.admin(),
        ga_import::cancel_import(request.context(), project, import_id),
        CONTEXT,
    )
}

pub(super) fn oauth_url(request: &ApiRequest, project: &AdminProject) -> RouteResult {
    request.session_json(
        project.admin(),
        require_verified(project.admin().user())
            .and_then(|()| ga_import::oauth_url(request.context(), project)),
        CONTEXT,
    )
}

pub(super) fn connect(request: &ApiRequest, project: &AdminProject) -> RouteResult {
    let payload = request.json_body::<ConnectRequest>("invalid connect payload")?;
    request.session_json(
        project.admin(),
        require_verified(project.admin().user())
            .and_then(|()| ga_import::connect(request.context(), project, payload)),
        CONTEXT,
    )
}

pub(super) fn save_settings(request: &ApiRequest, project: &AdminProject) -> RouteResult {
    let payload = request.json_body::<ImportedDataSettings>("invalid settings payload")?;
    request.session_json(
        project.admin(),
        ga_import::save_settings(request.context(), project, payload),
        CONTEXT,
    )
}

pub(super) fn coverage(request: &ApiRequest, project: &AuthorizedProject) -> RouteResult {
    let query = request.retained_query(project, ga_import::parse_coverage_query)?;
    project_json(
        ga_import::imported_coverage(request.context(), project, query),
        "imported_coverage",
    )
}
