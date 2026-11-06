use fxhash::{FxHashMap, FxHashSet};
use smallvec::SmallVec;
use std::cmp::Ordering;
use std::sync::Arc;

use rusqlite::types::ToSql;
use rusqlite::{params, params_from_iter};
use serde::Serialize;

use crate::auth::{AuthError, map_row_error};
use crate::authorize::AuthorizedProject;
use crate::db::{ProjectConnection, open_project_database_read};
use crate::ga_import::{ImportedDays, ImportedPage};
use crate::metrics::ratio;
use crate::metrics::{
    WilsonInterval, conversion_rate_wilson_interval, conversion_rate_wilson_lower_bound,
};
use crate::query_params::{SortDirection, SortedPage, cmp_f64, paginate, parse_report_query};
use crate::segment_filter::{SegmentDimension, SegmentFilters, parse_segment_filters};
use crate::server_context::ServerContext;
use crate::timezone;
use crate::util::NANOS_PER_SECOND;

/// Sessions that spent under this many seconds on the site count as bounces.
const BOUNCE_DURATION_SECONDS: i64 = 10;

/// Relative change applied to each lever for the marginal-revenue estimates
/// (e.g. "increase conversion by 25%" or "reduce exits by 25%").
const MARGINAL_CHANGE_FRACTION: f64 = 0.25;

/// Per-page engagement and attributed revenue over the requested window.
///
/// A "page" is a row in `paths`; metrics aggregate every pageview of that path
/// in the window. Session-scoped values (`converting_visitors`, `bounced`,
/// `exits`, session duration) are de-duplicated so a session that views the
/// same page multiple times is counted once. `avg_scroll_percent`,
/// `avg_page_duration`, and `attributed_revenue` use individual pageviews instead.
#[derive(Debug, Serialize)]
pub(crate) struct PageRow {
    /// Stable `paths.id` for linking to a table row.
    pub(crate) id: i64,
    /// Request path (always begins with `/`) from `paths.path`.
    pub(crate) path: String,
    /// Hostname the path was served on from `paths.hostname`.
    pub(crate) hostname: String,
    /// Distinct sessions that viewed this page in the window, imported ones included.
    pub(crate) visitors: i64,
    /// The visitors tracked by us. Revenue and exits only exist for those, so
    /// the rates built on them are shares of this and not of [`Self::visitors`].
    #[serde(skip)]
    pub(crate) tracked_visitors: i64,
    /// The visitors from imported data, which transitions can't show.
    pub(crate) imported_visitors: i64,
    /// Sessions viewing this page that also have attributed payment revenue.
    pub(crate) converting_visitors: i64,
    /// Revenue weighted by this page's share of the session duration before
    /// the payment. Amounts are still in thousandths. Follows the pageview
    /// cohort (not payment date).
    pub(crate) attributed_revenue: f64,
    /// Converting visitors divided by tracked visitors (0 when none).
    pub(crate) conversion_rate: f64,
    /// Lower bound of the 95% Wilson interval, used for ranking.
    pub(crate) conversion_lower_bound: f64,
    /// 95% Wilson confidence interval for the conversion rate (absent when no visitors).
    pub(crate) conversion_ci: Option<WilsonInterval>,
    /// Share of visitors whose session lasted under [`BOUNCE_DURATION_SECONDS`].
    pub(crate) bounce_rate: f64,
    /// [`Self::bounce_rate`] of tracked visitors alone, for weighing against revenue,
    /// which only they have.
    #[serde(skip)]
    pub(crate) tracked_bounce_rate: f64,
    /// The sessions [`Self::bounce_rate`] is a share of.
    #[serde(skip)]
    pub(crate) bounce_sessions: i64,
    /// Share of visitors whose session ended on this page (last pageview).
    pub(crate) exit_rate: f64,
    /// Mean `landings.session_duration_seconds` across this page's visitors.
    pub(crate) avg_session_duration: f64,
    /// Mean `pageviews.duration_seconds` across this page's pageviews that
    /// reported a dwell time.
    pub(crate) avg_page_duration: f64,
    /// Mean `pageviews.scrolled_percent` (0..100) across this page's pageviews
    /// that reported a scroll depth.
    pub(crate) avg_scroll_percent: f64,
    /// Attributed revenue divided by tracked visitors (0 when none). Thousandths.
    pub(crate) revenue_per_visitor: f64,
    /// Marginal attributed revenue from lifting visitors by
    /// [`MARGINAL_CHANGE_FRACTION`] at the current revenue per visitor.
    pub(crate) increase_visitors_25pct: f64,
    /// Marginal attributed revenue from lifting the conversion rate by
    /// [`MARGINAL_CHANGE_FRACTION`] (more converters at the same revenue each).
    pub(crate) increase_conversion_25pct: f64,
    /// Marginal attributed revenue from cutting the exit rate by
    /// [`MARGINAL_CHANGE_FRACTION`]: visitors kept in the funnel are valued at
    /// this page's revenue per visitor.
    pub(crate) reduce_exits_25pct: f64,
    /// Marginal attributed revenue from cutting the bounce rate by
    /// [`MARGINAL_CHANGE_FRACTION`]: recovered bounces are valued at this
    /// page's revenue per visitor.
    pub(crate) reduce_bounces_25pct: f64,
    /// Marginal attributed revenue from extending this page's dwell time by
    /// [`MARGINAL_CHANGE_FRACTION`], using the time-share attribution model.
    pub(crate) increase_session_25pct: f64,
    /// True when this page has the highest [`Self::increase_visitors_25pct`].
    pub(crate) best_visitors_uplift: bool,
    /// True when this page leads [`Self::increase_conversion_25pct`] among pages
    /// whose conversion rate is below the (visitor-weighted) site average.
    pub(crate) best_conversion_uplift: bool,
    /// True when this page leads [`Self::reduce_exits_25pct`] among pages whose
    /// exit rate is above the (visitor-weighted) site average.
    pub(crate) best_exit_uplift: bool,
    /// True when this page leads [`Self::reduce_bounces_25pct`] among pages
    /// whose bounce rate is above the (visitor-weighted) site average.
    pub(crate) best_bounce_uplift: bool,
    /// True when this page has the highest [`Self::increase_session_25pct`].
    pub(crate) best_session_uplift: bool,
    /// True when this page has the highest [`Self::exit_rate`] among pages
    /// whose exit rate is above the (visitor-weighted) site average.
    pub(crate) highest_exit_rate: bool,
    /// True when this page has the highest [`Self::bounce_rate`] among pages
    /// whose bounce rate is above the (visitor-weighted) site average.
    pub(crate) highest_bounce_rate: bool,
}

/// Pages ranked by the requested `order_by` for the requested window.
///
/// Page size is [`PAGES_PAGE_SIZE`]. Unique paths grow without bound, so the
/// table and API both page instead of returning every URL.
#[derive(Debug, Serialize)]
pub(crate) struct PagesResponse {
    pub(crate) pages: Vec<PageRow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) next_offset: Option<i64>,
    pub(crate) offset: i64,
    pub(crate) limit: i64,
}

/// Rows returned per page for `/pages` and `/landing-pages`.
pub(crate) const PAGES_PAGE_SIZE: i64 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PagesOrderBy {
    Url,
    Visitors,
    AttributedRevenue,
    Conversion,
    Bounce,
    Exit,
    SessionDuration,
    PageDuration,
    Scroll,
}

pub(crate) struct PagesQuery {
    pub(crate) start: i64,
    pub(crate) end: i64,
    pub(crate) offset: i64,
    pub(crate) order_by: PagesOrderBy,
    pub(crate) direction: SortDirection,
    pub(crate) segments: SegmentFilters,
}

pub(crate) fn parse_pages_query(target: &str) -> Result<PagesQuery, &'static str> {
    let mut sort = SortedPage::new(PagesOrderBy::AttributedRevenue, parse_pages_order_by);
    let range = parse_report_query(target, |key, value| sort.accept(key, value).map(drop))?;
    let (start, end) = range.validate()?;
    Ok(PagesQuery {
        start,
        end,
        offset: sort.offset,
        order_by: sort.order_by,
        direction: sort.direction,
        segments: parse_segment_filters(target),
    })
}

pub(crate) fn pages(
    server_context: Arc<ServerContext>,
    project: &AuthorizedProject,
    query: PagesQuery,
) -> Result<PagesResponse, AuthError> {
    fetch_pages(server_context, project, &query)
}

fn fetch_pages(
    server_context: Arc<ServerContext>,
    project: &AuthorizedProject,
    query: &PagesQuery,
) -> Result<PagesResponse, AuthError> {
    let timezone = timezone::project_timezone(server_context.clone(), project.key());
    let conn = open_project_database_read(server_context.as_ref(), project.id())
        .map_err(|error| AuthError::Db(error.to_string()))?;
    let imported =
        ImportedDays::for_report(&conn, &timezone, query.start, query.end, &query.segments)?;
    build_pages_response_with_imported(&conn, query, imported.as_ref())
}

/// Builds the pages response from an open project database connection.
///
/// Scans every pageview whose `timestamp_id` lies in `[start, end)`, joined to
/// its path and its session's landing (for session duration). Rows are ordered
/// by `(path_id, visitor_timestamp_id)` so session-scoped metrics can dedup with
/// a previous-value check. Exit pageviews are detected inline via an
/// anti-existence check. Qualifying payments are loaded for sessions that have
/// a pageview in the window (payment date is unconstrained), then applied to
/// pageviews that happened before each payment.
pub(crate) fn build_pages_response_with_imported(
    conn: &ProjectConnection,
    query: &PagesQuery,
    imported: Option<&ImportedDays>,
) -> Result<PagesResponse, AuthError> {
    let session_payments = load_session_payments(conn, query)?;
    // Pages already LEFT JOIN the visitor's landing row, so country filters
    // directly on `l.country`; a country filter turns the join into a match.
    let country_filter = query.segments.country_in_clause("l.country", 3);
    // The pages view lists pages, so it ignores the page segment filter (users
    // add pages from this very view); other dimensions still narrow it.
    let session_filter = query
        .segments
        .session_filter_clause("pv.visitor_timestamp_id", Some(SegmentDimension::Page));
    let sql = format!(
        "SELECT
            p.id AS path_id,
            p.path AS path,
            p.hostname AS hostname,
            pv.visitor_timestamp_id AS session_id,
            pv.timestamp_id AS pageview_id,
            pv.scrolled_percent AS scrolled_percent,
            pv.duration_seconds AS duration_seconds,
            COALESCE(l.session_duration_seconds, 0) AS session_duration_seconds,
            NOT EXISTS (
                SELECT 1
                FROM pageviews next
                WHERE next.visitor_timestamp_id = pv.visitor_timestamp_id
                  AND next.timestamp_id > pv.timestamp_id
            ) AS is_exit_pageview
         FROM pageviews pv
         JOIN paths p ON p.id = pv.path_id
         LEFT JOIN landings l ON l.visitor_timestamp_id = pv.visitor_timestamp_id
         WHERE pv.timestamp_id >= ?1
           AND pv.timestamp_id < ?2
           AND p.deleted_at IS NULL{country_filter}{session_filter}
         ORDER BY p.id, pv.visitor_timestamp_id"
    );

    let mut pages: FxHashMap<i64, PageAccumulator> = FxHashMap::default();

    let start_ns = query.start * NANOS_PER_SECOND;
    let end_ns = query.end * NANOS_PER_SECOND;
    let mut sql_params: Vec<&dyn ToSql> = vec![&start_ns, &end_ns];
    query.segments.append_country_params(&mut sql_params);

    conn.with_query(&sql, params_from_iter(sql_params), |mut rows| {
        while let Some(row) = rows.next()? {
            let path_id = row.get::<_, i64>(0)?;
            let session_id = row.get::<_, i64>(3)?;
            let pageview_id = row.get::<_, i64>(4)?;
            let scrolled_percent = row.get::<_, Option<i64>>(5)?;
            let duration_seconds = row.get::<_, Option<i64>>(6)?;
            let session_duration_seconds = row.get::<_, i64>(7)?;
            let is_exit_pageview = row.get::<_, i64>(8)? != 0;

            let entry = match pages.get_mut(&path_id) {
                Some(entry) => entry,
                None => {
                    let path = row.get::<_, String>(1)?;
                    let hostname = row.get::<_, String>(2)?;
                    pages
                        .entry(path_id)
                        .or_insert_with(|| PageAccumulator::new(path, hostname))
                }
            };
            entry.add_pageview(scrolled_percent, duration_seconds);
            entry.add_session(session_id, session_duration_seconds, is_exit_pageview);
            if let Some(payments) = session_payments.get(&session_id) {
                let viewed_at = pageview_id / NANOS_PER_SECOND;
                for payment in payments {
                    if payment.paid_at >= viewed_at {
                        entry.add_payment(
                            session_id,
                            payment.amount,
                            duration_seconds,
                            payment.session_duration_seconds,
                        );
                    }
                }
            }
        }
        Ok(())
    })
    .map_err(map_row_error)?;

    if let Some(days) = imported {
        for page in days.pages(conn)? {
            imported_entry(&mut pages, &page).add_imported_page(&page);
        }
    }
    Ok(page_rows(pages, query))
}

/// The page's accumulator, starting one for a page only imported data has seen.
pub(crate) fn imported_entry<'a>(
    pages: &'a mut FxHashMap<i64, PageAccumulator>,
    page: &ImportedPage,
) -> &'a mut PageAccumulator {
    pages
        .entry(page.path_id)
        .or_insert_with(|| PageAccumulator::new(page.path.clone(), page.hostname.clone()))
}

#[cfg(test)]
pub(crate) fn build_pages_response(
    conn: &ProjectConnection,
    query: &PagesQuery,
) -> Result<PagesResponse, AuthError> {
    build_pages_response_with_imported(conn, query, None)
}

pub(crate) struct PagePayment {
    pub(crate) amount: i64,
    pub(crate) paid_at: i64,
    pub(crate) session_duration_seconds: Option<i64>,
}

/// Excludes subscription renewal invoices; only the first successful charge counts.
const EXCLUDE_SUBSCRIPTION_RENEWALS: &str = "NOT (
    payment_type = 'subscription'
    AND EXISTS (
        SELECT 1
        FROM payments prior
        WHERE prior.subscription_id = payments.subscription_id
          AND prior.status = 'ok'
          AND prior.payment_type = 'subscription'
          AND prior.paid_at < payments.paid_at
          AND prior.id != payments.id
    )
)";

/// Loads qualifying payments for sessions with a pageview in `[start, end)`.
///
/// Renewals are excluded. Payment date is unconstrained so revenue follows the
/// visit/pageview cohort (Pages, Landing Pages, Events).
pub(crate) fn load_session_payments(
    conn: &ProjectConnection,
    query: &PagesQuery,
) -> Result<FxHashMap<i64, SmallVec<[PagePayment; 1]>>, AuthError> {
    let sql = format!(
        "SELECT
            attributed_visitor_timestamp_id,
            amount,
            paid_at,
            session_duration_seconds
         FROM payments
         WHERE attributed_visitor_timestamp_id IS NOT NULL
           AND status = 'ok'
           AND payment_type IN ('single', 'subscription')
           AND EXISTS (
                SELECT 1
                FROM pageviews pv
                WHERE pv.visitor_timestamp_id = payments.attributed_visitor_timestamp_id
                  AND pv.timestamp_id >= ?1
                  AND pv.timestamp_id < ?2
           )
           AND {EXCLUDE_SUBSCRIPTION_RENEWALS}"
    );

    let mut payments: FxHashMap<i64, SmallVec<[PagePayment; 1]>> = FxHashMap::default();
    let start_ns = query.start * NANOS_PER_SECOND;
    let end_ns = query.end * NANOS_PER_SECOND;
    conn.with_query(&sql, params![start_ns, end_ns], |mut rows| {
        while let Some(row) = rows.next()? {
            let session_id = row.get::<_, i64>(0)?;
            payments.entry(session_id).or_default().push(PagePayment {
                amount: row.get::<_, i64>(1)?,
                paid_at: row.get::<_, i64>(2)?,
                session_duration_seconds: row.get::<_, Option<i64>>(3)?,
            });
        }
        Ok(())
    })
    .map_err(map_row_error)?;

    Ok(payments)
}

/// Accumulates per-page metrics during the scan.
///
/// Callers must feed pageviews ordered by `(path_id, session_id)` so
/// [`Self::add_session`] can treat a change in `session_id` as a new distinct
/// visitor. Converting sessions stay in a set because payment application is
/// not ordered that way and cardinalities are small.
#[derive(Default)]
pub(crate) struct PageAccumulator {
    path: String,
    hostname: String,
    /// Last session id seen for this path; used for O(1) distinct visitors.
    last_session_id: Option<i64>,
    seen_converting_sessions: FxHashSet<i64>,
    visitors: i64,
    converting_visitors: i64,
    attributed_revenue: f64,
    bounced: i64,
    exits: i64,
    session_duration_total: i64,
    page_duration_total: i64,
    page_duration_count: i64,
    scroll_sum: i64,
    scroll_count: i64,
    /// Extra attributed revenue this page would earn if its dwell time grew by
    /// [`MARGINAL_CHANGE_FRACTION`], summed across converting payments using the
    /// same time-share weighting as [`Self::attributed_revenue`]. Thousandths.
    session_duration_uplift: f64,
    imported_visitors: i64,
    imported_pageviews: i64,
    imported_page_seconds: i64,
    /// Sessions imported for this page as a landing page, the only ones GA can
    /// say bounced or lasted how long.
    imported_landings: i64,
    imported_bounced: i64,
    imported_session_seconds: i64,
}

impl PageAccumulator {
    pub(crate) fn new(path: String, hostname: String) -> Self {
        Self {
            path,
            hostname,
            ..Self::default()
        }
    }

    fn add_imported_page(&mut self, page: &ImportedPage) {
        self.imported_visitors += page.sessions;
        self.imported_pageviews += page.pageviews;
        self.imported_page_seconds += page.engagement_seconds;
    }

    pub(crate) fn add_imported_landing(&mut self, page: &ImportedPage) {
        self.imported_visitors += page.sessions;
        self.imported_landings += page.sessions;
        self.imported_bounced += page.sessions.saturating_sub(page.engaged_sessions).max(0);
        self.imported_session_seconds += page.engagement_seconds;
    }

    pub(crate) fn add_pageview(
        &mut self,
        scrolled_percent: Option<i64>,
        duration_seconds: Option<i64>,
    ) {
        if let Some(scroll) = scrolled_percent {
            self.scroll_sum += scroll;
            self.scroll_count += 1;
        }
        if let Some(duration) = duration_seconds {
            self.page_duration_total += duration;
            self.page_duration_count += 1;
        }
    }

    pub(crate) fn add_session(
        &mut self,
        session_id: i64,
        session_duration_seconds: i64,
        is_exit_pageview: bool,
    ) {
        // A session has exactly one exit pageview (last by timestamp_id PK), so
        // this accumulator sees is_exit_pageview at most once per session.
        if is_exit_pageview {
            self.exits += 1;
        }
        if self.last_session_id == Some(session_id) {
            return;
        }
        self.last_session_id = Some(session_id);
        self.visitors += 1;
        if session_duration_seconds < BOUNCE_DURATION_SECONDS {
            self.bounced += 1;
        }
        self.session_duration_total += session_duration_seconds;
    }

    fn add_payment(
        &mut self,
        session_id: i64,
        amount: i64,
        duration_seconds: Option<i64>,
        session_duration_seconds: Option<i64>,
    ) {
        if amount > 0 && self.seen_converting_sessions.insert(session_id) {
            self.converting_visitors += 1;
        }
        let Some(duration_seconds) = duration_seconds else {
            return;
        };
        let Some(session_duration_seconds) = session_duration_seconds else {
            return;
        };
        if duration_seconds <= 0 || session_duration_seconds <= 0 {
            return;
        }
        let amount = amount as f64;
        let dwell = duration_seconds as f64;
        let session = session_duration_seconds as f64;
        let current_share = amount * dwell / session;
        self.attributed_revenue += current_share;
        self.session_duration_uplift += session_duration_uplift_amount(amount, dwell, session);
    }

    pub(crate) fn add_landing_payment(
        &mut self,
        session_id: i64,
        amount: i64,
        duration_seconds: Option<i64>,
        session_duration_seconds: Option<i64>,
    ) {
        if amount > 0 && self.seen_converting_sessions.insert(session_id) {
            self.converting_visitors += 1;
        }
        self.attributed_revenue += amount as f64;
        let Some(duration_seconds) = duration_seconds else {
            return;
        };
        let Some(session_duration_seconds) = session_duration_seconds else {
            return;
        };
        if duration_seconds <= 0 || session_duration_seconds <= 0 {
            return;
        }
        self.session_duration_uplift += session_duration_uplift_amount(
            amount as f64,
            duration_seconds as f64,
            session_duration_seconds as f64,
        );
    }
}

/// Marginal attributed revenue from extending a page's dwell by
/// [`MARGINAL_CHANGE_FRACTION`] under the time-share attribution model.
fn session_duration_uplift_amount(amount: f64, dwell: f64, session: f64) -> f64 {
    let current_share = amount * dwell / session;
    // If this page's dwell grew by the marginal fraction (others held
    // constant), its share of the converting session — and thus its
    // attributed revenue — rises sub-linearly. The gain is largest for
    // pages that currently own a small slice of the session.
    let extra_dwell = dwell * MARGINAL_CHANGE_FRACTION;
    let boosted_share = amount * (dwell + extra_dwell) / (session + extra_dwell);
    boosted_share - current_share
}

/// Converts accumulated pages into ranked, paginated rows.
///
/// Opportunity stars (`best_*_uplift`) are computed across every page before
/// the requested `order_by` slice, so paging does not change which URLs lead.
/// Default rank is attributed revenue (visitors break ties, then hostname+path).
pub(crate) fn page_rows(
    pages: FxHashMap<i64, PageAccumulator>,
    query: &PagesQuery,
) -> PagesResponse {
    let mut out: Vec<PageRow> = pages
        .into_iter()
        .map(|(id, page)| {
            let tracked_visitors = page.visitors;
            let visitors = tracked_visitors + page.imported_visitors;
            let converting = page.converting_visitors;
            let revenue = page.attributed_revenue;
            let exit_rate = ratio(page.exits, tracked_visitors);
            let bounce_rate = ratio(
                page.bounced + page.imported_bounced,
                tracked_visitors + page.imported_landings,
            );
            let tracked_bounce_rate = ratio(page.bounced, tracked_visitors);
            PageRow {
                id,
                path: page.path,
                hostname: page.hostname,
                visitors,
                tracked_visitors,
                imported_visitors: page.imported_visitors,
                converting_visitors: converting,
                attributed_revenue: revenue,
                conversion_rate: ratio(converting, tracked_visitors),
                conversion_lower_bound: conversion_rate_wilson_lower_bound(
                    converting as f64,
                    tracked_visitors as f64,
                ),
                conversion_ci: conversion_rate_wilson_interval(
                    converting as f64,
                    tracked_visitors as f64,
                ),
                bounce_rate,
                tracked_bounce_rate,
                bounce_sessions: tracked_visitors + page.imported_landings,
                exit_rate,
                avg_session_duration: average(
                    page.session_duration_total + page.imported_session_seconds,
                    tracked_visitors + page.imported_landings,
                ),
                avg_page_duration: average(
                    page.page_duration_total + page.imported_page_seconds,
                    page.page_duration_count + page.imported_pageviews,
                ),
                avg_scroll_percent: average(page.scroll_sum, page.scroll_count),
                revenue_per_visitor: if tracked_visitors > 0 {
                    revenue / tracked_visitors as f64
                } else {
                    0.0
                },
                // More visitors at the same revenue per visitor scale revenue
                // linearly, so the gain is the marginal fraction of revenue.
                increase_visitors_25pct: MARGINAL_CHANGE_FRACTION * revenue,
                // A relative lift in the conversion rate adds the same fraction
                // of converters, each worth today's revenue per converter.
                increase_conversion_25pct: MARGINAL_CHANGE_FRACTION * revenue,
                // Visitors recovered from exiting are valued at this page's
                // revenue per visitor: 25% * exit_rate * visitors * (rev/visitor).
                reduce_exits_25pct: MARGINAL_CHANGE_FRACTION * exit_rate * revenue,
                reduce_bounces_25pct: MARGINAL_CHANGE_FRACTION * tracked_bounce_rate * revenue,
                increase_session_25pct: page.session_duration_uplift,
                best_visitors_uplift: false,
                best_conversion_uplift: false,
                best_exit_uplift: false,
                best_bounce_uplift: false,
                best_session_uplift: false,
                highest_exit_rate: false,
                highest_bounce_rate: false,
            }
        })
        .collect();
    mark_best_opportunities(&mut out);
    paginate_page_rows(out, query)
}

fn paginate_page_rows(mut out: Vec<PageRow>, query: &PagesQuery) -> PagesResponse {
    out.sort_by(|a, b| cmp_page_rows(a, b, query.order_by, query.direction));
    let (page, next_offset) = paginate(out, query.offset, PAGES_PAGE_SIZE);
    PagesResponse {
        pages: page,
        next_offset,
        offset: query.offset,
        limit: PAGES_PAGE_SIZE,
    }
}

fn cmp_page_rows(
    a: &PageRow,
    b: &PageRow,
    order_by: PagesOrderBy,
    direction: SortDirection,
) -> Ordering {
    let primary = match order_by {
        PagesOrderBy::Url => cmp_page_url(a, b),
        PagesOrderBy::Visitors => a.visitors.cmp(&b.visitors),
        PagesOrderBy::AttributedRevenue => cmp_f64(a.attributed_revenue, b.attributed_revenue),
        PagesOrderBy::Conversion => cmp_f64(a.conversion_lower_bound, b.conversion_lower_bound),
        PagesOrderBy::Bounce => cmp_f64(a.bounce_rate, b.bounce_rate),
        PagesOrderBy::Exit => cmp_f64(a.exit_rate, b.exit_rate),
        PagesOrderBy::SessionDuration => cmp_f64(a.avg_session_duration, b.avg_session_duration),
        PagesOrderBy::PageDuration => cmp_f64(a.avg_page_duration, b.avg_page_duration),
        PagesOrderBy::Scroll => cmp_f64(a.avg_scroll_percent, b.avg_scroll_percent),
    };
    let primary = direction.apply(primary);
    if primary != Ordering::Equal {
        return primary;
    }
    if matches!(order_by, PagesOrderBy::AttributedRevenue) {
        let visitors = b.visitors.cmp(&a.visitors);
        if visitors != Ordering::Equal {
            return visitors;
        }
    } else if !matches!(order_by, PagesOrderBy::Url) {
        let revenue = cmp_f64(b.attributed_revenue, a.attributed_revenue);
        if revenue != Ordering::Equal {
            return revenue;
        }
    }
    cmp_page_url(a, b)
}

fn cmp_page_url(a: &PageRow, b: &PageRow) -> Ordering {
    let left = format!("{}{}", a.hostname, a.path).to_ascii_lowercase();
    let right = format!("{}{}", b.hostname, b.path).to_ascii_lowercase();
    left.cmp(&right)
}

/// Flags best-opportunity and worst-rate pages for dashboard highlights.
///
/// Conversion candidates are restricted to pages below the visitor-weighted
/// average conversion rate, and exit/bounce candidates to pages above the
/// average rate, so the marks point at genuine underperformers. Visitor and
/// session uplift consider every page. Ties (equal best or worst value) are
/// all flagged, and a category with no positive opportunity or no above-average
/// rate is left unflagged.
fn mark_best_opportunities(rows: &mut [PageRow]) {
    let total_visitors: f64 = rows.iter().map(|row| row.tracked_visitors as f64).sum();
    if total_visitors <= 0.0 {
        return;
    }
    let total_converting: f64 = rows.iter().map(|row| row.converting_visitors as f64).sum();
    let total_exits: f64 = rows
        .iter()
        .map(|row| row.exit_rate * row.tracked_visitors as f64)
        .sum();
    let total_bounces: f64 = rows
        .iter()
        .map(|row| row.tracked_bounce_rate * row.tracked_visitors as f64)
        .sum();
    let avg_conversion_rate = total_converting / total_visitors;
    let avg_exit_rate = total_exits / total_visitors;
    let avg_bounce_rate = total_bounces / total_visitors;

    flag_best(
        rows,
        |_| true,
        |row| row.increase_visitors_25pct,
        |row| row.best_visitors_uplift = true,
    );
    flag_best(
        rows,
        |_| true,
        |row| row.increase_session_25pct,
        |row| row.best_session_uplift = true,
    );
    flag_best(
        rows,
        |row| row.tracked_visitors > 0 && row.conversion_rate < avg_conversion_rate,
        |row| row.increase_conversion_25pct,
        |row| row.best_conversion_uplift = true,
    );
    flag_best(
        rows,
        |row| row.tracked_visitors > 0 && row.exit_rate > avg_exit_rate,
        |row| row.reduce_exits_25pct,
        |row| row.best_exit_uplift = true,
    );
    flag_best(
        rows,
        |row| row.tracked_visitors > 0 && row.tracked_bounce_rate > avg_bounce_rate,
        |row| row.reduce_bounces_25pct,
        |row| row.best_bounce_uplift = true,
    );
    flag_best(
        rows,
        |row| row.tracked_visitors > 0 && row.exit_rate > avg_exit_rate,
        |row| row.exit_rate,
        |row| row.highest_exit_rate = true,
    );
    // Unlike the uplift above, this one marks the rate on show, imported bounces and all.
    let shown_bounces: f64 = rows
        .iter()
        .map(|row| row.bounce_rate * row.bounce_sessions as f64)
        .sum();
    let shown_sessions: f64 = rows.iter().map(|row| row.bounce_sessions as f64).sum();
    let avg_shown_bounce_rate = shown_bounces / shown_sessions;
    flag_best(
        rows,
        |row| row.bounce_sessions > 0 && row.bounce_rate > avg_shown_bounce_rate,
        |row| row.bounce_rate,
        |row| row.highest_bounce_rate = true,
    );
}

/// Sets a flag on every eligible row whose `value` equals the maximum positive
/// value among eligible rows. No row is flagged when the best value is ≤ 0.
fn flag_best(
    rows: &mut [PageRow],
    eligible: impl Fn(&PageRow) -> bool,
    value: impl Fn(&PageRow) -> f64,
    mut set_flag: impl FnMut(&mut PageRow),
) {
    let best = rows
        .iter()
        .filter(|row| eligible(row))
        .map(&value)
        .filter(|candidate| *candidate > 0.0)
        .fold(f64::NEG_INFINITY, f64::max);
    if !best.is_finite() {
        return;
    }
    for row in rows.iter_mut() {
        if eligible(row) && value(row) == best {
            set_flag(row);
        }
    }
}

fn average(total: i64, count: i64) -> f64 {
    if count > 0 {
        total as f64 / count as f64
    } else {
        0.0
    }
}

fn parse_pages_order_by(value: &str) -> Result<PagesOrderBy, &'static str> {
    match value {
        "url" => Ok(PagesOrderBy::Url),
        "visitors" => Ok(PagesOrderBy::Visitors),
        "attributed_revenue" => Ok(PagesOrderBy::AttributedRevenue),
        "conversion" => Ok(PagesOrderBy::Conversion),
        "bounce" => Ok(PagesOrderBy::Bounce),
        "exit" => Ok(PagesOrderBy::Exit),
        "session_duration" => Ok(PagesOrderBy::SessionDuration),
        "page_duration" => Ok(PagesOrderBy::PageDuration),
        "scroll" => Ok(PagesOrderBy::Scroll),
        _ => Err("invalid order_by"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::open_migrated_db;

    use crate::db::ProjectConnection;

    use crate::test_utils::{PaymentFixture, insert_payment as insert_test_payment};

    fn timestamp_id(seed: u8, viewed_at: i64) -> i64 {
        viewed_at * NANOS_PER_SECOND + i64::from(seed)
    }

    fn insert_path(conn: &ProjectConnection, path: &str, hostname: &str) -> i64 {
        conn.execute(
            "INSERT INTO paths (path, hostname) VALUES (?1, ?2)",
            params![path, hostname],
        )
        .expect("insert path");
        conn.query_row(
            "SELECT id FROM paths WHERE path = ?1 AND hostname = ?2",
            params![path, hostname],
            |row| row.get::<_, i64>(0),
        )
        .expect("path id")
    }

    /// Inserts a landing pageview (session start) plus its `landings` row.
    fn insert_landing(
        conn: &ProjectConnection,
        seed: u8,
        viewed_at: i64,
        path_id: i64,
        session_duration_seconds: i64,
        session_page_count: i64,
        scrolled_percent: Option<i64>,
    ) -> i64 {
        insert_landing_with_duration(
            conn,
            seed,
            viewed_at,
            path_id,
            session_duration_seconds,
            session_page_count,
            scrolled_percent,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_landing_with_duration(
        conn: &ProjectConnection,
        seed: u8,
        viewed_at: i64,
        path_id: i64,
        session_duration_seconds: i64,
        session_page_count: i64,
        scrolled_percent: Option<i64>,
        duration_seconds: Option<i64>,
    ) -> i64 {
        let session_id = timestamp_id(seed, viewed_at);
        conn.execute(
            "INSERT INTO pageviews (
                timestamp_id, visitor_timestamp_id, previous_timestamp_id, path_id,
                duration_seconds, scrolled_percent
             ) VALUES (?1, ?1, NULL, ?2, ?3, ?4)",
            params![session_id, path_id, duration_seconds, scrolled_percent],
        )
        .expect("insert landing pageview");
        conn.execute(
            "INSERT INTO landings (
                visitor_timestamp_id, session_duration_seconds, session_page_count,
                is_google, tracker_version, device
             ) VALUES (?1, ?2, ?3, 0, 1.0, 'desktop')",
            params![session_id, session_duration_seconds, session_page_count],
        )
        .expect("insert landing");
        session_id
    }

    /// Inserts a subsequent pageview in an existing session.
    fn insert_pageview(
        conn: &ProjectConnection,
        seed: u8,
        viewed_at: i64,
        path_id: i64,
        session_id: i64,
        scrolled_percent: Option<i64>,
    ) {
        insert_pageview_with_duration(
            conn,
            seed,
            viewed_at,
            path_id,
            session_id,
            scrolled_percent,
            None,
        );
    }

    fn insert_pageview_with_duration(
        conn: &ProjectConnection,
        seed: u8,
        viewed_at: i64,
        path_id: i64,
        session_id: i64,
        scrolled_percent: Option<i64>,
        duration_seconds: Option<i64>,
    ) {
        let view_id = timestamp_id(seed, viewed_at);
        conn.execute(
            "INSERT INTO pageviews (
                timestamp_id, visitor_timestamp_id, previous_timestamp_id, path_id,
                duration_seconds, scrolled_percent
             ) VALUES (?1, ?2, NULL, ?3, ?4, ?5)",
            params![
                view_id,
                session_id,
                path_id,
                duration_seconds,
                scrolled_percent
            ],
        )
        .expect("insert pageview");
    }

    fn insert_payment(
        conn: &ProjectConnection,
        provider_payment_id: &str,
        session_id: i64,
        amount: i64,
        paid_at: i64,
        session_duration_seconds: Option<i64>,
    ) {
        let mut payment = PaymentFixture::new(provider_payment_id, amount, paid_at);
        payment.visitor_timestamp_id = Some(session_id);
        payment.session_duration_seconds = session_duration_seconds;
        insert_test_payment(conn, payment);
    }

    fn baseline_query(start: i64, end: i64) -> PagesQuery {
        PagesQuery {
            start,
            end,
            offset: 0,
            order_by: PagesOrderBy::AttributedRevenue,
            direction: SortDirection::Desc,
            segments: SegmentFilters::default(),
        }
    }

    #[test]
    fn imported_sessions_add_visitors_but_not_to_rates_they_cannot_answer() {
        let conn = open_migrated_db();
        conn.execute_batch(
            "INSERT INTO paths (id, path, hostname) VALUES (1, '/a', 'acme.test');
             INSERT INTO imports VALUES (1, 'google_analytics', 'Acme', '1970-01-01', '1970-01-01');
             INSERT INTO imported_pages VALUES (1, '1970-01-01', 1, 90, 180, 900);",
        )
        .expect("seed");
        let session = NANOS_PER_SECOND * 100;
        conn.execute_batch(&format!(
            "INSERT INTO pageviews (timestamp_id, visitor_timestamp_id, path_id, duration_seconds)
                 VALUES ({session}, {session}, 1, 30);
             INSERT INTO landings (visitor_timestamp_id, session_duration_seconds, is_google, tracker_version, device)
                 VALUES ({session}, 5, 0, 1.0, 'desktop');"
        ))
        .expect("tracked session");
        let query = PagesQuery {
            start: 0,
            end: 86_400,
            offset: 0,
            order_by: PagesOrderBy::Visitors,
            direction: SortDirection::Desc,
            segments: SegmentFilters::default(),
        };
        let days = ImportedDays::covering(&crate::timezone::ProjectTimezone::utc(), 0, 86_400);
        let response =
            build_pages_response_with_imported(&conn, &query, days.as_ref()).expect("response");
        let page = &response.pages[0];
        assert_eq!(page.visitors, 91);
        assert_eq!(page.tracked_visitors, 1);
        // The one tracked session was short, and GA can't say about the other 90.
        assert_eq!(page.bounce_rate, 1.0);
        assert_eq!(page.exit_rate, 1.0);
        // (30 tracked + 900 imported seconds) over (1 + 180) pageviews.
        assert!((page.avg_page_duration - 930.0 / 181.0).abs() < 1e-9);
    }

    #[test]
    fn rejects_inverted_range() {
        assert!(parse_pages_query("/v2/read/projects/1/pages?start=200&end=100",).is_err());
    }

    #[test]
    fn rejects_range_longer_than_max_days() {
        let max = crate::comparison::MAX_RANGE_SECONDS;
        let ok = format!("/v2/read/projects/1/pages?start=0&end={max}");
        parse_pages_query(&ok).expect("exact max range is allowed");
        let too_long = format!("/v2/read/projects/1/pages?start=0&end={}", max + 1);
        assert!(matches!(
            parse_pages_query(&too_long),
            Err("date range must be at most 366 days")
        ));
    }

    #[test]
    fn parses_offset_and_ordering() {
        let query = parse_pages_query(
            "/v2/read/projects/1/pages?start=1&end=100&offset=200&order_by=visitors&direction=asc",
        )
        .expect("query");
        assert_eq!(query.offset, 200);
        assert_eq!(query.order_by, PagesOrderBy::Visitors);
        assert_eq!(query.direction, SortDirection::Asc);
    }

    #[test]
    fn rejects_invalid_offset_and_ordering() {
        assert!(parse_pages_query("/v2/read/projects/1/pages?start=1&end=100&offset=-1",).is_err());
        assert!(
            parse_pages_query("/v2/read/projects/1/pages?start=1&end=100&order_by=bad",).is_err()
        );
        assert!(
            parse_pages_query("/v2/read/projects/1/pages?start=1&end=100&direction=sideways",)
                .is_err()
        );
    }

    #[test]
    fn empty_database_returns_no_pages() {
        let conn = open_migrated_db();
        let response = build_pages_response(&conn, &baseline_query(0, 10_000)).expect("response");
        assert!(response.pages.is_empty());
        assert_eq!(response.offset, 0);
        assert_eq!(response.limit, PAGES_PAGE_SIZE);
        assert_eq!(response.next_offset, None);
    }

    #[test]
    fn ranks_pages_by_attributed_revenue_and_computes_metrics() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/", "acme.test");
        let pricing = insert_path(&conn, "/pricing", "acme.test");

        // Session 1 lands on home (120s, scroll 80), converts $30k.
        let s1 = insert_landing(&conn, 1, 100, home, 120, 2, Some(80));
        // Session 1 also views pricing (scroll 50).
        insert_pageview(&conn, 2, 110, pricing, s1, Some(50));
        // Session 2 lands on pricing (5s = bounce, scroll 10), converts $50k.
        let s2 = insert_landing(&conn, 3, 120, pricing, 5, 1, Some(10));
        // Session 3 lands on home (60s, no scroll reported), no revenue.
        let _s3 = insert_landing(&conn, 4, 130, home, 60, 1, None);

        insert_payment(&conn, "pay_1", s1, 30_000, 200, None);
        insert_payment(&conn, "pay_2", s2, 50_000, 210, None);

        let response = build_pages_response(&conn, &baseline_query(0, 10_000)).expect("response");
        assert_eq!(response.pages.len(), 2);

        // Tied on attributed revenue (no per-pageview durations); home ranks first by path.
        let home_row = &response.pages[0];
        assert_eq!(home_row.path, "/");
        assert_eq!(home_row.visitors, 2);
        assert_eq!(home_row.converting_visitors, 1);
        assert_eq!(home_row.attributed_revenue, 0.0);
        assert!((home_row.conversion_rate - 0.5).abs() < 1e-9);
        // Neither home session bounced (120s, 60s).
        assert_eq!(home_row.bounce_rate, 0.0);
        // s3 ended on home; s1 continued to pricing. No attributed revenue, so
        // the negative highlight is the highest rate rather than exit uplift.
        assert!((home_row.exit_rate - 0.5).abs() < 1e-9);
        assert!(!home_row.highest_exit_rate);
        assert!(!home_row.best_exit_uplift);
        assert!((home_row.avg_session_duration - 90.0).abs() < 1e-9);
        // Only s1's home pageview reported scroll (80); s3 reported none.
        assert!((home_row.avg_scroll_percent - 80.0).abs() < 1e-9);

        let pricing_row = &response.pages[1];
        assert_eq!(pricing_row.path, "/pricing");
        assert_eq!(pricing_row.hostname, "acme.test");
        assert_eq!(pricing_row.visitors, 2);
        assert_eq!(pricing_row.converting_visitors, 2);
        assert_eq!(pricing_row.attributed_revenue, 0.0);
        assert_eq!(pricing_row.conversion_rate, 1.0);
        // s2 lasted 5s (< 10s) so one of two visitors bounced.
        assert!((pricing_row.bounce_rate - 0.5).abs() < 1e-9);
        // Both sessions ended on pricing; highest rate, but no revenue to lift.
        assert_eq!(pricing_row.exit_rate, 1.0);
        assert!(pricing_row.highest_exit_rate);
        assert!(!pricing_row.best_exit_uplift);
        // avg session duration = (120 + 5) / 2.
        assert!((pricing_row.avg_session_duration - 62.5).abs() < 1e-9);
        // scroll averages the two pricing pageviews: (50 + 10) / 2.
        assert!((pricing_row.avg_scroll_percent - 30.0).abs() < 1e-9);
    }

    #[test]
    fn repeated_pageviews_count_session_once() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/", "acme.test");
        // Session views home as landing and again later.
        let s1 = insert_landing(&conn, 1, 100, home, 45, 2, Some(20));
        insert_pageview(&conn, 2, 150, home, s1, Some(60));
        insert_payment(&conn, "pay_1", s1, 12_000, 200, None);

        let response = build_pages_response(&conn, &baseline_query(0, 10_000)).expect("response");
        assert_eq!(response.pages.len(), 1);
        let home_row = &response.pages[0];
        // Counted as one visitor despite two pageviews.
        assert_eq!(home_row.visitors, 1);
        assert_eq!(home_row.converting_visitors, 1);
        // Scroll still averages both pageviews: (20 + 60) / 2.
        assert!((home_row.avg_scroll_percent - 40.0).abs() < 1e-9);
    }

    #[test]
    fn attributes_payments_outside_query_window_when_pageview_is_inside() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/", "acme.test");
        let session = insert_landing(&conn, 1, 100, home, 120, 1, Some(80));
        // Payment date is after the pageview window; revenue still follows the
        // in-window pageview cohort (same as Sources / Audience).
        insert_payment(&conn, "pay_1", session, 30_000, 500, None);

        let response = build_pages_response(&conn, &baseline_query(0, 400)).expect("response");
        assert_eq!(response.pages.len(), 1);
        assert_eq!(response.pages[0].converting_visitors, 1);
    }

    #[test]
    fn attributed_revenue_counts_pageviews_before_payment_and_weights_by_time() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/", "acme.test");
        let pricing = insert_path(&conn, "/pricing", "acme.test");
        let thank_you = insert_path(&conn, "/thank-you", "acme.test");

        let session = insert_landing_with_duration(&conn, 1, 100, home, 100, 3, Some(80), Some(40));
        insert_pageview_with_duration(&conn, 2, 120, pricing, session, Some(50), Some(30));
        insert_pageview_with_duration(&conn, 3, 140, thank_you, session, Some(10), Some(30));

        // Session duration up to payment: 40s on home + 30s on pricing = 70s.
        insert_payment(&conn, "pay_1", session, 100_000, 130, Some(70));

        let response = build_pages_response(&conn, &baseline_query(0, 10_000)).expect("response");
        assert_eq!(response.pages.len(), 3);

        let home_row = response
            .pages
            .iter()
            .find(|page| page.path == "/")
            .expect("home row");
        assert!((home_row.attributed_revenue - 40_000.0 * 100.0 / 70.0).abs() < 1e-9);
        assert_eq!(home_row.converting_visitors, 1);
        assert!((home_row.avg_page_duration - 40.0).abs() < 1e-9);

        let pricing_row = response
            .pages
            .iter()
            .find(|page| page.path == "/pricing")
            .expect("pricing row");
        assert!((pricing_row.attributed_revenue - 30_000.0 * 100.0 / 70.0).abs() < 1e-9);
        assert_eq!(pricing_row.converting_visitors, 1);
        assert!((pricing_row.avg_page_duration - 30.0).abs() < 1e-9);

        let thank_you_row = response
            .pages
            .iter()
            .find(|page| page.path == "/thank-you")
            .expect("thank-you row");
        assert_eq!(thank_you_row.visitors, 1);
        assert_eq!(thank_you_row.attributed_revenue, 0.0);
        assert_eq!(thank_you_row.converting_visitors, 0);
    }

    #[test]
    fn exit_rate_counts_sessions_whose_last_pageview_was_this_page() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/", "acme.test");
        let pricing = insert_path(&conn, "/pricing", "acme.test");
        let docs = insert_path(&conn, "/docs", "acme.test");

        // Session 1: home -> pricing -> docs (ends on docs).
        let s1 = insert_landing(&conn, 1, 100, home, 120, 3, Some(80));
        insert_pageview(&conn, 2, 110, pricing, s1, Some(50));
        insert_pageview(&conn, 3, 120, docs, s1, Some(90));
        // Session 2: home only (ends on home).
        let _s2 = insert_landing(&conn, 4, 130, home, 60, 1, Some(40));
        // Session 3: pricing only (ends on pricing).
        let _s3 = insert_landing(&conn, 5, 140, pricing, 45, 1, Some(10));

        let response = build_pages_response(&conn, &baseline_query(0, 10_000)).expect("response");
        let home_row = response
            .pages
            .iter()
            .find(|page| page.path == "/")
            .expect("home row");
        assert_eq!(home_row.visitors, 2);
        assert!((home_row.exit_rate - 0.5).abs() < 1e-9);

        let pricing_row = response
            .pages
            .iter()
            .find(|page| page.path == "/pricing")
            .expect("pricing row");
        assert_eq!(pricing_row.visitors, 2);
        assert!((pricing_row.exit_rate - 0.5).abs() < 1e-9);

        let docs_row = response
            .pages
            .iter()
            .find(|page| page.path == "/docs")
            .expect("docs row");
        assert_eq!(docs_row.visitors, 1);
        assert_eq!(docs_row.exit_rate, 1.0);
    }

    #[test]
    fn computes_marginal_revenue_and_opportunity_highlights() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/", "acme.test");
        let pricing = insert_path(&conn, "/pricing", "acme.test");

        // Session 1: home (dwell 40) -> pricing (dwell 30), converts $100.
        let s1 = insert_landing_with_duration(&conn, 1, 100, home, 100, 2, Some(80), Some(40));
        insert_pageview_with_duration(&conn, 2, 120, pricing, s1, Some(50), Some(30));
        // Session 2: pricing only, exits on pricing, no conversion.
        let _s2 = insert_landing_with_duration(&conn, 3, 130, pricing, 20, 1, Some(10), Some(20));

        // Session duration up to payment: 40s home + 30s pricing = 70s.
        insert_payment(&conn, "pay_1", s1, 100_000, 140, Some(70));

        let response = build_pages_response(&conn, &baseline_query(0, 10_000)).expect("response");
        let home_row = response
            .pages
            .iter()
            .find(|page| page.path == "/")
            .expect("home row");
        let pricing_row = response
            .pages
            .iter()
            .find(|page| page.path == "/pricing")
            .expect("pricing row");

        let approx = |a: f64, b: f64| assert!((a - b).abs() < 1e-6, "expected {a} ≈ {b}");

        // Home: revenue 100_000 * 40/70, single visitor.
        let home_revenue = 100_000.0 * 40.0 / 70.0;
        approx(home_row.attributed_revenue, home_revenue);
        approx(home_row.revenue_per_visitor, home_revenue);
        approx(home_row.increase_visitors_25pct, 0.25 * home_revenue);
        approx(home_row.increase_conversion_25pct, 0.25 * home_revenue);
        // Home never ends a session, so cutting its exits yields nothing.
        approx(home_row.reduce_exits_25pct, 0.0);
        approx(
            home_row.increase_session_25pct,
            100_000.0 * 50.0 / 80.0 - home_revenue,
        );

        // Pricing: revenue 100_000 * 30/70 across two visitors.
        let pricing_revenue = 100_000.0 * 30.0 / 70.0;
        approx(pricing_row.attributed_revenue, pricing_revenue);
        approx(pricing_row.revenue_per_visitor, pricing_revenue / 2.0);
        approx(pricing_row.reduce_exits_25pct, 0.25 * 1.0 * pricing_revenue);

        // Highlights: home leads visitor uplift (higher revenue); pricing is the
        // below-average converter and above-average exiter, and wins session
        // uplift (it owns the smaller share of the converting session).
        assert!(home_row.best_visitors_uplift);
        assert!(!home_row.best_conversion_uplift);
        assert!(!home_row.best_exit_uplift);
        assert!(!home_row.best_session_uplift);
        assert!(!home_row.highest_exit_rate);
        assert!(!pricing_row.best_visitors_uplift);
        assert!(pricing_row.best_conversion_uplift);
        assert!(pricing_row.best_exit_uplift);
        assert!(pricing_row.best_session_uplift);
        assert!(pricing_row.highest_exit_rate);
    }

    #[test]
    fn filters_pageviews_outside_window() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/", "acme.test");
        // Before window.
        insert_landing(&conn, 1, 50, home, 30, 1, Some(40));
        // Inside window [100, 200).
        insert_landing(&conn, 2, 150, home, 30, 1, Some(40));
        // After window.
        insert_landing(&conn, 3, 250, home, 30, 1, Some(40));

        let response = build_pages_response(&conn, &baseline_query(100, 200)).expect("response");
        assert_eq!(response.pages.len(), 1);
        assert_eq!(response.pages[0].visitors, 1);
    }

    #[test]
    fn excludes_deleted_paths() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/", "acme.test");
        insert_landing(&conn, 1, 100, home, 30, 1, Some(40));
        conn.execute(
            "UPDATE paths SET deleted_at = 1 WHERE id = ?1",
            params![home],
        )
        .expect("soft delete path");

        let response = build_pages_response(&conn, &baseline_query(0, 10_000)).expect("response");
        assert!(response.pages.is_empty());
    }

    #[test]
    fn landing_page_segment_filter_narrows_pages_to_sessions_with_that_landing() {
        let conn = open_migrated_db();
        let a = insert_path(&conn, "/a", "acme.test");
        let b = insert_path(&conn, "/b", "acme.test");
        let common = insert_path(&conn, "/common", "acme.test");
        // Session A lands on /a then views /common.
        let sa = insert_landing(&conn, 1, 100, a, 60, 2, Some(80));
        insert_pageview(&conn, 2, 110, common, sa, Some(50));
        // Session B lands on /b then views /common.
        let sb = insert_landing(&conn, 3, 200, b, 60, 2, Some(80));
        insert_pageview(&conn, 4, 210, common, sb, Some(50));

        let mut query = baseline_query(0, 10_000);
        query.segments = SegmentFilters {
            countries: Vec::new(),
            sources: Vec::new(),
            pages: Vec::new(),
            landing_pages: vec![a],
            events: Vec::new(),
            ..Default::default()
        };
        let response = build_pages_response(&conn, &query).expect("response");
        let paths: Vec<&str> = response.pages.iter().map(|p| p.path.as_str()).collect();
        // Only session A survives: its pages (/a and /common) show, /b does not.
        assert!(paths.contains(&"/a"));
        assert!(paths.contains(&"/common"));
        assert!(!paths.contains(&"/b"));
    }

    #[test]
    fn source_segment_filter_narrows_pages_to_matching_sessions() {
        let conn = open_migrated_db();
        let home = insert_path(&conn, "/", "acme.test");
        let pricing = insert_path(&conn, "/pricing", "acme.test");
        // A direct session on "/" and a campaign session on "/pricing".
        insert_landing(&conn, 1, 100, home, 60, 1, Some(80));
        let campaign_session = insert_landing(&conn, 2, 200, pricing, 60, 1, Some(80));
        conn.execute(
            "INSERT INTO campaigns (source, medium, campaign, first_seen_at)
             VALUES ('ads', 'cpc', 'launch', 0)",
            [],
        )
        .expect("insert campaign");
        let campaign_id: i64 = conn
            .query_row("SELECT id FROM campaigns LIMIT 1", [], |row| row.get(0))
            .expect("campaign id");
        conn.execute(
            "UPDATE landings SET campaign = ?1 WHERE visitor_timestamp_id = ?2",
            params![campaign_id, campaign_session],
        )
        .expect("attach campaign");

        // Filtering to the direct source drops the campaign session's page.
        let mut query = baseline_query(0, 10_000);
        query.segments = SegmentFilters {
            countries: Vec::new(),
            sources: vec![crate::segment_filter::SourceFilter::Direct],
            pages: Vec::new(),
            landing_pages: Vec::new(),
            events: Vec::new(),
            ..Default::default()
        };
        let response = build_pages_response(&conn, &query).expect("response");
        assert_eq!(response.pages.len(), 1);
        assert_eq!(response.pages[0].path, "/");

        // Filtering to the campaign source keeps only its page.
        query.segments = SegmentFilters {
            countries: Vec::new(),
            sources: vec![crate::segment_filter::SourceFilter::Campaign(campaign_id)],
            pages: Vec::new(),
            landing_pages: Vec::new(),
            events: Vec::new(),
            ..Default::default()
        };
        let response = build_pages_response(&conn, &query).expect("response");
        assert_eq!(response.pages.len(), 1);
        assert_eq!(response.pages[0].path, "/pricing");
    }

    #[test]
    fn paginates_pages_with_limit_and_offset() {
        let conn = open_migrated_db();
        let total = (PAGES_PAGE_SIZE as usize) + 1;
        for index in 0..total {
            let path_id = insert_path(&conn, &format!("/page-{index:03}"), "acme.test");
            let session = insert_landing_with_duration(
                &conn,
                index as u8,
                100 + index as i64,
                path_id,
                60,
                1,
                None,
                Some(60),
            );
            // Distinct revenue so default (attributed revenue desc) order is
            // deterministic: highest remaining index first.
            insert_payment(
                &conn,
                &format!("pay_{index}"),
                session,
                (total - index) as i64 * 1_000,
                5_000,
                Some(60),
            );
        }

        let mut query = baseline_query(0, 10_000);
        let first_page = build_pages_response(&conn, &query).expect("first page");
        assert_eq!(first_page.pages.len(), PAGES_PAGE_SIZE as usize);
        assert_eq!(first_page.offset, 0);
        assert_eq!(first_page.limit, PAGES_PAGE_SIZE);
        assert_eq!(first_page.next_offset, Some(PAGES_PAGE_SIZE));
        assert_eq!(first_page.pages[0].path, "/page-000");
        assert_eq!(
            first_page.pages.last().expect("last").path,
            format!("/page-{:03}", PAGES_PAGE_SIZE - 1)
        );

        query.offset = PAGES_PAGE_SIZE;
        let second_page = build_pages_response(&conn, &query).expect("second page");
        assert_eq!(second_page.pages.len(), 1);
        assert_eq!(second_page.next_offset, None);
        assert_eq!(
            second_page.pages[0].path,
            format!("/page-{:03}", PAGES_PAGE_SIZE)
        );
    }

    #[test]
    fn sorts_pages_by_visitors() {
        let conn = open_migrated_db();
        let alpha = insert_path(&conn, "/alpha", "acme.test");
        let bravo = insert_path(&conn, "/bravo", "acme.test");
        let charlie = insert_path(&conn, "/charlie", "acme.test");
        insert_landing(&conn, 1, 100, alpha, 60, 1, None);
        insert_landing(&conn, 2, 110, bravo, 60, 1, None);
        insert_landing(&conn, 3, 120, bravo, 60, 1, None);
        insert_landing(&conn, 4, 130, charlie, 60, 1, None);
        insert_landing(&conn, 5, 140, charlie, 60, 1, None);
        insert_landing(&conn, 6, 150, charlie, 60, 1, None);

        let mut query = baseline_query(0, 10_000);
        query.order_by = PagesOrderBy::Visitors;
        query.direction = SortDirection::Desc;
        let response = build_pages_response(&conn, &query).expect("response");
        let paths: Vec<_> = response.pages.iter().map(|row| row.path.as_str()).collect();
        assert_eq!(paths, vec!["/charlie", "/bravo", "/alpha"]);
    }
}
