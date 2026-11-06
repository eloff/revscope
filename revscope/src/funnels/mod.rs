//! Funnels: ordered steps (pageviews, events, purchases) a visitor passes
//! through, with how many visitors reach each step and where the rest go.

mod report;
mod request;
mod store;
#[cfg(test)]
mod tests;
mod types;
mod walk;

use std::sync::Arc;

use crate::auth::AuthError;
use crate::authorize::{AuthorizedProject, WritableProject};
use crate::db::{open_project_database_read, open_project_database_write};
use crate::segment_filter::SegmentFilters;
use crate::server_context::ServerContext;

pub(crate) use request::{
    normalize_preview_request, normalize_save_request, parse_delete_funnel_query,
    parse_funnel_data_query,
};
pub(crate) use types::*;

pub(crate) fn funnels(
    server_context: Arc<ServerContext>,
    project: &AuthorizedProject,
) -> Result<FunnelsResponse, AuthError> {
    let conn = open_project_database_read(server_context.as_ref(), project.id())?;
    store::load_funnels(&conn)
}

pub(crate) fn funnel_options(
    server_context: Arc<ServerContext>,
    project: &AuthorizedProject,
) -> Result<FunnelOptionsResponse, AuthError> {
    let conn = open_project_database_read(server_context.as_ref(), project.id())?;
    store::load_funnel_options(&conn)
}

pub(crate) fn funnel_data(
    server_context: Arc<ServerContext>,
    project: &AuthorizedProject,
    query: FunnelDataQuery,
) -> Result<FunnelDataRouteResponse, AuthError> {
    let conn = open_project_database_read(server_context.as_ref(), project.id())?;
    report::build_funnel_data_route_response(&conn, &query)
}

/// Reports on a draft funnel's steps without saving it.
pub(crate) fn funnel_preview(
    server_context: Arc<ServerContext>,
    project: &AuthorizedProject,
    request: FunnelPreviewRequest,
) -> Result<FunnelDataResponse, AuthError> {
    let conn = open_project_database_read(server_context.as_ref(), project.id())?;
    report::build_funnel_preview_response(&conn, &request)
}

/// Creates or replaces a funnel, answering with it and its report for the
/// request's date range.
pub(crate) fn save_funnel(
    server_context: Arc<ServerContext>,
    project: &WritableProject,
    request: SaveFunnelRequest,
) -> Result<SaveFunnelResponse, AuthError> {
    let conn = open_project_database_write(server_context.as_ref(), project.id())?;
    let funnel_id = store::persist_funnel(&conn, &request)?;
    let funnel = store::load_funnel_by_id(&conn, funnel_id)?;
    // Dates are filled in by `normalize_save_request`.
    let query = FunnelDataQuery {
        funnel_id: Some(funnel_id),
        start: request.start.expect("start normalized"),
        end: request.end.expect("end normalized"),
        previous_start: request.previous_start.expect("previous_start normalized"),
        previous_end: request.previous_end.expect("previous_end normalized"),
        segments: SegmentFilters::default(),
    };
    let data = report::build_funnel_data_response(&conn, &query, funnel_id)?;
    Ok(SaveFunnelResponse { funnel, data })
}

/// Soft-deletes a funnel, answering with the funnels that remain.
pub(crate) fn delete_funnel(
    server_context: Arc<ServerContext>,
    project: &WritableProject,
    query: DeleteFunnelQuery,
) -> Result<FunnelsResponse, AuthError> {
    let conn = open_project_database_write(server_context.as_ref(), project.id())?;
    store::soft_delete_funnel(&conn, query.funnel_id)?;
    store::load_funnels(&conn)
}
