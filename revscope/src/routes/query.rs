//! SQL access to a project's database, for the dashboard and for clients
//! holding a project-scoped query token, such as the MCP server.

use crate::auth::AuthError;
use crate::authorize::{Account, AuthorizedProject, OwnedProject};
use crate::http::Response;
use crate::query::{self, QueryError, QueryRequest};
use crate::query_token::{self, ProjectAuth};
use crate::schema;

use super::request::{ApiRequest, RouteResult};
use super::responses::{bad_request, map_auth_error, unauthorized_rejected};

/// A caller the query policy admitted by a session or a query token, with
/// what to evict should a handler go on to reject the session.
pub(super) struct QueryCaller {
    pub(super) project: AuthorizedProject,
    pub(super) auth: ProjectAuth,
    pub(super) session_cache_key: Option<u128>,
}

impl QueryCaller {
    fn error(&self, request: &ApiRequest, error: AuthError, context: &str) -> Response {
        match error {
            AuthError::InvalidSession | AuthError::InvalidCredentials => {
                unauthorized_rejected(&request.server_context, &self.auth, self.session_cache_key)
            }
            error => map_auth_error(error, context),
        }
    }
}

pub(super) fn query(request: &ApiRequest, caller: &QueryCaller) -> RouteResult {
    let payload = request.json_body::<QueryRequest>("invalid query payload")?;
    match query::query(request.context(), &caller.project, payload) {
        Ok(response) => Ok(Response::ok_json(&response)),
        Err(QueryError::Invalid(message)) => Err(bad_request(&message)),
        Err(QueryError::Auth(error)) => Err(caller.error(request, error, "query")),
    }
}

pub(super) fn schema(request: &ApiRequest, caller: &QueryCaller) -> RouteResult {
    let sql = schema::schema(request.context(), &caller.project)
        .map_err(|error| caller.error(request, error, "schema"))?;
    Ok(Response::with_content_type(
        "200 OK",
        "application/sql; charset=utf-8",
        sql,
        &[(
            "Content-Disposition",
            format!(
                "attachment; filename=\"{}-schema.sql\"",
                caller.project.id()
            ),
        )],
    ))
}

/// Mints a query token for the signed-in owner's project.
pub(super) fn query_token(
    request: &ApiRequest,
    account: &Account,
    project: &OwnedProject,
) -> RouteResult {
    let outcome = query_token::create_query_token(request.context(), account, project);
    request.session_json(account, outcome, "query_token")
}
