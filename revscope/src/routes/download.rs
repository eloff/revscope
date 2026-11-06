//! Downloading a project's SQLite database.

use crate::authorize::{Account, DownloadableProject};
use crate::download_db;

use super::request::{ApiRequest, RouteResult};
use super::responses::map_project_auth_error;

pub(super) fn download_db(request: &ApiRequest, project: &DownloadableProject) -> RouteResult {
    let (range, if_range) = (request.parsed.range, request.parsed.if_range);
    download_db::download_db(request.context(), project, range, if_range)
        .map_err(|error| map_project_auth_error(error, "download_db"))
}

/// Mints a token a browser can download with by navigating to a URL.
pub(super) fn download_db_token(
    request: &ApiRequest,
    account: &Account,
    project: &DownloadableProject,
) -> RouteResult {
    let outcome = download_db::create_download_token(request.context(), account, project);
    request.session_json(account, outcome, "download_db_token")
}
