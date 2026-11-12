//! SQL access to a project's database, for the dashboard and for clients
//! holding a project-scoped query token, such as the MCP server.

use std::time::Duration;

use crate::auth::AuthError;
use crate::authorize::{Account, AuthorizedProject, OwnedProject};
use crate::http::Response;
use crate::query::{self, QueryError, QueryRequest};
use crate::query_token::{self, ProjectAuth};
use crate::rate_limit::ip_key;
use crate::schema;

use super::request::{ApiRequest, RouteResult};
use super::responses::{bad_request, map_auth_error, rate_limited, unauthorized_rejected};

/// Most statements finish well inside their timeout, so a slot is usually free
/// again within a few seconds.
const QUERY_BUSY_RETRY_AFTER: Duration = Duration::from_secs(5);

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
    let reader = request.client_ip.map(ip_key).unwrap_or_default();
    match query::query(request.context(), &caller.project, &reader, payload) {
        Ok(response) => Ok(Response::ok_json(&response)),
        Err(QueryError::Invalid(message)) => Err(bad_request(&message)),
        Err(error @ QueryError::Busy) => Err(rate_limited(
            QUERY_BUSY_RETRY_AFTER,
            &error.client_message(),
        )),
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
