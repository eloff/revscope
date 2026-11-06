use std::sync::Arc;

use rusqlite::params;
use serde::Serialize;

use crate::auth::{AuthError, map_row_error};
use crate::authorize::AuthorizedProject;
use crate::db::{ProjectConnection, open_project_database_read};
use crate::query_params::parse_report_query;
use crate::server_context::ServerContext;
use crate::timezone::{self, BucketGranularity, ProjectTimezone, parse_bucket_granularity};

/// Sanity cap so a crawler that hits many paths cannot dump unbounded rows.
const CRAWLER_DETAIL_PAGES_LIMIT: i64 = 500;
/// Packed last-3-days series: 18 × 4-hour buckets ending at this crawler's latest hour.
const LAST_3_DAYS_BUCKET_SECONDS: i64 = 4 * 3600;
const LAST_3_DAYS_BUCKETS: usize = 18;

#[derive(Debug, Serialize, PartialEq, Eq)]
pub(crate) struct CrawlerSeriesPoint {
    pub(crate) start: i64,
    pub(crate) views: i64,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub(crate) struct CrawlerPageRow {
    pub(crate) path_id: i64,
    pub(crate) path: String,
    pub(crate) hostname: String,
    pub(crate) views: i64,
    /// Views in each 4-hour bucket from [`CrawlerDetailResponse::last_3_days_start`].
    /// Empty means every bucket is zero.
    pub(crate) last_3_days: Vec<i64>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub(crate) struct CrawlerDetailResponse {
    pub(crate) granularity: &'static str,
    pub(crate) series: Vec<CrawlerSeriesPoint>,
    pub(crate) pages: Vec<CrawlerPageRow>,
    /// Unix second at the start of `pages[].last_3_days[0]`. Each later index is +4 hours.
    pub(crate) last_3_days_start: i64,
}

#[derive(Debug)]
pub(crate) struct CrawlerDetailQuery {
    pub(crate) start: i64,
    pub(crate) end: i64,
    pub(crate) useragent_id: i64,
    pub(crate) source: i64,
    pub(crate) verified: i64,
    pub(crate) bucket: BucketGranularity,
}

pub(crate) fn parse_crawler_detail_query(target: &str) -> Result<CrawlerDetailQuery, &'static str> {
    let mut useragent_id = None;
    let mut source = None;
    let mut verified = None;
    let mut bucket = BucketGranularity::default();
    let range = parse_report_query(target, |key, value| {
        match key {
            "useragent_id" => useragent_id = value.parse().ok(),
            "source" => source = value.parse().ok(),
            "verified" => verified = value.parse().ok(),
            "bucket" => bucket = parse_bucket_granularity(value)?,
            _ => {}
        }
        Ok(())
    })?;
    let (start, end) = range.validate()?;
    let useragent_id = useragent_id.ok_or("useragent_id is required")?;
    if useragent_id < 1 {
        return Err("useragent_id must be at least 1");
    }
    let source = source.ok_or("source is required")?;
    if source != 0 && source != 1 {
        return Err("source must be 0 or 1");
    }
    let verified = verified.ok_or("verified is required")?;
    if verified != 0 && verified != 1 {
        return Err("verified must be 0 or 1");
    }
    Ok(CrawlerDetailQuery {
        start,
        end,
        useragent_id,
        source,
        verified,
        bucket,
    })
}

pub(crate) fn crawler_detail(
    server_context: Arc<ServerContext>,
    project: &AuthorizedProject,
    query: CrawlerDetailQuery,
) -> Result<CrawlerDetailResponse, AuthError> {
    let timezone = timezone::project_timezone(server_context.clone(), project.key());
    let conn = open_project_database_read(server_context.as_ref(), project.id())
        .map_err(|error| AuthError::Db(error.to_string()))?;
    build_crawler_detail_response(&conn, &query, &timezone)
}

pub(crate) fn build_crawler_detail_response(
    conn: &ProjectConnection,
    query: &CrawlerDetailQuery,
    timezone: &ProjectTimezone,
) -> Result<CrawlerDetailResponse, AuthError> {
    let buckets = timezone.buckets(query.start, query.end, query.bucket);
    let mut views_by_bucket = vec![0i64; buckets.starts().len()];
    let mut last_hour: Option<i64> = None;
    conn.with_query(
        "SELECT hour, SUM(estimated_count)
         FROM crawler_views_by_hour
         WHERE hour >= ?1 AND hour < ?2
           AND useragent_id = ?3 AND source = ?4 AND verified = ?5
         GROUP BY hour",
        params![
            query.start,
            query.end,
            query.useragent_id,
            query.source,
            query.verified
        ],
        |mut cursor| {
            while let Some(row) = cursor.next()? {
                let hour: i64 = row.get(0)?;
                let views: i64 = row.get(1)?;
                last_hour = Some(last_hour.map_or(hour, |previous| previous.max(hour)));
                if let Some(index) = buckets.index_for_second(hour) {
                    views_by_bucket[index] += views;
                }
            }
            Ok(())
        },
    )
    .map_err(map_row_error)?;

    let series = buckets
        .starts()
        .iter()
        .zip(views_by_bucket)
        .map(|(start, views)| CrawlerSeriesPoint {
            start: *start,
            views,
        })
        .collect();

    let last_3_days_start = last_3_days_window_start(last_hour, query.end);
    let window_join_start = last_3_days_start.max(query.start);
    let mut pages = Vec::new();
    conn.with_query(
        "SELECT totals.path_id, p.path, p.hostname, totals.views,
                cv.hour, cv.estimated_count
         FROM (
            SELECT path_id, SUM(estimated_count) AS views
            FROM crawler_views_by_hour
            WHERE hour >= ?1 AND hour < ?2
              AND useragent_id = ?3 AND source = ?4 AND verified = ?5
            GROUP BY path_id
            ORDER BY views DESC
            LIMIT ?6
         ) totals
         JOIN crawler_paths p ON p.id = totals.path_id
         LEFT JOIN crawler_views_by_hour cv
            ON cv.path_id = totals.path_id
           AND cv.useragent_id = ?3
           AND cv.source = ?4
           AND cv.verified = ?5
           AND cv.hour >= ?7
           AND cv.hour < ?2
         ORDER BY totals.views DESC, totals.path_id ASC, cv.hour ASC",
        params![
            query.start,
            query.end,
            query.useragent_id,
            query.source,
            query.verified,
            CRAWLER_DETAIL_PAGES_LIMIT,
            window_join_start
        ],
        |mut cursor| {
            let mut current: Option<CrawlerPageRow> = None;
            while let Some(row) = cursor.next()? {
                let path_id: i64 = row.get(0)?;
                if current.as_ref().is_none_or(|page| page.path_id != path_id) {
                    if let Some(page) = current.take() {
                        pages.push(finish_page(page));
                    }
                    current = Some(CrawlerPageRow {
                        path_id,
                        path: row.get(1)?,
                        hostname: row.get(2)?,
                        views: row.get(3)?,
                        last_3_days: vec![0; LAST_3_DAYS_BUCKETS],
                    });
                }
                let hour: Option<i64> = row.get(4)?;
                let hour_views: Option<i64> = row.get(5)?;
                if let (Some(hour), Some(hour_views), Some(page)) =
                    (hour, hour_views, current.as_mut())
                    && let Some(index) = last_3_days_index(hour, last_3_days_start)
                {
                    page.last_3_days[index] += hour_views;
                }
            }
            if let Some(page) = current {
                pages.push(finish_page(page));
            }
            Ok(())
        },
    )
    .map_err(map_row_error)?;

    Ok(CrawlerDetailResponse {
        granularity: query.bucket.label(),
        series,
        pages,
        last_3_days_start,
    })
}

fn last_3_days_window_start(last_hour: Option<i64>, range_end: i64) -> i64 {
    let last_bucket_start = match last_hour {
        Some(hour) => hour - hour.rem_euclid(LAST_3_DAYS_BUCKET_SECONDS),
        None => range_end - LAST_3_DAYS_BUCKET_SECONDS,
    };
    last_bucket_start - (LAST_3_DAYS_BUCKETS as i64 - 1) * LAST_3_DAYS_BUCKET_SECONDS
}

fn finish_page(mut page: CrawlerPageRow) -> CrawlerPageRow {
    if page.last_3_days.iter().all(|views| *views == 0) {
        page.last_3_days.clear();
    }
    page
}

fn last_3_days_index(hour: i64, last_3_days_start: i64) -> Option<usize> {
    let offset = hour.checked_sub(last_3_days_start)?;
    if offset < 0 {
        return None;
    }
    let index = (offset / LAST_3_DAYS_BUCKET_SECONDS) as usize;
    (index < LAST_3_DAYS_BUCKETS).then_some(index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::open_migrated_db;

    use rusqlite::params;

    use crate::db::ProjectConnection;

    fn insert_useragent(conn: &ProjectConnection, useragent: &str) -> i64 {
        conn.execute(
            "INSERT INTO useragents (useragent) VALUES (?1)",
            params![useragent],
        )
        .expect("insert useragent");
        conn.query_row(
            "SELECT id FROM useragents WHERE useragent = ?1",
            params![useragent],
            |row| row.get::<_, i64>(0),
        )
        .expect("useragent id")
    }

    fn insert_crawler_path(conn: &ProjectConnection, path: &str, hostname: &str) -> i64 {
        conn.execute(
            "INSERT INTO crawler_paths (path, hostname) VALUES (?1, ?2)",
            params![path, hostname],
        )
        .expect("insert crawler path");
        conn.query_row(
            "SELECT id FROM crawler_paths WHERE path = ?1 AND hostname = ?2",
            params![path, hostname],
            |row| row.get::<_, i64>(0),
        )
        .expect("crawler path id")
    }

    fn insert_view(
        conn: &ProjectConnection,
        hour: i64,
        source: i64,
        useragent_id: i64,
        path_id: i64,
        estimated_count: i64,
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
                i64::from(source == 1)
            ],
        )
        .expect("insert crawler view");
    }

    fn query(
        start: i64,
        end: i64,
        useragent_id: i64,
        source: i64,
        bucket: BucketGranularity,
    ) -> CrawlerDetailQuery {
        CrawlerDetailQuery {
            start,
            end,
            useragent_id,
            source,
            verified: i64::from(source == 1),
            bucket,
        }
    }

    #[test]
    fn parses_crawler_detail_query() {
        let parsed = parse_crawler_detail_query(
            "/v2/read/projects/1/crawler-detail?start=10&end=100&useragent_id=4&source=1&verified=1&bucket=hourly",
        )
        .expect("query");
        assert_eq!(parsed.start, 10);
        assert_eq!(parsed.end, 100);
        assert_eq!(parsed.useragent_id, 4);
        assert_eq!(parsed.source, 1);
        assert_eq!(parsed.verified, 1);
        assert_eq!(parsed.bucket, BucketGranularity::Hourly);
    }

    #[test]
    fn rejects_invalid_source() {
        assert!(
            parse_crawler_detail_query(
                "/v2/read/projects/1/crawler-detail?start=10&end=100&useragent_id=4&source=2&verified=1",
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_missing_or_invalid_verified() {
        assert_eq!(
            parse_crawler_detail_query(
                "/v2/read/projects/1/crawler-detail?start=10&end=100&useragent_id=4&source=1",
            )
            .unwrap_err(),
            "verified is required"
        );
        assert_eq!(
            parse_crawler_detail_query(
                "/v2/read/projects/1/crawler-detail?start=10&end=100&useragent_id=4&source=1&verified=2",
            )
            .unwrap_err(),
            "verified must be 0 or 1"
        );
    }

    #[test]
    fn buckets_hourly_views_and_zero_fills() {
        let conn = open_migrated_db();
        let ua = insert_useragent(&conn, "gptbot/1.0");
        let path = insert_crawler_path(&conn, "/", "acme.test");
        // 1970-01-01 01:00 and 03:00 UTC; the 02:00 bucket stays empty.
        insert_view(&conn, 3_600, 1, ua, path, 7);
        insert_view(&conn, 10_800, 1, ua, path, 3);
        insert_view(&conn, 3_600, 0, ua, path, 99);

        let response = build_crawler_detail_response(
            &conn,
            &query(3_600, 14_400, ua, 1, BucketGranularity::Hourly),
            &ProjectTimezone::utc(),
        )
        .expect("response");

        assert_eq!(response.granularity, "hour");
        assert_eq!(response.series.len(), 3);
        assert_eq!(
            response.series,
            vec![
                CrawlerSeriesPoint {
                    start: 3_600,
                    views: 7
                },
                CrawlerSeriesPoint {
                    start: 7_200,
                    views: 0
                },
                CrawlerSeriesPoint {
                    start: 10_800,
                    views: 3
                },
            ]
        );
        assert_eq!(
            response.last_3_days_start,
            last_3_days_window_start(Some(10_800), 14_400)
        );
    }

    #[test]
    fn daily_buckets_sum_hours() {
        let conn = open_migrated_db();
        let ua = insert_useragent(&conn, "gptbot/1.0");
        let path = insert_crawler_path(&conn, "/", "acme.test");
        insert_view(&conn, 3_600, 1, ua, path, 4);
        insert_view(&conn, 86_400 + 3_600, 1, ua, path, 6);

        let response = build_crawler_detail_response(
            &conn,
            &query(0, 2 * 86_400, ua, 1, BucketGranularity::Daily),
            &ProjectTimezone::utc(),
        )
        .expect("response");

        assert_eq!(response.granularity, "day");
        assert_eq!(response.series.len(), 2);
        assert_eq!(response.series[0].start, 0);
        assert_eq!(response.series[0].views, 4);
        assert_eq!(response.series[1].start, 86_400);
        assert_eq!(response.series[1].views, 6);
    }

    #[test]
    fn groups_pages_by_crawler_path() {
        let conn = open_migrated_db();
        let ua = insert_useragent(&conn, "gptbot/1.0");
        let home = insert_crawler_path(&conn, "/", "acme.test");
        let robots = insert_crawler_path(&conn, "/robots.txt", "acme.test");
        insert_view(&conn, 3_600, 1, ua, home, 8);
        insert_view(&conn, 3_600, 1, ua, robots, 5);
        insert_view(&conn, 7_200, 1, ua, robots, 2);

        let response = build_crawler_detail_response(
            &conn,
            &query(0, 14_400, ua, 1, BucketGranularity::Hourly),
            &ProjectTimezone::utc(),
        )
        .expect("response");

        assert_eq!(response.pages.len(), 2);
        assert_eq!(response.pages[0].path_id, home);
        assert_eq!(response.pages[0].path, "/");
        assert_eq!(response.pages[0].hostname, "acme.test");
        assert_eq!(response.pages[0].views, 8);
        assert_eq!(response.pages[1].path_id, robots);
        assert_eq!(response.pages[1].path, "/robots.txt");
        assert_eq!(response.pages[1].views, 7);
        assert_eq!(response.pages[0].last_3_days.len(), LAST_3_DAYS_BUCKETS);
        assert_eq!(response.pages[1].last_3_days.len(), LAST_3_DAYS_BUCKETS);
        assert_eq!(
            response.pages[0].last_3_days.iter().sum::<i64>(),
            response.pages[0].views
        );
        assert_eq!(
            response.pages[1].last_3_days.iter().sum::<i64>(),
            response.pages[1].views
        );
    }

    #[test]
    fn caps_pages_at_limit() {
        let conn = open_migrated_db();
        let ua = insert_useragent(&conn, "gptbot/1.0");
        let total = (CRAWLER_DETAIL_PAGES_LIMIT as usize) + 1;
        for index in 0..total {
            let path_id = insert_crawler_path(&conn, &format!("/p{index}"), "acme.test");
            insert_view(&conn, 3_600, 1, ua, path_id, (index + 1) as i64);
        }

        let response = build_crawler_detail_response(
            &conn,
            &query(0, 14_400, ua, 1, BucketGranularity::Hourly),
            &ProjectTimezone::utc(),
        )
        .expect("response");

        assert_eq!(response.pages.len(), CRAWLER_DETAIL_PAGES_LIMIT as usize);
        assert_eq!(response.pages[0].views, total as i64);
        assert_eq!(response.pages.last().map(|row| row.views), Some(2));
    }

    #[test]
    fn packs_last_three_days_into_four_hour_buckets() {
        let conn = open_migrated_db();
        let ua = insert_useragent(&conn, "gptbot/1.0");
        let home = insert_crawler_path(&conn, "/", "acme.test");
        let robots = insert_crawler_path(&conn, "/robots.txt", "acme.test");
        let end = 30 * 86_400;
        let window_start = last_3_days_window_start(None, end);
        // Older than the last-3-days window: totals only.
        insert_view(&conn, 3_600, 1, ua, home, 10);
        insert_view(&conn, 7_200, 1, ua, robots, 4);
        // Two hours in bucket 0, one hour in bucket 1.
        insert_view(&conn, window_start, 1, ua, home, 3);
        insert_view(&conn, window_start + 3_600, 1, ua, home, 2);
        insert_view(
            &conn,
            window_start + LAST_3_DAYS_BUCKET_SECONDS,
            1,
            ua,
            home,
            7,
        );
        // Last 4-hour bucket.
        insert_view(
            &conn,
            window_start + (LAST_3_DAYS_BUCKETS as i64 - 1) * LAST_3_DAYS_BUCKET_SECONDS,
            1,
            ua,
            home,
            1,
        );

        let response = build_crawler_detail_response(
            &conn,
            &query(0, end, ua, 1, BucketGranularity::Daily),
            &ProjectTimezone::utc(),
        )
        .expect("response");

        assert_eq!(response.last_3_days_start, window_start);
        assert_eq!(response.pages.len(), 2);
        assert_eq!(response.pages[0].path_id, home);
        assert_eq!(response.pages[0].views, 23);
        assert_eq!(response.pages[0].last_3_days.len(), LAST_3_DAYS_BUCKETS);
        assert_eq!(response.pages[0].last_3_days[0], 5);
        assert_eq!(response.pages[0].last_3_days[1], 7);
        assert_eq!(response.pages[0].last_3_days[LAST_3_DAYS_BUCKETS - 1], 1);
        assert!(
            response.pages[0].last_3_days[2..LAST_3_DAYS_BUCKETS - 1]
                .iter()
                .all(|views| *views == 0)
        );
        assert_eq!(response.pages[1].path_id, robots);
        assert_eq!(response.pages[1].views, 4);
        assert_eq!(response.pages[1].last_3_days, Vec::<i64>::new());
    }

    #[test]
    fn anchors_last_3_days_to_latest_hour_not_range_end() {
        let conn = open_migrated_db();
        let ua = insert_useragent(&conn, "gptbot/1.0");
        let home = insert_crawler_path(&conn, "/", "acme.test");
        let end = 30 * 86_400;
        // 12 hours before the range end, on a 4-hour boundary.
        let last_hour = end - 12 * 3_600;
        insert_view(&conn, last_hour, 1, ua, home, 9);
        insert_view(
            &conn,
            last_hour - (LAST_3_DAYS_BUCKETS as i64 - 1) * LAST_3_DAYS_BUCKET_SECONDS,
            1,
            ua,
            home,
            4,
        );

        let response = build_crawler_detail_response(
            &conn,
            &query(0, end, ua, 1, BucketGranularity::Daily),
            &ProjectTimezone::utc(),
        )
        .expect("response");

        let window_start = last_3_days_window_start(Some(last_hour), end);
        assert_eq!(response.last_3_days_start, window_start);
        assert_ne!(
            response.last_3_days_start,
            last_3_days_window_start(None, end)
        );
        assert_eq!(response.pages[0].last_3_days[0], 4);
        assert_eq!(response.pages[0].last_3_days[LAST_3_DAYS_BUCKETS - 1], 9);
    }
}
