//! The `/v2` API: resolving a path to its route, checking the credentials the
//! route table declares for its method, and the synchronous handlers that
//! answer each one. A route's actual work lives in its own module; the
//! handlers here read the input and call it with the grant the guard made.

mod account;
mod campaigns;
mod chat;
mod dashboard;
mod download;
mod funnels;
mod ga_import;
mod guard;
mod id_token_sign_in;
mod mcp;
mod project_admin;
mod query;
pub(crate) mod request;
mod resolve;
mod responses;
mod session;
mod sign_in_providers;
mod table;

use std::net::IpAddr;
use std::sync::Arc;

use crate::http::{ParsedRequest, Response};
use crate::server_context::ServerContext;

use request::ApiRequest;
pub(crate) use resolve::{
    ApiRoute, IngestRoute, PathPrefix, ResolvedApiRoute, ingest_project_name, resolve_api_route,
    resolve_ingest_route,
};
pub(crate) use responses::map_auth_error;
use responses::method_not_allowed;
#[cfg(feature = "billing")]
pub(crate) use responses::too_many_requests;
pub(crate) use session::request_session_key;

/// Synchronous route handler. Designed to run inside `tokio::task::spawn_blocking`
/// so that the per-route sqlite work, request deserialization, and response
/// serialization all happen off the async runtime threads.
pub(crate) fn handle_api_route(
    resolved: ResolvedApiRoute,
    parsed: &ParsedRequest<'_>,
    body: &[u8],
    client_ip: Option<IpAddr>,
    forwarded_headers_trusted: bool,
    server_context: Arc<ServerContext>,
) -> Response {
    let request = ApiRequest {
        resolved: &resolved,
        parsed,
        body,
        client_ip,
        forwarded_headers_trusted,
        server_context,
    };
    let methods = table::methods(resolved.route);
    if methods.is_empty() {
        // `/v2/mcp` is answered by `handle_mcp_request` before the blocking
        // dispatch, so reaching here means the two disagree about the path.
        return Response::internal_server_error().with_internal_error(format!(
            "{:?} reached the blocking route table",
            resolved.route
        ));
    }
    let Some((_, policy)) = methods
        .iter()
        .find(|(method, _)| *method == request.method())
    else {
        let allowed: Vec<&str> = methods.iter().map(|(method, _)| *method).collect();
        return method_not_allowed(&format!("use {}", allowed.join(" or ")));
    };
    if policy.prefix() != resolved.prefix {
        return Response::internal_server_error().with_internal_error(format!(
            "{} {:?} is guarded by a {:?} policy under the {:?} prefix",
            request.method(),
            resolved.route,
            policy.prefix(),
            resolved.prefix
        ));
    }
    let outcome = guard::run(&request, policy);
    outcome.unwrap_or_else(|rejection| rejection)
}
