//! Account and project administration, all done by a signed-in user.

use crate::account_users;
use crate::authorize::{Account, AdminAccount, AdminProject, OwnedProject};
use crate::create_project::{self, CreateProjectRequest};
use crate::delete_project::{self, DeleteProjectRequest};
use crate::install_snippet;
use crate::invite_user::{self, InviteUserRequest};
use crate::remove_user::{self, RemoveUserRequest};
use crate::settings::{self, SaveSettingsRequest};
use crate::troubleshooting::{self, TroubleshootingCheckRequest};
use crate::update_user::{self, UpdateUserRequest};

use super::request::{ApiRequest, RouteResult};

pub(super) fn get_settings(
    request: &ApiRequest,
    account: &Account,
    project: &OwnedProject,
) -> RouteResult {
    let outcome = settings::get_settings(request.context(), project);
    request.session_json(account, outcome, "settings")
}

pub(super) fn save_settings(request: &ApiRequest, project: &AdminProject) -> RouteResult {
    let payload = request.json_body::<SaveSettingsRequest>("invalid settings payload")?;
    let outcome = settings::save_settings(request.context(), project, payload);
    request.session_json(project.admin(), outcome, "settings")
}

pub(super) fn account_users(request: &ApiRequest, admin: &AdminAccount) -> RouteResult {
    let outcome = account_users::account_users(request.context(), admin);
    request.session_json(admin, outcome, "account_users")
}

pub(super) fn invite_user(request: &ApiRequest, admin: &AdminAccount) -> RouteResult {
    let payload = request.json_body::<InviteUserRequest>("invalid invite payload")?;
    let outcome = invite_user::invite_user(request.context(), admin, payload);
    request.session_json(admin, outcome, "invite_user")
}

pub(super) fn update_user(request: &ApiRequest, admin: &AdminAccount) -> RouteResult {
    let payload = request.json_body::<UpdateUserRequest>("invalid user payload")?;
    let outcome = update_user::update_user(request.context(), admin, payload);
    request.session_json(admin, outcome, "update_user")
}

pub(super) fn remove_user(request: &ApiRequest, admin: &AdminAccount) -> RouteResult {
    let payload = request.json_body::<RemoveUserRequest>("invalid user payload")?;
    let outcome = remove_user::remove_user(request.context(), admin, payload);
    request.session_json(admin, outcome, "remove_user")
}

pub(super) fn create_project(request: &ApiRequest, admin: &AdminAccount) -> RouteResult {
    let payload = request.json_body::<CreateProjectRequest>("invalid project payload")?;
    let outcome = create_project::create_project(request.context(), admin, payload);
    request.session_json(admin, outcome, "create_project")
}

pub(super) fn delete_project(request: &ApiRequest, project: &AdminProject) -> RouteResult {
    let payload = request.json_body::<DeleteProjectRequest>("invalid delete payload")?;
    let outcome = delete_project::delete_project(request.context(), project, payload);
    request.session_json(project.admin(), outcome, "delete_project")
}

pub(super) fn install(
    request: &ApiRequest,
    account: &Account,
    project: &OwnedProject,
) -> RouteResult {
    let outcome = install_snippet::install(request.context(), project, request.parsed.host);
    request.session_json(account, outcome, "install")
}

pub(super) fn troubleshooting(
    request: &ApiRequest,
    account: &Account,
    project: &OwnedProject,
) -> RouteResult {
    let outcome = troubleshooting::get_status(request.context(), project);
    request.session_json(account, outcome, "troubleshooting")
}

pub(super) fn troubleshooting_check(
    request: &ApiRequest,
    account: &Account,
    project: &OwnedProject,
) -> RouteResult {
    let payload = request
        .json_body::<TroubleshootingCheckRequest>("invalid troubleshooting check payload")?;
    let outcome = troubleshooting::run_check(request.context(), project, payload);
    request.session_json(account, outcome, "troubleshooting_check")
}

pub(super) fn troubleshooting_ingest(
    request: &ApiRequest,
    account: &Account,
    project: &OwnedProject,
) -> RouteResult {
    let query = request.query(troubleshooting::parse_ingest_query)?;
    let outcome = troubleshooting::poll_ingest(request.context(), project, query);
    request.session_json(account, outcome, "troubleshooting_ingest")
}
