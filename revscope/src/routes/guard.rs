//! Runs the credential check a route's [`AuthPolicy`] names, then calls its
//! handler with the grant that check produced. The only path from a request
//! to a handler, so a handler cannot skip or swap its check.

use crate::auth::{AuthError, SessionCookie};
use crate::authorize::{
    Account, authorize_account, authorize_admin, authorize_admin_project, authorize_chat,
    authorize_download_token, authorize_query_project, authorize_read_project,
    authorize_session_project, authorize_session_project_mut, authorize_session_project_or_demo,
};
use crate::download_db;
use crate::http::Response;

use super::query::QueryCaller;
use super::request::{ApiRequest, RouteResult};
use super::responses::{
    map_auth_error, map_project_auth_error, unauthorized_invalid_session,
    unauthorized_not_authenticated, unauthorized_rejected,
};
use super::session::{prepare_project_auth, resolve_project_auth, session_cache_key};
use super::table::AuthPolicy;

pub(super) fn run(request: &ApiRequest, policy: &AuthPolicy) -> RouteResult {
    let context = request.context();
    match *policy {
        AuthPolicy::Public(handle) => handle(request),
        AuthPolicy::Account(handle) => {
            let account = require_account(request)?;
            handle(request, &account)
        }
        AuthPolicy::Admin(handle) => {
            let account = require_account(request)?;
            let admin = authorize_admin(&account).map_err(|error| project_error(request, error))?;
            handle(request, &admin)
        }
        AuthPolicy::AdminProjectWrite(handle) => {
            let account = require_account(request)?;
            let admin = authorize_admin(&account).map_err(|error| project_error(request, error))?;
            let project = authorize_admin_project(context, &admin, request.project_id())
                .map_err(|error| project_error(request, error))?;
            handle(request, &project)
        }
        AuthPolicy::OwnProject(handle) => {
            let account = require_account(request)?;
            let project = authorize_session_project(context, &account, request.project_id())
                .map_err(|error| project_error(request, error))?;
            handle(request, &account, &project)
        }
        AuthPolicy::OwnProjectOrDemo(handle) => {
            let account = require_account(request)?;
            let project =
                authorize_session_project_or_demo(context, &account, request.project_id())
                    .map_err(|error| project_error(request, error))?;
            handle(request, &account, &project)
        }
        AuthPolicy::OwnProjectWrite(handle) => {
            let account = require_account(request)?;
            let project = authorize_session_project_mut(context, &account, request.project_id())
                .map_err(|error| project_error(request, error))?;
            handle(request, &account, &project)
        }
        AuthPolicy::Read(handle) => {
            let account = optional_account(request)?;
            let project =
                authorize_read_project(context, account.as_ref(), request.project_id())
                    .map_err(|error| caller_project_error(request, account.as_ref(), error))?;
            handle(request, &project)
        }
        AuthPolicy::Query(handle) => handle(request, &authorize_query_caller(request)?),
        AuthPolicy::Download(handle) => {
            // A browser navigating to the download URL sends no
            // `Authorization` header, so without a session it presents a
            // download token in the query string instead.
            let project = match optional_account(request)? {
                Some(account) => {
                    authorize_session_project_or_demo(context, &account, request.project_id())
                        .map_err(|error| project_error(request, error))?
                }
                None => {
                    let query = download_db::parse_download_db_query(request.target());
                    let token = query
                        .token
                        .as_deref()
                        .ok_or_else(unauthorized_not_authenticated)?;
                    authorize_download_token(context, token, request.project_id())
                        .map_err(|error| project_error(request, error))?
                }
            };
            handle(request, &project)
        }
        AuthPolicy::Chat(handle) => {
            let account = optional_account(request)?;
            let grant = authorize_chat(
                context,
                account.as_ref(),
                request.project_id(),
                request.client_ip,
            )
            .map_err(|error| caller_project_error(request, account.as_ref(), error))?;
            handle(request, &grant)
        }
    }
}

/// Names the route in logged errors.
fn route_name(request: &ApiRequest) -> String {
    format!("{:?}", request.resolved.route)
}

/// A refusal of a verified caller: the session is fine, so it stays cached.
fn project_error(request: &ApiRequest, error: AuthError) -> Response {
    map_project_auth_error(error, &route_name(request))
}

/// [`project_error`] for routes that also admit anonymous callers. Without a
/// session, a project that isn't public may well be one logging in would open,
/// so the caller is told to authenticate rather than that it doesn't exist.
fn caller_project_error(
    request: &ApiRequest,
    account: Option<&Account>,
    error: AuthError,
) -> Response {
    match (account, error) {
        (None, AuthError::NoProjectAccess) => unauthorized_not_authenticated(),
        (_, error) => project_error(request, error),
    }
}

fn require_account(request: &ApiRequest) -> Result<Account, Response> {
    optional_account(request)?.ok_or_else(unauthorized_not_authenticated)
}

/// No session is fine, but one that cannot be valid is not: a signed-in
/// caller whose session died is told so rather than treated as anonymous.
fn optional_account(request: &ApiRequest) -> Result<Option<Account>, Response> {
    let Some(session) = request.session() else {
        return Ok(None);
    };
    verify_session(request, &session).map(Some)
}

fn verify_session(request: &ApiRequest, session: &SessionCookie) -> Result<Account, Response> {
    let cache_key = session_cache_key(session).ok_or_else(unauthorized_invalid_session)?;
    authorize_account(request.context(), session).map_err(|error| match error {
        AuthError::InvalidSession | AuthError::InvalidCredentials => {
            request.server_context.session_cache.evict(cache_key);
            unauthorized_invalid_session()
        }
        error => map_auth_error(error, &route_name(request)),
    })
}

fn authorize_query_caller(request: &ApiRequest) -> Result<QueryCaller, Response> {
    let auth = resolve_project_auth(request.parsed.cookie, request.parsed.authorization)
        .ok_or_else(unauthorized_not_authenticated)?;
    let session_cache_key = prepare_project_auth(&auth)?;
    let project = authorize_query_project(request.context(), &auth, request.project_id()).map_err(
        |error| match error {
            AuthError::InvalidSession | AuthError::InvalidCredentials => {
                unauthorized_rejected(&request.server_context, &auth, session_cache_key)
            }
            error => map_auth_error(error, &route_name(request)),
        },
    )?;
    Ok(QueryCaller {
        project,
        auth,
        session_cache_key,
    })
}
