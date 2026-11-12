//! The dashboard's read-only reports. Each is a GET whose query string names
//! the range and filters, behind the read policy: the project's owner or, for
//! the public demo, anyone.

use std::sync::Arc;

use serde::Serialize;

use crate::auth::AuthError;
use crate::authorize::AuthorizedProject;
use crate::http::Response;
use crate::server_context::ServerContext;
use crate::{
    audience, crawler_detail, crawler_report, events, landing_pages, overview, pages,
    payment_activity, payment_journey, payments, revenue_source, search_terms, transitions,
    visitors,
};

use super::request::{ApiRequest, RouteResult, project_json};
use super::responses::map_project_auth_error;

/// Parses the query and answers `report`'s result as JSON. `context` names
/// the report in logged errors.
fn report<Q, R: Serialize>(
    request: &ApiRequest,
    project: &AuthorizedProject,
    context: &str,
    parse: impl FnOnce(&str) -> Result<Q, &'static str>,
    report: impl FnOnce(Arc<ServerContext>, &AuthorizedProject, Q) -> Result<R, AuthError>,
) -> RouteResult {
    let query = request.retained_query(project, parse)?;
    project_json(report(request.context(), project, query), context)
}

/// Like [`report`], but serves the public demo's unfiltered overviews from
/// cache.
pub(super) fn overview(request: &ApiRequest, project: &AuthorizedProject) -> RouteResult {
    let query = request.retained_query(project, overview::parse_overview_query)?;
    let server_context = request.context();
    if let Some(body) =
        overview::take_cached_demo_overview(&server_context, request.project_id(), &query)
    {
        return Ok(Response::json("200 OK", body));
    }
    match overview::overview(server_context.clone(), project, &query) {
        Ok(response) => Ok(overview::respond_overview_json(
            &server_context,
            request.project_id(),
            &query,
            &response,
        )),
        Err(error) => Err(map_project_auth_error(error, "overview")),
    }
}

pub(super) fn conversions(request: &ApiRequest, project: &AuthorizedProject) -> RouteResult {
    report(
        request,
        project,
        "conversions",
        payments::parse_payments_query,
        payments::conversions,
    )
}

pub(super) fn payment_activity(request: &ApiRequest, project: &AuthorizedProject) -> RouteResult {
    // A fixed window back from now rather than a range the query names.
    let oldest = request.oldest_retained_second(project);
    report(
        request,
        project,
        "payment_activity",
        payment_activity::parse_payment_activity_query,
        |context, project, query| {
            payment_activity::payment_activity(context, project, query, oldest)
        },
    )
}

pub(super) fn payment_journey(request: &ApiRequest, project: &AuthorizedProject) -> RouteResult {
    // A journey is one payment, named by id rather than by a range.
    let oldest = request.oldest_retained_second(project);
    report(
        request,
        project,
        "payment_journey",
        payment_journey::parse_payment_journey_query,
        |context, project, query| payment_journey::payment_journey(context, project, query, oldest),
    )
}

pub(super) fn revenue_sources(request: &ApiRequest, project: &AuthorizedProject) -> RouteResult {
    report(
        request,
        project,
        "revenue_sources",
        revenue_source::parse_revenue_source_query,
        revenue_source::revenue_source,
    )
}

pub(super) fn search_terms(request: &ApiRequest, project: &AuthorizedProject) -> RouteResult {
    report(
        request,
        project,
        "search_terms",
        search_terms::parse_search_terms_query,
        search_terms::search_terms,
    )
}

pub(super) fn audience(request: &ApiRequest, project: &AuthorizedProject) -> RouteResult {
    report(
        request,
        project,
        "audience",
        audience::parse_audience_query,
        audience::audience,
    )
}

pub(super) fn pages(request: &ApiRequest, project: &AuthorizedProject) -> RouteResult {
    report(
        request,
        project,
        "pages",
        pages::parse_pages_query,
        pages::pages,
    )
}

pub(super) fn page_transitions(request: &ApiRequest, project: &AuthorizedProject) -> RouteResult {
    report(
        request,
        project,
        "page_transitions",
        transitions::parse_page_transitions_query,
        transitions::page_transitions,
    )
}

pub(super) fn landing_pages(request: &ApiRequest, project: &AuthorizedProject) -> RouteResult {
    report(
        request,
        project,
        "landing_pages",
        landing_pages::parse_landing_pages_query,
        landing_pages::landing_pages,
    )
}

pub(super) fn visitors(request: &ApiRequest, project: &AuthorizedProject) -> RouteResult {
    report(
        request,
        project,
        "visitors",
        visitors::parse_visitors_query,
        visitors::visitors,
    )
}

pub(super) fn visitor_journey(request: &ApiRequest, project: &AuthorizedProject) -> RouteResult {
    // A journey is one session, named by id rather than by a range.
    let oldest = request.oldest_retained_second(project);
    report(
        request,
        project,
        "visitor_journey",
        visitors::parse_visitor_journey_query,
        |context, project, query| visitors::visitor_journey(context, project, query, oldest),
    )
}

pub(super) fn events(request: &ApiRequest, project: &AuthorizedProject) -> RouteResult {
    report(
        request,
        project,
        "events",
        events::parse_events_query,
        events::events,
    )
}

pub(super) fn crawlers(request: &ApiRequest, project: &AuthorizedProject) -> RouteResult {
    report(
        request,
        project,
        "crawlers",
        crawler_report::parse_crawlers_query,
        crawler_report::crawlers,
    )
}

pub(super) fn crawler_detail(request: &ApiRequest, project: &AuthorizedProject) -> RouteResult {
    report(
        request,
        project,
        "crawler_detail",
        crawler_detail::parse_crawler_detail_query,
        crawler_detail::crawler_detail,
    )
}
