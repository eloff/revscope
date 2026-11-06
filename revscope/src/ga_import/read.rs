//! Reading imported rows back for reports.
//!
//! Imported rows only answer a date and one dimension, so a report asking for
//! anything narrower (any segment filter) leaves them out; [`ImportedDays::for_report`]
//! decides that in one place.

use std::sync::Arc;

use jiff::ToSpan;
use jiff::civil::Date;
use rusqlite::params;
use serde::Serialize;

use crate::auth::{AuthError, map_row_error};
use crate::authorize::AuthorizedProject;
use crate::db::{ProjectConnection, open_project_database_read};
use crate::query_params::parse_report_query;
use crate::referrer::SourceGroup;
use crate::segment_filter::{SegmentFilters, SourceFilter};
use crate::server_context::ServerContext;
use crate::timezone::{self, ProjectTimezone};

/// The imported dates a report's `[start, end)` covers, inclusive. An imported day
/// counts as a whole, at the moment it starts, so one that only part of the range
/// covers counts if its start is in the range.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ImportedDays {
    from: String,
    until: String,
}

/// Whether reports add imported data at all, by the project's setting.
pub(crate) fn reports_include_imported(conn: &ProjectConnection) -> Result<bool, rusqlite::Error> {
    conn.query_row(
        "SELECT COALESCE((SELECT include_in_reports FROM imported_data_settings WHERE id = 1), 1)",
        [],
        |row| row.get(0),
    )
}

impl ImportedDays {
    pub(crate) fn covering(timezone: &ProjectTimezone, start: i64, end: i64) -> Option<Self> {
        if start >= end {
            return None;
        }
        let first_date = timezone.date_of(start)?;
        let from = if timezone.day_start(first_date)? >= start {
            first_date
        } else {
            first_date.checked_add(1.day()).ok()?
        };
        let until = timezone.date_of(end - 1)?;
        (from <= until).then(|| Self {
            from: from.to_string(),
            until: until.to_string(),
        })
    }

    /// What this report should add: nothing when a filter narrows it, the project
    /// switched imports off, or there are none.
    pub(crate) fn for_report(
        conn: &ProjectConnection,
        timezone: &ProjectTimezone,
        start: i64,
        end: i64,
        segments: &SegmentFilters,
    ) -> Result<Option<Self>, AuthError> {
        if !segments.is_empty() {
            return Ok(None);
        }
        let has_imports: bool = conn
            .query_row("SELECT EXISTS (SELECT 1 FROM imports)", [], |row| {
                row.get(0)
            })
            .map_err(map_row_error)?;
        if !has_imports || !reports_include_imported(conn).map_err(map_row_error)? {
            return Ok(None);
        }
        Ok(Self::covering(timezone, start, end))
    }

    /// Runs a query over `?1 <= date <= ?2`, reading each row with `read`.
    fn collect<T>(
        &self,
        conn: &ProjectConnection,
        sql: &str,
        read: impl Fn(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    ) -> Result<Vec<T>, AuthError> {
        conn.with_query(sql, params![self.from, self.until], |mut rows| {
            let mut out = Vec::new();
            while let Some(row) = rows.next()? {
                out.push(read(row)?);
            }
            Ok(out)
        })
        .map_err(map_row_error)
    }

    pub(crate) fn daily_totals(
        &self,
        conn: &ProjectConnection,
    ) -> Result<Vec<ImportedDayTotals>, AuthError> {
        self.collect(
            conn,
            "SELECT date, SUM(sessions), SUM(engaged_sessions), SUM(engagement_seconds)
             FROM imported_totals
             WHERE date >= ?1 AND date <= ?2
             GROUP BY date",
            |row| {
                let date: String = row.get(0)?;
                Ok(ImportedDayTotals {
                    date: date.parse().ok(),
                    sessions: row.get(1)?,
                    engaged_sessions: row.get(2)?,
                    engagement_seconds: row.get(3)?,
                })
            },
        )
    }

    /// Pages on a live path, with what a page row needs.
    pub(crate) fn pages(&self, conn: &ProjectConnection) -> Result<Vec<ImportedPage>, AuthError> {
        self.collect(
            conn,
            "SELECT p.id, p.path, p.hostname,
                    SUM(i.sessions), SUM(i.pageviews), SUM(i.engagement_seconds)
             FROM imported_pages i
             JOIN paths p ON p.id = i.path_id
             WHERE i.date >= ?1 AND i.date <= ?2 AND p.deleted_at IS NULL
             GROUP BY p.id",
            |row| {
                Ok(ImportedPage {
                    path_id: row.get(0)?,
                    path: row.get(1)?,
                    hostname: row.get(2)?,
                    sessions: row.get(3)?,
                    pageviews: row.get(4)?,
                    engagement_seconds: row.get(5)?,
                    engaged_sessions: 0,
                })
            },
        )
    }

    pub(crate) fn landing_pages(
        &self,
        conn: &ProjectConnection,
    ) -> Result<Vec<ImportedPage>, AuthError> {
        self.collect(
            conn,
            "SELECT p.id, p.path, p.hostname,
                    SUM(i.sessions), SUM(i.engaged_sessions), SUM(i.engagement_seconds)
             FROM imported_landing_pages i
             JOIN paths p ON p.id = i.path_id
             WHERE i.date >= ?1 AND i.date <= ?2 AND p.deleted_at IS NULL
             GROUP BY p.id",
            |row| {
                Ok(ImportedPage {
                    path_id: row.get(0)?,
                    path: row.get(1)?,
                    hostname: row.get(2)?,
                    sessions: row.get(3)?,
                    pageviews: 0,
                    engaged_sessions: row.get(4)?,
                    engagement_seconds: row.get(5)?,
                })
            },
        )
    }

    /// Sessions by `SourceFilter` value and channel. A session from a campaign
    /// since soft-deleted keeps its channel but has no source row, like a tracked
    /// session whose row falls back on a referrer we don't have here.
    pub(crate) fn sources(
        &self,
        conn: &ProjectConnection,
    ) -> Result<Vec<ImportedSourceSessions>, AuthError> {
        let mut sources = self.collect(
            conn,
            "SELECT referrer_id, channel, SUM(sessions)
             FROM imported_referrers
             WHERE date >= ?1 AND date <= ?2
             GROUP BY referrer_id, channel",
            |row| {
                let referrer: i64 = row.get(0)?;
                let referrer = (referrer != 0).then_some(referrer);
                Ok(ImportedSourceSessions {
                    source: Some(SourceFilter::for_landing(None, referrer).param()),
                    channel: row.get(1)?,
                    sessions: row.get(2)?,
                })
            },
        )?;
        sources.extend(self.collect(
            conn,
            "SELECT i.campaign_id, c.deleted_at IS NULL, SUM(i.sessions)
             FROM imported_campaigns i
             JOIN campaigns c ON c.id = i.campaign_id
             WHERE i.date >= ?1 AND i.date <= ?2
             GROUP BY i.campaign_id",
            |row| {
                let campaign: i64 = row.get(0)?;
                let live: bool = row.get(1)?;
                Ok(ImportedSourceSessions {
                    source: live.then(|| SourceFilter::Campaign(campaign).param()),
                    channel: SourceGroup::Campaigns.as_str().to_string(),
                    sessions: row.get(2)?,
                })
            },
        )?);
        Ok(sources)
    }

    pub(crate) fn places(&self, conn: &ProjectConnection) -> Result<Vec<ImportedPlace>, AuthError> {
        self.collect(
            conn,
            "SELECT country, city, SUM(sessions)
             FROM imported_geo
             WHERE date >= ?1 AND date <= ?2
             GROUP BY country, city",
            |row| {
                let city: String = row.get(1)?;
                Ok(ImportedPlace {
                    country: row.get(0)?,
                    city: (!city.is_empty()).then_some(city),
                    sessions: row.get(2)?,
                })
            },
        )
    }

    pub(crate) fn technology(
        &self,
        conn: &ProjectConnection,
    ) -> Result<Vec<ImportedTechnology>, AuthError> {
        self.collect(
            conn,
            "SELECT device, browser_family, os, SUM(sessions)
             FROM imported_technology
             WHERE date >= ?1 AND date <= ?2
             GROUP BY device, browser_family, os",
            |row| {
                Ok(ImportedTechnology {
                    device: row.get(0)?,
                    browser_family: row.get(1)?,
                    os: row.get(2)?,
                    sessions: row.get(3)?,
                })
            },
        )
    }

    /// Custom events on live pages, with the goal an event belongs to, if any.
    pub(crate) fn events(&self, conn: &ProjectConnection) -> Result<Vec<ImportedEvent>, AuthError> {
        self.collect(
            conn,
            "SELECT en.id, en.name, p.id, p.path, p.hostname, goals.id, SUM(i.sessions)
             FROM imported_events i
             JOIN event_names en ON en.id = i.event_name_id
             JOIN paths p ON p.id = i.path_id
             LEFT JOIN (
                SELECT fs.event_name_id, MIN(fs.id) AS id
                FROM funnel_steps fs
                JOIN funnels f ON f.id = fs.funnel_id AND f.deleted_at IS NULL
                WHERE fs.event_name_id IS NOT NULL
                GROUP BY fs.event_name_id
             ) goals ON goals.event_name_id = en.id
             WHERE i.date >= ?1 AND i.date <= ?2
               AND en.deleted_at IS NULL
               AND p.deleted_at IS NULL
             GROUP BY en.id, p.id",
            |row| {
                Ok(ImportedEvent {
                    event_name_id: row.get(0)?,
                    event_name: row.get(1)?,
                    path_id: row.get(2)?,
                    path: row.get(3)?,
                    hostname: row.get(4)?,
                    goal_id: row.get(5)?,
                    sessions: row.get(6)?,
                })
            },
        )
    }
}

/// The imported days of a report that compares its range with an earlier window.
#[derive(Debug, Default)]
pub(crate) struct ImportedWindows {
    pub(crate) current: Option<ImportedDays>,
    pub(crate) previous: Option<ImportedDays>,
}

impl ImportedWindows {
    pub(crate) fn for_report(
        conn: &ProjectConnection,
        timezone: &ProjectTimezone,
        previous_start: i64,
        start: i64,
        end: i64,
        segments: &SegmentFilters,
    ) -> Result<Self, AuthError> {
        Ok(Self {
            current: ImportedDays::for_report(conn, timezone, start, end, segments)?,
            previous: ImportedDays::for_report(conn, timezone, previous_start, start, segments)?,
        })
    }
}

pub(crate) struct ImportedDayTotals {
    pub(crate) date: Option<Date>,
    pub(crate) sessions: i64,
    pub(crate) engaged_sessions: i64,
    pub(crate) engagement_seconds: i64,
}

pub(crate) struct ImportedPage {
    pub(crate) path_id: i64,
    pub(crate) path: String,
    pub(crate) hostname: String,
    pub(crate) sessions: i64,
    pub(crate) pageviews: i64,
    pub(crate) engaged_sessions: i64,
    pub(crate) engagement_seconds: i64,
}

pub(crate) struct ImportedSourceSessions {
    pub(crate) source: Option<String>,
    pub(crate) channel: String,
    pub(crate) sessions: i64,
}

pub(crate) struct ImportedPlace {
    pub(crate) country: String,
    pub(crate) city: Option<String>,
    pub(crate) sessions: i64,
}

pub(crate) struct ImportedTechnology {
    pub(crate) device: String,
    pub(crate) browser_family: String,
    pub(crate) os: String,
    pub(crate) sessions: i64,
}

pub(crate) struct ImportedEvent {
    pub(crate) event_name_id: i64,
    pub(crate) event_name: String,
    pub(crate) path_id: i64,
    pub(crate) path: String,
    pub(crate) hostname: String,
    pub(crate) goal_id: Option<i64>,
    pub(crate) sessions: i64,
}

/// Inclusive date range of an import, for shading the chart.
#[derive(Debug, Serialize, PartialEq)]
pub(crate) struct ImportedRange {
    start_date: String,
    end_date: String,
    source_name: String,
}

#[derive(Debug, Serialize, PartialEq)]
pub(crate) struct ImportedCoverage {
    /// Whether the dashboard adds imported data at all.
    include_in_reports: bool,
    /// Whether imported rows exist for days in the range, whatever the setting.
    has_data: bool,
    /// Whether custom events are among them, since a property may not send any.
    has_events: bool,
    /// The days each import has rows for, within the range.
    ranges: Vec<ImportedRange>,
}

pub(crate) struct CoverageQuery {
    start: i64,
    end: i64,
}

pub(crate) fn parse_coverage_query(target: &str) -> Result<CoverageQuery, &'static str> {
    let range = parse_report_query(target, |_, _| Ok(()))?;
    let (start, end) = range.validate()?;
    Ok(CoverageQuery { start, end })
}

/// What the dashboard needs in order to say when imported data is left out of a
/// view: those decisions (which tab, which filters) are all the browser's to make.
pub(crate) fn imported_coverage(
    server_context: Arc<ServerContext>,
    project: &AuthorizedProject,
    query: CoverageQuery,
) -> Result<ImportedCoverage, AuthError> {
    let timezone = timezone::project_timezone(server_context.clone(), project.key());
    let conn = open_project_database_read(server_context.as_ref(), project.id())?;
    build_coverage(&conn, &timezone, query.start, query.end)
}

fn build_coverage(
    conn: &ProjectConnection,
    timezone: &ProjectTimezone,
    start: i64,
    end: i64,
) -> Result<ImportedCoverage, AuthError> {
    let include_in_reports = reports_include_imported(conn).map_err(map_row_error)?;
    let Some(days) = ImportedDays::covering(timezone, start, end) else {
        return Ok(ImportedCoverage {
            include_in_reports,
            has_data: false,
            has_events: false,
            ranges: Vec::new(),
        });
    };
    // From the rows, not the import's requested dates: one cancelled part way only
    // has the months it reached, and shading the rest would claim data that isn't there.
    let ranges = days.collect(
        conn,
        "SELECT MIN(t.date), MAX(t.date), i.source_name
         FROM imported_totals t
         JOIN imports i ON i.id = t.import_id
         WHERE t.date >= ?1 AND t.date <= ?2
         GROUP BY i.id
         ORDER BY MIN(t.date)",
        |row| {
            Ok(ImportedRange {
                start_date: row.get(0)?,
                end_date: row.get(1)?,
                source_name: row.get(2)?,
            })
        },
    )?;
    let has_events = !days
        .collect(
            conn,
            "SELECT 1 FROM imported_events WHERE date >= ?1 AND date <= ?2 LIMIT 1",
            |_| Ok(()),
        )?
        .is_empty();
    Ok(ImportedCoverage {
        include_in_reports,
        has_data: !ranges.is_empty(),
        has_events,
        ranges,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::segment_filter::parse_segment_filters;
    use crate::test_utils::open_migrated_db;
    use crate::timezone::validate;
    use jiff::civil::date;

    const JAN_5_UTC: i64 = 1_767_571_200;
    const DAY: i64 = 86_400;

    fn seeded() -> ProjectConnection {
        let conn = open_migrated_db();
        conn.execute_batch(
            "INSERT INTO imports VALUES (1, 'google_analytics', 'Acme', '2026-01-01', '2026-01-31');
             INSERT INTO imported_totals VALUES (1, '2026-01-05', 10, 20, 6, 100);",
        )
        .expect("seed");
        conn
    }

    #[test]
    fn a_day_counts_when_its_start_is_in_the_range() {
        let utc = ProjectTimezone::utc();
        let days = ImportedDays::covering(&utc, JAN_5_UTC, JAN_5_UTC + DAY).expect("one day");
        assert_eq!(
            days,
            ImportedDays {
                from: "2026-01-05".to_string(),
                until: "2026-01-05".to_string()
            }
        );
        // Starting an hour into the 5th leaves the 5th out.
        assert_eq!(
            ImportedDays::covering(&utc, JAN_5_UTC + 3600, JAN_5_UTC + DAY),
            None
        );
    }

    #[test]
    fn days_are_read_in_the_projects_timezone() {
        let tokyo = validate("Asia/Tokyo").expect("zone");
        // Midnight on the 5th in Tokyo is 15:00 UTC on the 4th.
        let start = tokyo.day_start(date(2026, 1, 5)).expect("day start");
        let days = ImportedDays::covering(&tokyo, start, start + DAY).expect("one day");
        assert_eq!(days.from, "2026-01-05");
        assert_eq!(days.until, "2026-01-05");
    }

    #[test]
    fn a_filter_or_the_setting_leaves_imported_data_out() {
        let conn = seeded();
        let utc = ProjectTimezone::utc();
        let none = SegmentFilters::default();
        let range = (JAN_5_UTC, JAN_5_UTC + DAY);
        assert!(
            ImportedDays::for_report(&conn, &utc, range.0, range.1, &none)
                .expect("report")
                .is_some()
        );
        let filtered = parse_segment_filters("/x?country=US");
        assert!(!filtered.is_empty());
        assert!(
            ImportedDays::for_report(&conn, &utc, range.0, range.1, &filtered)
                .expect("filtered")
                .is_none()
        );
        crate::ga_import::store::set_include_in_reports(&conn, false).expect("toggle");
        assert!(
            ImportedDays::for_report(&conn, &utc, range.0, range.1, &none)
                .expect("off")
                .is_none()
        );
    }

    #[test]
    fn a_project_without_imports_reads_nothing_extra() {
        let conn = open_migrated_db();
        assert!(
            ImportedDays::for_report(
                &conn,
                &ProjectTimezone::utc(),
                JAN_5_UTC,
                JAN_5_UTC + DAY,
                &SegmentFilters::default()
            )
            .expect("report")
            .is_none()
        );
    }

    #[test]
    fn coverage_only_counts_days_an_import_reached() {
        let conn = seeded();
        let utc = ProjectTimezone::utc();
        let with_rows = build_coverage(&conn, &utc, JAN_5_UTC, JAN_5_UTC + DAY).expect("rows");
        assert!(with_rows.has_data);
        assert_eq!(
            with_rows.ranges,
            vec![ImportedRange {
                start_date: "2026-01-05".to_string(),
                end_date: "2026-01-05".to_string(),
                source_name: "Acme".to_string(),
            }]
        );
        let without =
            build_coverage(&conn, &utc, JAN_5_UTC + DAY, JAN_5_UTC + 2 * DAY).expect("no rows");
        assert!(!without.has_data);
        assert!(
            without.ranges.is_empty(),
            "the import was declared to cover this day"
        );
        let outside =
            build_coverage(&conn, &utc, JAN_5_UTC + 60 * DAY, JAN_5_UTC + 61 * DAY).expect("out");
        assert!(!outside.has_data);
        assert!(outside.ranges.is_empty());
    }
}
