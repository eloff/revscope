//! The search terms report: Search Console queries with the revenue their
//! clicks brought, the clicks a better click-through rate would have won, and
//! queries split across competing landing pages.

mod fetch;
mod opportunities;
mod rows;
#[cfg(test)]
mod tests;

use std::cmp::Ordering;
use std::sync::Arc;

use fxhash::FxHashMap;

use crate::auth::AuthError;
use crate::authorize::AuthorizedProject;
use crate::comparison;
use crate::db::{ProjectConnection, open_project_database_read};
use crate::query_params::{SortDirection, SortedPage, cmp_f64, paginate, parse_report_query};
use crate::segment_filter::{SegmentFilters, parse_segment_filters};
use crate::server_context::ServerContext;
use crate::util::seconds_since_epoch;

use fetch::{attach_cannibalized_clicks, attach_landing_paths, fetch_search_terms_metrics};
use opportunities::{
    attach_ctr_opportunities, mark_search_term_highlights, promote_highlighted_terms,
};
use rows::{
    RawTerm, SearchTermsPeriod, SearchTermsResponse, click_through_rate, mobile_share,
    summary_with_changes, term_with_changes,
};

pub(crate) const SEARCH_TERMS_PAGE_SIZE: i64 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SearchTermsOrderBy {
    Query,
    LandingPaths,
    Position,
    Impressions,
    Clicks,
    Ctr,
    Mobile,
    /// Alias of [`SearchTermsOrderBy::Clicks`]: estimated revenue is proportional
    /// to clicks within a period (shared channel revenue / total clicks).
    EstimatedRevenue,
}

pub(crate) struct SearchTermsQuery {
    pub(crate) start: i64,
    pub(crate) end: i64,
    pub(crate) offset: i64,
    pub(crate) order_by: SearchTermsOrderBy,
    pub(crate) direction: SortDirection,
    pub(crate) segments: SegmentFilters,
}

pub(crate) fn parse_search_terms_query(target: &str) -> Result<SearchTermsQuery, &'static str> {
    let mut sort = SortedPage::new(SearchTermsOrderBy::EstimatedRevenue, parse_order_by);
    let range = parse_report_query(target, |key, value| sort.accept(key, value).map(drop))?;
    let (start, end) = range.validate()?;
    Ok(SearchTermsQuery {
        start,
        end,
        offset: sort.offset,
        order_by: sort.order_by,
        direction: sort.direction,
        segments: parse_segment_filters(target),
    })
}

pub(crate) fn search_terms(
    server_context: Arc<ServerContext>,
    project: &AuthorizedProject,
    query: SearchTermsQuery,
) -> Result<SearchTermsResponse, AuthError> {
    let conn = open_project_database_read(server_context.as_ref(), project.id())?;
    build_search_terms_response(&conn, &query)
}

/// Builds the search-terms response from an open project database connection.
///
/// Returns the requested window (`current`) with each term and the summary
/// carrying display-ready period-over-period deltas versus the prior 30 days
/// (additive quantities normalized to the current period length; rates and the
/// average position compared directly). The raw previous period is consumed
/// server-side and not returned.
pub(crate) fn build_search_terms_response(
    conn: &ProjectConnection,
    query: &SearchTermsQuery,
) -> Result<SearchTermsResponse, AuthError> {
    build_search_terms_response_at(conn, query, seconds_since_epoch() as i64)
}

/// Inner builder with an explicit `now` so the normalization factor is
/// deterministic in tests.
fn build_search_terms_response_at(
    conn: &ProjectConnection,
    query: &SearchTermsQuery,
    now: i64,
) -> Result<SearchTermsResponse, AuthError> {
    let previous_start = comparison::previous_period_start(query.start);
    let factor = comparison::normalization_factor(query.start, query.end, now);

    // One metrics fetch per window (current + previous). Landing paths attach
    // only for the page — or for all current terms when ranking by landing_paths.
    // Splitting summary/page/priors into separate queries that each rebuild
    // `search_terms_window_ctes` was measured ~2× slower on this endpoint.
    let mut current = fetch_search_terms_metrics(conn, query.start, query.end, &query.segments)?;
    let previous = fetch_search_terms_metrics(conn, previous_start, query.start, &query.segments)?;

    attach_ctr_opportunities(
        &mut current.terms,
        current.search_channel_revenue,
        current.summary.clicks,
    );
    attach_cannibalized_clicks(conn, &mut current.terms, query.start, query.end)?;
    mark_search_term_highlights(&mut current.terms);

    let sort_by_landing_paths = matches!(query.order_by, SearchTermsOrderBy::LandingPaths);
    if sort_by_landing_paths {
        attach_landing_paths(conn, &mut current.terms, query.start, query.end)?;
    }
    sort_raw_terms(&mut current.terms, query.order_by, query.direction);
    // Default table/treemap ranking is estimated revenue ≡ clicks. Promote
    // highlights there so they are visible without changing explicit sorts.
    if matches!(
        query.order_by,
        SearchTermsOrderBy::EstimatedRevenue | SearchTermsOrderBy::Clicks
    ) && matches!(query.direction, SortDirection::Desc)
    {
        promote_highlighted_terms(&mut current.terms);
    }
    let (mut page_terms, next_offset) =
        paginate(current.terms, query.offset, SEARCH_TERMS_PAGE_SIZE);
    if !sort_by_landing_paths {
        attach_landing_paths(conn, &mut page_terms, query.start, query.end)?;
    }

    let previous_by_query: FxHashMap<&str, &RawTerm> = previous
        .terms
        .iter()
        .map(|term| (term.query.as_str(), term))
        .collect();

    let terms = page_terms
        .iter()
        .map(|term| {
            let prior = previous_by_query.get(term.query.as_str()).copied();
            term_with_changes(term, prior, factor)
        })
        .collect();

    let summary = summary_with_changes(&current.summary, &previous.summary, factor);

    Ok(SearchTermsResponse {
        current: SearchTermsPeriod {
            search_channel_revenue: current.search_channel_revenue,
            summary,
            terms,
        },
        next_offset,
        offset: query.offset,
        limit: SEARCH_TERMS_PAGE_SIZE,
    })
}

/// Ranks terms for pagination. `estimated_revenue` uses the same comparator as
/// `clicks` (shared channel revenue / total clicks per period), including the
/// same secondary keys.
fn sort_raw_terms(terms: &mut [RawTerm], order_by: SearchTermsOrderBy, direction: SortDirection) {
    let descending = matches!(direction, SortDirection::Desc);
    // Estimated revenue is proportional to clicks within a period, so both
    // columns share the clicks primary key.
    let effective_order_by = match order_by {
        SearchTermsOrderBy::EstimatedRevenue => SearchTermsOrderBy::Clicks,
        other => other,
    };
    terms.sort_by(|left, right| {
        let primary = compare_raw_terms(left, right, effective_order_by);
        let ordered = if descending {
            primary.reverse()
        } else {
            primary
        };
        if ordered != Ordering::Equal {
            return ordered;
        }
        if matches!(effective_order_by, SearchTermsOrderBy::Clicks) {
            return left.query.cmp(&right.query);
        }
        right
            .clicks
            .cmp(&left.clicks)
            .then_with(|| left.query.cmp(&right.query))
    });
}

fn compare_raw_terms(left: &RawTerm, right: &RawTerm, order_by: SearchTermsOrderBy) -> Ordering {
    match order_by {
        SearchTermsOrderBy::Query => left.query.cmp(&right.query),
        SearchTermsOrderBy::LandingPaths => left
            .landing_paths
            .iter()
            .map(|entry| entry.path.as_str())
            .cmp(right.landing_paths.iter().map(|entry| entry.path.as_str())),
        SearchTermsOrderBy::Position => cmp_f64(left.position, right.position),
        SearchTermsOrderBy::Impressions => left.impressions.cmp(&right.impressions),
        SearchTermsOrderBy::Clicks | SearchTermsOrderBy::EstimatedRevenue => {
            left.clicks.cmp(&right.clicks)
        }
        SearchTermsOrderBy::Ctr => cmp_f64(
            click_through_rate(left.clicks, left.impressions),
            click_through_rate(right.clicks, right.impressions),
        ),
        SearchTermsOrderBy::Mobile => cmp_f64(
            mobile_share(left.mobile_clicks, left.detail_clicks),
            mobile_share(right.mobile_clicks, right.detail_clicks),
        ),
    }
}

fn parse_order_by(value: &str) -> Result<SearchTermsOrderBy, &'static str> {
    match value {
        "query" => Ok(SearchTermsOrderBy::Query),
        "landing_paths" => Ok(SearchTermsOrderBy::LandingPaths),
        "position" => Ok(SearchTermsOrderBy::Position),
        "impressions" => Ok(SearchTermsOrderBy::Impressions),
        "clicks" => Ok(SearchTermsOrderBy::Clicks),
        "ctr" => Ok(SearchTermsOrderBy::Ctr),
        "mobile" => Ok(SearchTermsOrderBy::Mobile),
        "estimated_revenue" => Ok(SearchTermsOrderBy::EstimatedRevenue),
        _ => Err("invalid order_by"),
    }
}
