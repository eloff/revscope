//! Reading Search Console metrics and attributed revenue for a window.

use fxhash::FxHashMap;
use rusqlite::params;
use rusqlite::params_from_iter;
use rusqlite::types::ToSql;

use crate::auth::{AuthError, map_row_error};
use crate::db::ProjectConnection;
use crate::page_revenue::page_revenue_cte;
use crate::segment_filter::SegmentFilters;
use crate::timezone;
use crate::util::NANOS_PER_SECOND;

use super::rows::{
    RawPeriod, RawSummary, RawTerm, SearchTermLandingPath, click_through_rate, mobile_share,
};

/// Shared CTEs for a search-terms window: date-bounded `page_revenue`,
/// search-channel revenue, per-query GSC totals, and window click total.
///
/// Bind params: `?1`/`?2` UTC-day bounds for the Search Console rows,
/// `?3`/`?4` landing nanos for the exact instant window. Country placeholders
/// begin at `?5` when a country segment is active.
pub(super) fn search_terms_window_ctes(segments: &SegmentFilters) -> String {
    // Country placeholders begin at ?5. GSC daily rows have no country
    // dimension, so impressions/clicks/position/mobile share cannot be
    // country-scoped; only attributed search-channel revenue is filtered.
    let revenue_country_filter = segments.country_in_clause("l.country", 5);
    // Session-scoped filters (e.g. source) narrow the landings-based revenue
    // channel only; the GSC click/impression rows carry no session, so they
    // cannot be session-filtered.
    let revenue_session_filter = segments.session_filter_clause("l.visitor_timestamp_id", None);
    // Same visit window as the landings filter (?3/?4); first subscription charge only.
    let page_revenue = page_revenue_cte(3, 4, true);
    format!(
        "{page_revenue},
         search_channel AS (
             SELECT
                 COALESCE(SUM(rev.amount), 0) AS revenue
             FROM landings l
             LEFT JOIN page_revenue rev ON rev.visitor_timestamp_id = l.visitor_timestamp_id
             WHERE l.visitor_timestamp_id >= ?3
               AND l.visitor_timestamp_id < ?4
               AND l.campaign IS NULL
               AND l.is_google = 1{revenue_country_filter}{revenue_session_filter}
         ),
         term_totals AS (
             SELECT
                 sq.id AS query_id,
                 sq.query AS query,
                 SUM(d.clicks) AS clicks,
                 SUM(d.impressions) AS impressions,
                 SUM(d.mobile_clicks) AS mobile_clicks,
                 SUM(d.mobile_clicks + d.desktop_clicks) AS detail_clicks,
                 CASE
                     WHEN SUM(d.impressions) > 0
                     THEN CAST(SUM(d.position * d.impressions) AS REAL)
                          / SUM(d.impressions)
                     ELSE 0
                 END AS position
             FROM search_queries sq
             JOIN search_queries_by_date d ON d.query_id = sq.id
             WHERE d.date >= ?1
               AND d.date < ?2
             GROUP BY sq.id, sq.query
         ),
         click_totals AS (
             SELECT COALESCE(SUM(clicks), 0) AS total_clicks FROM term_totals
         )"
    )
}

pub(super) fn window_bind_params<'a>(
    start: &'a i64,
    end: &'a i64,
    start_ns: &'a i64,
    end_ns: &'a i64,
    segments: &'a SegmentFilters,
) -> Vec<&'a dyn ToSql> {
    let mut sql_params: Vec<&dyn ToSql> = vec![start, end, start_ns, end_ns];
    segments.append_country_params(&mut sql_params);
    sql_params
}

/// Aggregates GSC metrics and search-channel revenue for every distinct query
/// in `[start, end)`. Landing paths are not included; call
/// [`attach_landing_paths`] for the terms that need them (the page, or all
/// current terms when sorting by landing paths).
///
/// Revenue is attributed from sessions, so it uses `[start, end)` exactly. The
/// Search Console metrics cannot: they are stored one row per UTC date, so the
/// window is widened to the UTC days it overlaps (see
/// [`timezone::utc_days_covering`]).
pub(super) fn fetch_search_terms_metrics(
    conn: &ProjectConnection,
    start: i64,
    end: i64,
    segments: &SegmentFilters,
) -> Result<RawPeriod, AuthError> {
    let ctes = search_terms_window_ctes(segments);
    let sql = format!(
        "WITH {ctes}
         SELECT
             t.query_id,
             t.query,
             t.position,
             t.impressions,
             t.clicks,
             t.mobile_clicks,
             t.detail_clicks,
             CAST(ROUND(
                 CASE WHEN ct.total_clicks > 0
                      THEN CAST(sc.revenue AS REAL) * t.clicks / ct.total_clicks
                      ELSE 0 END
             ) AS INTEGER) AS estimated_revenue,
             sc.revenue AS channel_revenue
         FROM term_totals t
         CROSS JOIN search_channel sc
         CROSS JOIN click_totals ct"
    );

    let (date_start, date_end) = timezone::utc_days_covering(start, end);
    let start_ns = start * NANOS_PER_SECOND;
    let end_ns = end * NANOS_PER_SECOND;
    let sql_params = window_bind_params(&date_start, &date_end, &start_ns, &end_ns, segments);
    let (terms, search_channel_revenue, mut summary) = conn
        .with_query(&sql, params_from_iter(sql_params), |mut rows| {
            let mut out = Vec::with_capacity(100);
            let mut search_channel_revenue = 0;
            let mut weighted_position = 0.0;
            let mut weighted_ctr = 0.0;
            let mut weighted_mobile_share = 0.0;
            let mut total_impressions = 0;
            let mut total_clicks = 0;
            while let Some(row) = rows.next()? {
                let query_id = row.get::<_, i64>(0)?;
                let query = row.get::<_, String>(1)?;
                let position = row.get::<_, f64>(2)?;
                let impressions = row.get::<_, i64>(3)?;
                let clicks = row.get::<_, i64>(4)?;
                let mobile_clicks = row.get::<_, i64>(5)?;
                let detail_clicks = row.get::<_, i64>(6)?;
                let estimated_revenue = row.get::<_, i64>(7)?;
                search_channel_revenue = row.get::<_, i64>(8)?;
                let click_weight = clicks as f64;
                weighted_position += position * click_weight;
                weighted_ctr += click_through_rate(clicks, impressions) * click_weight;
                weighted_mobile_share += mobile_share(mobile_clicks, detail_clicks) * click_weight;
                total_impressions += impressions;
                total_clicks += clicks;
                out.push(RawTerm {
                    query_id,
                    query,
                    landing_paths: Vec::new(),
                    position,
                    impressions,
                    clicks,
                    mobile_clicks,
                    detail_clicks,
                    estimated_revenue,
                    expected_ctr: 0.0,
                    lost_clicks: 0.0,
                    lost_revenue: 0.0,
                    cannibalized_clicks: 0,
                    highest_lost_clicks: false,
                    highest_cannibalization: false,
                });
            }
            let summary = RawSummary {
                position: if total_clicks > 0 {
                    weighted_position / total_clicks as f64
                } else {
                    0.0
                },
                impressions: total_impressions,
                clicks: total_clicks,
                ctr: if total_clicks > 0 {
                    weighted_ctr / total_clicks as f64
                } else {
                    0.0
                },
                mobile_share: if total_clicks > 0 {
                    weighted_mobile_share / total_clicks as f64
                } else {
                    0.0
                },
                estimated_revenue: 0,
            };
            Ok((out, search_channel_revenue, summary))
        })
        .map_err(map_row_error)?;

    summary.estimated_revenue = search_channel_revenue;

    Ok(RawPeriod {
        search_channel_revenue,
        summary,
        terms,
    })
}

/// Loads landing paths for the given terms and attaches them in place. Paths
/// are ordered by clicks descending, then path ascending, using only Search
/// Console rows in `[start, end)`.
pub(super) fn attach_landing_paths(
    conn: &ProjectConnection,
    terms: &mut [RawTerm],
    start: i64,
    end: i64,
) -> Result<(), AuthError> {
    if terms.is_empty() {
        return Ok(());
    }
    let mut paths_by_query_id =
        fetch_landing_paths(conn, terms.iter().map(|term| term.query_id), start, end)?;
    for term in terms.iter_mut() {
        if let Some(paths) = paths_by_query_id.remove(&term.query_id) {
            term.landing_paths = paths;
        }
    }
    Ok(())
}

/// Returns every landing path for the provided query ids, keyed by `query_id`.
/// Paths are ordered by clicks descending, then path ascending. Clicks are
/// summed across the daily GSC rows covering `[start, end)`, using the same
/// UTC-day widening as [`fetch_search_terms_metrics`].
pub(super) fn fetch_landing_paths(
    conn: &ProjectConnection,
    query_ids: impl IntoIterator<Item = i64>,
    start: i64,
    end: i64,
) -> Result<FxHashMap<i64, Vec<SearchTermLandingPath>>, AuthError> {
    let query_ids: Vec<i64> = query_ids.into_iter().collect();
    if query_ids.is_empty() {
        return Ok(FxHashMap::default());
    }
    let placeholders = (1..=query_ids.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let start_param = query_ids.len() + 1;
    let end_param = query_ids.len() + 2;
    let sql = format!(
        "SELECT
             sp.query_id,
             sp.path_id,
             p.path,
             SUM(sp.clicks) AS clicks
         FROM search_query_paths_by_date sp
         JOIN paths p ON p.id = sp.path_id
         WHERE sp.query_id IN ({placeholders})
           AND sp.date >= ?{start_param}
           AND sp.date < ?{end_param}
         GROUP BY sp.query_id, sp.path_id, p.path
         ORDER BY sp.query_id, clicks DESC, p.path ASC"
    );
    let (date_start, date_end) = timezone::utc_days_covering(start, end);
    let mut params: Vec<&dyn ToSql> = query_ids.iter().map(|id| id as &dyn ToSql).collect();
    params.push(&date_start);
    params.push(&date_end);
    conn.with_query(&sql, params_from_iter(params), |mut rows| {
        let mut out: FxHashMap<i64, Vec<SearchTermLandingPath>> = FxHashMap::default();
        while let Some(row) = rows.next()? {
            let query_id = row.get::<_, i64>(0)?;
            let path_id = row.get::<_, i64>(1)?;
            let path = row.get::<_, String>(2)?;
            let clicks = row.get::<_, i64>(3)?;
            out.entry(query_id)
                .or_default()
                .push(SearchTermLandingPath {
                    path_id,
                    path,
                    clicks,
                });
        }
        Ok(out)
    })
    .map_err(map_row_error)
}

/// Clicks that did not land on the leading path for each query in `[start, end)`.
pub(super) fn fetch_cannibalized_clicks(
    conn: &ProjectConnection,
    start: i64,
    end: i64,
) -> Result<FxHashMap<i64, i64>, AuthError> {
    let sql = "SELECT query_id, SUM(path_clicks) - MAX(path_clicks) AS cannibalized
         FROM (
             SELECT query_id, path_id, SUM(clicks) AS path_clicks
             FROM search_query_paths_by_date
             WHERE date >= ?1 AND date < ?2
             GROUP BY query_id, path_id
         )
         GROUP BY query_id
         HAVING cannibalized > 0";
    let (date_start, date_end) = timezone::utc_days_covering(start, end);
    conn.with_query(sql, params![date_start, date_end], |mut rows| {
        let mut out = FxHashMap::default();
        while let Some(row) = rows.next()? {
            let query_id = row.get::<_, i64>(0)?;
            let cannibalized = row.get::<_, i64>(1)?;
            out.insert(query_id, cannibalized);
        }
        Ok(out)
    })
    .map_err(map_row_error)
}

pub(super) fn attach_cannibalized_clicks(
    conn: &ProjectConnection,
    terms: &mut [RawTerm],
    start: i64,
    end: i64,
) -> Result<(), AuthError> {
    if terms.is_empty() {
        return Ok(());
    }
    let scores = fetch_cannibalized_clicks(conn, start, end)?;
    for term in terms.iter_mut() {
        if let Some(clicks) = scores.get(&term.query_id) {
            term.cannibalized_clicks = *clicks;
        }
    }
    Ok(())
}
