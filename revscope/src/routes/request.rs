use std::net::IpAddr;
use std::sync::Arc;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::auth::{AuthError, SessionCookie};
use crate::authorize::{Account, AuthorizedProject};
use crate::http::{ParsedRequest, Response};
use crate::server_context::ServerContext;

use super::ResolvedApiRoute;
use super::responses::{
    bad_request, map_auth_error, map_project_auth_error, unauthorized_invalid_session,
};
use super::session::{resolve_session, session_cache_key};

/// What a route handler answers with. Both arms are a response to send: `Err`
/// is an early rejection, so handlers can bail out of each step with `?`.
pub(crate) type RouteResult = Result<Response, Response>;

/// One API request with its resolved route, plus the steps route handlers
/// share for reading it. Credentials are checked before a handler runs, by
/// [`super::guard`], which hands the handler its grant.
pub(crate) struct ApiRequest<'a> {
    pub(crate) resolved: &'a ResolvedApiRoute,
    pub(crate) parsed: &'a ParsedRequest<'a>,
    pub(crate) body: &'a [u8],
    pub(crate) client_ip: Option<IpAddr>,
    #[cfg_attr(not(feature = "billing"), allow(dead_code))]
    pub(crate) forwarded_headers_trusted: bool,
    pub(crate) server_context: Arc<ServerContext>,
}

impl ApiRequest<'_> {
    pub(crate) fn method(&self) -> &str {
        self.parsed.method
    }

    pub(crate) fn target(&self) -> &str {
        self.parsed.target
    }

    pub(crate) fn context(&self) -> Arc<ServerContext> {
        self.server_context.clone()
    }

    /// The project id from the path. The resolver only yields project-scoped
    /// routes with one, so its absence is a routing bug.
    pub(crate) fn project_id(&self) -> &str {
        self.resolved
            .project_id
            .as_deref()
            .expect("project routes resolve with a project id")
    }

    pub(crate) fn query<Q>(
        &self,
        parse: impl FnOnce(&str) -> Result<Q, &'static str>,
    ) -> Result<Q, Response> {
        parse(self.target()).map_err(bad_request)
    }

    /// Like [`Self::query`], for a report on `project`: a `start` older than
    /// the project's retention is moved up to it before `parse` reads it.
    pub(crate) fn retained_query<Q>(
        &self,
        project: &AuthorizedProject,
        parse: impl FnOnce(&str) -> Result<Q, &'static str>,
    ) -> Result<Q, Response> {
        let target = crate::query_params::retained_target(
            self.target(),
            self.oldest_retained_second(project),
        )
        .map_err(bad_request)?;
        parse(&target).map_err(bad_request)
    }

    /// The first second `project`'s reports may show, or `None` for all of it.
    pub(crate) fn oldest_retained_second(&self, project: &AuthorizedProject) -> Option<i64> {
        let months = project.retention_months()?;
        let timezone = crate::timezone::project_timezone(self.context(), project.key());
        crate::plan_retention::oldest_retained_second(
            Some(months),
            &timezone,
            crate::util::unix_seconds(),
        )
    }

    pub(crate) fn json_body<T: DeserializeOwned>(
        &self,
        invalid_message: &str,
    ) -> Result<T, Response> {
        serde_json::from_slice(self.body).map_err(|_| bad_request(invalid_message))
    }

    /// The session the request presents, if any, without validating it.
    /// Only for routes that act on a session without trusting it, like logout;
    /// everything else takes the [`Account`] its guard verified.
    pub(crate) fn session(&self) -> Option<SessionCookie> {
        resolve_session(self.parsed.cookie, self.parsed.authorization)
    }

    /// Maps a failure from work done for a session. A session Postgres
    /// rejected is evicted from the cache so the next request re-checks it.
    pub(crate) fn session_error(
        &self,
        session: Option<&SessionCookie>,
        error: AuthError,
        context: &str,
    ) -> Response {
        match error {
            AuthError::InvalidSession | AuthError::InvalidCredentials => {
                if let Some(cache_key) = session.and_then(session_cache_key) {
                    self.server_context.session_cache.evict(cache_key);
                }
                unauthorized_invalid_session()
            }
            error => map_auth_error(error, context),
        }
    }

    /// Answers a session-authenticated handler's outcome as JSON, via
    /// [`Self::session_error`] on failure.
    pub(crate) fn session_json<T: Serialize>(
        &self,
        account: &Account,
        outcome: Result<T, AuthError>,
        context: &str,
    ) -> RouteResult {
        outcome
            .map(Response::ok_json)
            .map_err(|error| self.session_error(Some(account.session()), error, context))
    }
}

/// Answers a project handler's outcome as JSON.
pub(crate) fn project_json<T: Serialize>(
    outcome: Result<T, AuthError>,
    context: &str,
) -> RouteResult {
    outcome
        .map(Response::ok_json)
        .map_err(|error| map_project_auth_error(error, context))
}
