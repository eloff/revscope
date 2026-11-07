use std::cmp::Ordering;
use std::sync::Arc;

use fxhash::{FxHashMap, FxHashSet};
use rusqlite::params;
use serde::Serialize;

use crate::auth::{AuthError, map_row_error};
use crate::authorize::AuthorizedProject;
use crate::db::{ProjectConnection, open_project_database_read};
use crate::query_params::{SortDirection, SortedPage, paginate, parse_report_query};
use crate::server_context::ServerContext;

/// Sanity cap so a pathological user-agent explosion cannot dump unbounded rows.
const CRAWLERS_SCAN_LIMIT: i64 = 5000;

/// Rows returned per page for `/crawlers`.
pub(crate) const CRAWLERS_PAGE_SIZE: i64 = 100;

const CRAWLER_CATEGORIES: &[&str] = &[
    "ai_crawler",
    "ai_search",
    "ai_assistant",
    "ai_browser",
    "search_engine",
    "archive",
    "headless",
];

#[derive(Debug, Serialize, PartialEq, Eq)]
pub(crate) struct CrawlerRow {
    pub(crate) useragent_id: i64,
    pub(crate) source: i64,
    pub(crate) verified: bool,
    pub(crate) useragent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) operator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) category: Option<String>,
    pub(crate) views: i64,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub(crate) struct CrawlerOperatorRow {
    pub(crate) source: i64,
    pub(crate) verified: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) operator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) category: Option<String>,
    pub(crate) views: i64,
}

#[derive(Debug, Default, Serialize, PartialEq, Eq)]
pub(crate) struct CrawlerCategoryTotals {
    pub(crate) ai_crawler: i64,
    pub(crate) ai_search: i64,
    pub(crate) ai_assistant: i64,
    pub(crate) ai_browser: i64,
    pub(crate) search_engine: i64,
    pub(crate) archive: i64,
    pub(crate) headless: i64,
}

/// Crawlers ranked by the requested `order_by` for the requested window.
///
/// Page size is [`CRAWLERS_PAGE_SIZE`]. Distinct user-agents can grow without
/// bound, so the table and API both page instead of returning every row.
/// `category_totals` and `operators` cover the full scan (not just the page)
/// so the summary cards and treemap stay complete.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub(crate) struct CrawlersResponse {
    pub(crate) rows: Vec<CrawlerRow>,
    pub(crate) category_totals: CrawlerCategoryTotals,
    pub(crate) operators: Vec<CrawlerOperatorRow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) next_offset: Option<i64>,
    pub(crate) offset: i64,
    pub(crate) limit: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CrawlersOrderBy {
    Views,
    Operator,
    Category,
    Useragent,
}

#[derive(Debug)]
pub(crate) struct CrawlersQuery {
    pub(crate) start: i64,
    pub(crate) end: i64,
    pub(crate) offset: i64,
    pub(crate) order_by: CrawlersOrderBy,
    pub(crate) direction: SortDirection,
    pub(crate) disabled_categories: FxHashSet<String>,
}

pub(crate) fn parse_crawlers_query(target: &str) -> Result<CrawlersQuery, &'static str> {
    let mut sort = SortedPage::new(CrawlersOrderBy::Views, parse_crawlers_order_by);
    let mut disabled_categories = FxHashSet::default();
    let range = parse_report_query(target, |key, value| {
        if !sort.accept(key, value)? && key == "disabled_category" {
            disabled_categories.insert(parse_crawler_category(value)?.to_string());
        }
        Ok(())
    })?;
    let (start, end) = range.validate()?;
    Ok(CrawlersQuery {
        start,
        end,
        offset: sort.offset,
        order_by: sort.order_by,
        direction: sort.direction,
        disabled_categories,
    })
}

pub(crate) fn crawlers(
    server_context: Arc<ServerContext>,
    project: &AuthorizedProject,
    query: CrawlersQuery,
) -> Result<CrawlersResponse, AuthError> {
    let conn = open_project_database_read(server_context.as_ref(), project.id())
        .map_err(|error| AuthError::Db(error.to_string()))?;
    build_crawlers_response(&conn, &query)
}

pub(crate) fn build_crawlers_response(
    conn: &ProjectConnection,
    query: &CrawlersQuery,
) -> Result<CrawlersResponse, AuthError> {
    let mut scanned = Vec::new();
    conn.with_query(
        "SELECT cv.useragent_id, cv.source, cv.verified, ua.useragent, ua.operator,
                ua.category, SUM(cv.estimated_count) AS views
         FROM crawler_views_by_hour cv
         JOIN useragents ua ON ua.id = cv.useragent_id
         WHERE cv.hour >= ?1 AND cv.hour < ?2
         GROUP BY cv.useragent_id, cv.source, cv.verified
         ORDER BY views DESC
         LIMIT ?3",
        params![query.start, query.end, CRAWLERS_SCAN_LIMIT],
        |mut cursor| {
            while let Some(row) = cursor.next()? {
                let verified: i64 = row.get(2)?;
                scanned.push(CrawlerRow {
                    useragent_id: row.get(0)?,
                    source: row.get(1)?,
                    verified: verified != 0,
                    useragent: row.get(3)?,
                    operator: row.get(4)?,
                    category: row.get(5)?,
                    views: row.get(6)?,
                });
            }
            Ok(())
        },
    )
    .map_err(map_row_error)?;

    let mut category_totals = CrawlerCategoryTotals::default();
    // Keyed by (source, verified, operator, category).
    type OperatorKey = (i64, bool, Option<String>, Option<String>);
    let mut operator_views: FxHashMap<OperatorKey, i64> = FxHashMap::default();
    for row in &scanned {
        add_category_total(&mut category_totals, category_for_row(row), row.views);
        *operator_views
            .entry((
                row.source,
                row.verified,
                row.operator.clone(),
                row.category.clone(),
            ))
            .or_insert(0) += row.views;
    }
    let operators = operator_views
        .into_iter()
        .map(
            |((source, verified, operator, category), views)| CrawlerOperatorRow {
                source,
                verified,
                operator,
                category,
                views,
            },
        )
        .collect();

    let mut rows: Vec<CrawlerRow> = scanned
        .into_iter()
        .filter(|row| !query.disabled_categories.contains(category_for_row(row)))
        .collect();
    rows.sort_by(|left, right| cmp_crawler_rows(left, right, query.order_by, query.direction));

    let (page, next_offset) = paginate(rows, query.offset, CRAWLERS_PAGE_SIZE);

    Ok(CrawlersResponse {
        rows: page,
        category_totals,
        operators,
        next_offset,
        offset: query.offset,
        limit: CRAWLERS_PAGE_SIZE,
    })
}

fn category_for_row(row: &CrawlerRow) -> &'static str {
    match row.category.as_deref() {
        Some("ai_crawler") => "ai_crawler",
        Some("ai_search") => "ai_search",
        Some("ai_assistant") => "ai_assistant",
        Some("ai_browser") => "ai_browser",
        Some("search_engine") => "search_engine",
        Some("archive") => "archive",
        Some("headless") => "headless",
        _ => "headless",
    }
}

fn add_category_total(totals: &mut CrawlerCategoryTotals, category: &str, views: i64) {
    match category {
        "ai_crawler" => totals.ai_crawler += views,
        "ai_search" => totals.ai_search += views,
        "ai_assistant" => totals.ai_assistant += views,
        "ai_browser" => totals.ai_browser += views,
        "search_engine" => totals.search_engine += views,
        "archive" => totals.archive += views,
        _ => totals.headless += views,
    }
}

fn operator_label_for_row(row: &CrawlerRow) -> String {
    if !row.verified {
        return "Unknown".to_string();
    }
    match row.operator.as_deref() {
        Some("openai") => "OpenAI".to_string(),
        Some("duckduckgo") => "DuckDuckGo".to_string(),
        Some("commoncrawl") => "Common Crawl".to_string(),
        Some(slug) if !slug.is_empty() => capitalize_operator_label(slug),
        _ => "Unknown".to_string(),
    }
}

fn capitalize_operator_label(label: &str) -> String {
    let mut chars = label.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => {
            let mut capitalized = first.to_uppercase().collect::<String>();
            capitalized.push_str(chars.as_str());
            capitalized
        }
    }
}

fn category_label(category: &str) -> &'static str {
    match category {
        "ai_crawler" => "AI crawler",
        "ai_search" => "AI search",
        "ai_assistant" => "AI assistant",
        "ai_browser" => "AI browser",
        "search_engine" => "Search engine",
        "archive" => "Archive",
        _ => "Headless browser",
    }
}

fn crawler_row_key(row: &CrawlerRow) -> String {
    format!(
        "{}:{}:{}",
        row.useragent_id,
        row.source,
        i64::from(row.verified)
    )
}

fn cmp_crawler_rows(
    left: &CrawlerRow,
    right: &CrawlerRow,
    order_by: CrawlersOrderBy,
    direction: SortDirection,
) -> Ordering {
    let primary = match order_by {
        CrawlersOrderBy::Views => left.views.cmp(&right.views),
        CrawlersOrderBy::Operator => {
            operator_label_for_row(left).cmp(&operator_label_for_row(right))
        }
        CrawlersOrderBy::Category => {
            category_label(category_for_row(left)).cmp(category_label(category_for_row(right)))
        }
        CrawlersOrderBy::Useragent => left.useragent.cmp(&right.useragent),
    };
    let ordered = direction.apply(primary);
    if ordered != Ordering::Equal {
        return ordered;
    }
    right
        .views
        .cmp(&left.views)
        .then_with(|| crawler_row_key(left).cmp(&crawler_row_key(right)))
}

fn parse_crawlers_order_by(value: &str) -> Result<CrawlersOrderBy, &'static str> {
    match value {
        "views" => Ok(CrawlersOrderBy::Views),
        "operator" => Ok(CrawlersOrderBy::Operator),
        "category" => Ok(CrawlersOrderBy::Category),
        "useragent" => Ok(CrawlersOrderBy::Useragent),
        _ => Err("invalid order_by"),
    }
}

fn parse_crawler_category(value: &str) -> Result<&'static str, &'static str> {
    CRAWLER_CATEGORIES
        .iter()
        .copied()
        .find(|category| *category == value)
        .ok_or("invalid disabled_category")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::open_migrated_db;

    use rusqlite::params;

    use crate::db::ProjectConnection;

    fn insert_useragent(
        conn: &ProjectConnection,
        useragent: &str,
        operator: Option<&str>,
        category: Option<&str>,
    ) -> i64 {
        conn.execute(
            "INSERT INTO useragents (useragent, operator, category) VALUES (?1, ?2, ?3)",
            params![useragent, operator, category],
        )
        .expect("insert useragent");
        conn.query_row(
            "SELECT id FROM useragents WHERE useragent = ?1",
            params![useragent],
            |row| row.get::<_, i64>(0),
        )
        .expect("useragent id")
    }

    fn insert_crawler_path(conn: &ProjectConnection) -> i64 {
        conn.execute(
            "INSERT INTO crawler_paths (path, hostname) VALUES ('/', 'acme.test')",
            [],
        )
        .expect("insert crawler path");
        conn.query_row("SELECT id FROM crawler_paths", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("crawler path id")
    }

    fn insert_view(
        conn: &ProjectConnection,
        hour: i64,
        source: i64,
        useragent_id: i64,
        path_id: i64,
        estimated_count: i64,
        verified: bool,
    ) {
        conn.execute(
            "INSERT INTO crawler_views_by_hour (
                hour, source, useragent_id, path_id, estimated_count, verified
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                hour,
                source,
                useragent_id,
                path_id,
                estimated_count,
                i64::from(verified)
            ],
        )
        .expect("insert crawler view");
    }

    fn query(start: i64, end: i64) -> CrawlersQuery {
        CrawlersQuery {
            start,
            end,
            offset: 0,
            order_by: CrawlersOrderBy::Views,
            direction: SortDirection::Desc,
            disabled_categories: FxHashSet::default(),
        }
    }

    #[test]
    fn parses_crawlers_query() {
        let parsed =
            parse_crawlers_query("/v2/read/projects/1/crawlers?start=10&end=100").expect("query");
        assert_eq!(parsed.start, 10);
        assert_eq!(parsed.end, 100);
        assert_eq!(parsed.offset, 0);
        assert_eq!(parsed.order_by, CrawlersOrderBy::Views);
        assert_eq!(parsed.direction, SortDirection::Desc);
        assert!(parsed.disabled_categories.is_empty());
    }

    #[test]
    fn parses_offset_ordering_and_disabled_categories() {
        let parsed = parse_crawlers_query(
            "/v2/read/projects/1/crawlers?start=1&end=100&offset=100&order_by=useragent&direction=asc&disabled_category=headless&disabled_category=archive",
        )
        .expect("query");
        assert_eq!(parsed.offset, 100);
        assert_eq!(parsed.order_by, CrawlersOrderBy::Useragent);
        assert_eq!(parsed.direction, SortDirection::Asc);
        assert_eq!(parsed.disabled_categories.len(), 2);
        assert!(parsed.disabled_categories.contains("headless"));
        assert!(parsed.disabled_categories.contains("archive"));
    }

    #[test]
    fn rejects_crawlers_query_missing_range() {
        assert!(parse_crawlers_query("/v2/read/projects/1/crawlers?start=10").is_err());
    }

    #[test]
    fn rejects_invalid_offset_and_ordering() {
        assert_eq!(
            parse_crawlers_query("/v2/read/projects/1/crawlers?start=1&end=100&offset=-1")
                .unwrap_err(),
            "offset must be at least 0"
        );
        assert_eq!(
            parse_crawlers_query("/v2/read/projects/1/crawlers?start=1&end=100&order_by=bad")
                .unwrap_err(),
            "invalid order_by"
        );
        assert_eq!(
            parse_crawlers_query(
                "/v2/read/projects/1/crawlers?start=1&end=100&disabled_category=robots"
            )
            .unwrap_err(),
            "invalid disabled_category"
        );
    }

    #[test]
    fn groups_views_by_useragent_and_source() {
        let conn = open_migrated_db();
        let gptbot = insert_useragent(&conn, "gptbot/1.0", Some("openai"), Some("ai_crawler"));
        let unknown = insert_useragent(&conn, "mystery-bot", None, None);
        let path = insert_crawler_path(&conn);

        insert_view(&conn, 3_600, 1, gptbot, path, 10, true);
        insert_view(&conn, 7_200, 1, gptbot, path, 5, true);
        insert_view(&conn, 3_600, 0, gptbot, path, 2, false);
        insert_view(&conn, 3_600, 1, unknown, path, 4, true);
        // Outside the selected window.
        insert_view(&conn, 14_400, 1, gptbot, path, 99, true);

        let response = build_crawlers_response(&conn, &query(3_600, 10_800)).expect("response");
        assert_eq!(response.rows.len(), 3);
        assert_eq!(response.offset, 0);
        assert_eq!(response.limit, CRAWLERS_PAGE_SIZE);
        assert_eq!(response.next_offset, None);
        assert_eq!(response.category_totals.ai_crawler, 17);
        assert_eq!(response.category_totals.headless, 4);
        assert_eq!(response.rows[0].useragent_id, gptbot);
        assert_eq!(response.rows[0].source, 1);
        assert_eq!(response.rows[0].views, 15);
        assert_eq!(response.rows[0].operator.as_deref(), Some("openai"));
        assert_eq!(response.rows[0].category.as_deref(), Some("ai_crawler"));
        assert!(response.rows[0].verified);

        let tracker = response
            .rows
            .iter()
            .find(|row| row.useragent_id == gptbot && row.source == 0)
            .expect("tracker row");
        assert_eq!(tracker.views, 2);
        assert!(!tracker.verified);

        let unknown_row = response
            .rows
            .iter()
            .find(|row| row.useragent_id == unknown)
            .expect("unknown row");
        assert_eq!(unknown_row.views, 4);
        assert_eq!(unknown_row.operator, None);
        assert_eq!(unknown_row.category, None);
    }

    #[test]
    fn empty_range_returns_no_rows() {
        let conn = open_migrated_db();
        let gptbot = insert_useragent(&conn, "gptbot/1.0", Some("openai"), Some("ai_crawler"));
        insert_view(
            &conn,
            3_600,
            1,
            gptbot,
            insert_crawler_path(&conn),
            10,
            true,
        );
        let response = build_crawlers_response(&conn, &query(10_800, 14_400)).expect("response");
        assert!(response.rows.is_empty());
        assert_eq!(response.category_totals, CrawlerCategoryTotals::default());
        assert!(response.operators.is_empty());
    }

    #[test]
    fn paginates_crawlers_with_limit_and_offset() {
        let conn = open_migrated_db();
        let path = insert_crawler_path(&conn);
        let total = (CRAWLERS_PAGE_SIZE as usize) + 1;
        for index in 0..total {
            let useragent_id = insert_useragent(
                &conn,
                &format!("bot-{index:03}"),
                Some("openai"),
                Some("ai_crawler"),
            );
            insert_view(
                &conn,
                3_600,
                1,
                useragent_id,
                path,
                (total - index) as i64,
                true,
            );
        }

        let mut query = query(3_600, 10_800);
        let first_page = build_crawlers_response(&conn, &query).expect("first page");
        assert_eq!(first_page.rows.len(), CRAWLERS_PAGE_SIZE as usize);
        assert_eq!(first_page.offset, 0);
        assert_eq!(first_page.limit, CRAWLERS_PAGE_SIZE);
        assert_eq!(first_page.next_offset, Some(CRAWLERS_PAGE_SIZE));
        assert_eq!(first_page.rows[0].useragent, "bot-000");
        assert_eq!(
            first_page.rows.last().expect("last").useragent,
            format!("bot-{:03}", CRAWLERS_PAGE_SIZE - 1)
        );
        assert_eq!(
            first_page.category_totals.ai_crawler,
            (1..=total as i64).sum::<i64>()
        );

        query.offset = CRAWLERS_PAGE_SIZE;
        let second_page = build_crawlers_response(&conn, &query).expect("second page");
        assert_eq!(second_page.rows.len(), 1);
        assert_eq!(second_page.next_offset, None);
        assert_eq!(
            second_page.rows[0].useragent,
            format!("bot-{:03}", CRAWLERS_PAGE_SIZE)
        );
        assert_eq!(
            second_page.category_totals.ai_crawler,
            first_page.category_totals.ai_crawler
        );
    }

    #[test]
    fn filters_disabled_categories_from_the_page_only() {
        let conn = open_migrated_db();
        let gptbot = insert_useragent(&conn, "gptbot/1.0", Some("openai"), Some("ai_crawler"));
        let googlebot = insert_useragent(
            &conn,
            "googlebot/1.0",
            Some("google"),
            Some("search_engine"),
        );
        let path = insert_crawler_path(&conn);
        insert_view(&conn, 3_600, 1, gptbot, path, 10, true);
        insert_view(&conn, 3_600, 1, googlebot, path, 4, true);

        let mut query = query(3_600, 10_800);
        query
            .disabled_categories
            .insert("search_engine".to_string());
        let response = build_crawlers_response(&conn, &query).expect("response");
        assert_eq!(response.rows.len(), 1);
        assert_eq!(response.rows[0].useragent_id, gptbot);
        assert_eq!(response.category_totals.ai_crawler, 10);
        assert_eq!(response.category_totals.search_engine, 4);
        assert_eq!(response.operators.len(), 2);
    }

    #[test]
    fn sorts_crawlers_by_useragent() {
        let conn = open_migrated_db();
        let zebra = insert_useragent(&conn, "zebra-bot", Some("openai"), Some("ai_crawler"));
        let alpha = insert_useragent(&conn, "alpha-bot", Some("openai"), Some("ai_crawler"));
        let path = insert_crawler_path(&conn);
        insert_view(&conn, 3_600, 1, zebra, path, 20, true);
        insert_view(&conn, 3_600, 1, alpha, path, 5, true);

        let mut query = query(3_600, 10_800);
        query.order_by = CrawlersOrderBy::Useragent;
        query.direction = SortDirection::Asc;
        let response = build_crawlers_response(&conn, &query).expect("response");
        assert_eq!(response.rows[0].useragent, "alpha-bot");
        assert_eq!(response.rows[1].useragent, "zebra-bot");
    }

    #[test]
    fn verified_tracker_googlebot_is_its_own_row() {
        let conn = open_migrated_db();
        let googlebot = insert_useragent(
            &conn,
            "googlebot/1.0",
            Some("google"),
            Some("search_engine"),
        );
        let path = insert_crawler_path(&conn);
        insert_view(&conn, 3_600, 0, googlebot, path, 3, true);
        insert_view(&conn, 3_600, 0, googlebot, path, 2, false);

        let response = build_crawlers_response(&conn, &query(3_600, 10_800)).expect("response");
        assert_eq!(response.rows.len(), 2);
        let verified = response
            .rows
            .iter()
            .find(|row| row.verified)
            .expect("verified");
        let unverified = response
            .rows
            .iter()
            .find(|row| !row.verified)
            .expect("unverified");
        assert_eq!(verified.views, 3);
        assert_eq!(verified.operator.as_deref(), Some("google"));
        assert_eq!(operator_label_for_row(verified), "Google");
        assert_eq!(unverified.views, 2);
        assert_eq!(unverified.operator.as_deref(), Some("google"));
        assert_eq!(operator_label_for_row(unverified), "Unknown");
        assert_eq!(response.operators.len(), 2);
    }

    #[test]
    fn unverified_rows_never_show_the_operator_they_claim() {
        let conn = open_migrated_db();
        let claims_openai =
            insert_useragent(&conn, "gptbot/1.0", Some("openai"), Some("ai_crawler"));
        let path = insert_crawler_path(&conn);
        insert_view(&conn, 3_600, 0, claims_openai, path, 5, false);

        let response = build_crawlers_response(&conn, &query(3_600, 10_800)).expect("response");
        assert_eq!(operator_label_for_row(&response.rows[0]), "Unknown");
        let serialized = serde_json::to_value(&response).expect("serialize");
        assert!(serialized["rows"][0].get("ip").is_none());
        assert!(serialized["operators"][0].get("ip").is_none());
    }
}
