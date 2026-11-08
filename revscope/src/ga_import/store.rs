//! Writes imported rows into the project's SQLite database.

use fxhash::FxHashMap;
use rusqlite::{OptionalExtension, params};

use crate::db::ProjectConnection;
use crate::ingest::{
    LandingCampaignTuple, resolve_event_name_id, resolve_landing_campaign_id, resolve_path_id,
    resolve_referrer_id,
};
use crate::referrer::SourceGroup;
use crate::timezone::ProjectTimezone;
use crate::untrusted_keys::UntrustedKeyMap;
use crate::util::NANOS_PER_SECOND;

use super::api::ReportRow;
use super::reports::{
    ImportedSource, ReportKind, browser_slug, classify_source, device_slug, is_rollup, known_value,
    os_slug, page_key, parse_ga_date,
};
use super::start_import::MAX_HOSTNAMES;

/// Every table holding an import's rows. Deleting goes through this list because
/// release builds don't enforce foreign keys, so the schema's cascades don't run.
const IMPORTED_TABLES: [&str; 8] = [
    "imported_totals",
    "imported_pages",
    "imported_landing_pages",
    "imported_referrers",
    "imported_campaigns",
    "imported_geo",
    "imported_technology",
    "imported_events",
];

fn tables_of(kind: ReportKind) -> &'static [&'static str] {
    match kind {
        ReportKind::Totals => &["imported_totals"],
        ReportKind::Pages => &["imported_pages"],
        ReportKind::LandingPages => &["imported_landing_pages"],
        ReportKind::Sources => &["imported_referrers", "imported_campaigns"],
        ReportKind::Geo => &["imported_geo"],
        ReportKind::Technology => &["imported_technology"],
        ReportKind::Events => &["imported_events"],
    }
}

pub(super) fn insert_import_row(
    conn: &ProjectConnection,
    import_id: i32,
    source_name: &str,
    start_date: &str,
    end_date: &str,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO imports (id, source, source_name, start_date, end_date)
         VALUES (?1, 'google_analytics', ?2, ?3, ?4)",
        params![import_id, source_name, start_date, end_date],
    )?;
    Ok(())
}

pub(super) fn delete_import_rows(
    conn: &ProjectConnection,
    import_id: i32,
) -> Result<(), rusqlite::Error> {
    conn.execute("BEGIN IMMEDIATE", [])?;
    let deleted = (|| {
        for table in IMPORTED_TABLES {
            conn.execute(
                &format!("DELETE FROM {table} WHERE import_id = ?1"),
                params![import_id],
            )?;
        }
        conn.execute("DELETE FROM imports WHERE id = ?1", params![import_id])?;
        Ok(())
    })();
    finish_transaction(conn, deleted)
}

fn finish_transaction<T>(
    conn: &ProjectConnection,
    result: Result<T, rusqlite::Error>,
) -> Result<T, rusqlite::Error> {
    match result {
        Ok(value) => {
            conn.execute("COMMIT", [])?;
            Ok(value)
        }
        Err(error) => {
            if let Err(rollback_error) = conn.execute("ROLLBACK", []) {
                log::warn!("failed to roll back imported rows: {rollback_error}");
            }
            Err(error)
        }
    }
}

/// Replaces one report's rows for one month, in one transaction, so a chunk that
/// ran twice after a restart can't count anything twice. Returns false without
/// writing when the import has been deleted: checking inside the write transaction
/// is what stops a chunk in flight from leaving rows behind a delete.
pub(super) fn write_chunk(
    conn: &ProjectConnection,
    import_id: i32,
    kind: ReportKind,
    from: &str,
    to: &str,
    timezone: &ProjectTimezone,
    rows: &[ReportRow],
) -> Result<bool, rusqlite::Error> {
    conn.execute("BEGIN IMMEDIATE", [])?;
    let written = (|| {
        let import_exists = conn
            .query_row(
                "SELECT 1 FROM imports WHERE id = ?1",
                params![import_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !import_exists {
            return Ok(false);
        }
        for table in tables_of(kind) {
            conn.execute(
                &format!("DELETE FROM {table} WHERE import_id = ?1 AND date >= ?2 AND date <= ?3"),
                params![import_id, from, to],
            )?;
        }
        let mut lookups = Lookups::default();
        for row in rows {
            let Some(date) = row.dimensions.first().and_then(|date| parse_ga_date(date)) else {
                continue;
            };
            write_row(conn, import_id, kind, &date, row, timezone, &mut lookups)?;
        }
        Ok(true)
    })();
    finish_transaction(conn, written)
}

/// Ids resolved so far in this chunk, since the same page recurs on every day of it.
#[derive(Default)]
struct Lookups {
    paths: UntrustedKeyMap<(String, String), i64>,
    referrers: UntrustedKeyMap<String, i64>,
    events: UntrustedKeyMap<String, i64>,
}

impl Lookups {
    fn path(
        &mut self,
        conn: &ProjectConnection,
        hostname: &str,
        path: &str,
    ) -> Result<Option<i64>, rusqlite::Error> {
        let Some(key) = page_key(hostname, path) else {
            return Ok(None);
        };
        if let Some(id) = self.paths.get(&key) {
            return Ok(Some(*id));
        }
        let id = resolve_path_id(conn, &key.1, &key.0)?;
        if let Some(id) = id {
            self.paths.insert(key, id);
        }
        Ok(id)
    }

    fn referrer(
        &mut self,
        conn: &ProjectConnection,
        host: &str,
    ) -> Result<Option<i64>, rusqlite::Error> {
        if let Some(id) = self.referrers.get(host) {
            return Ok(Some(*id));
        }
        let id = resolve_referrer_id(conn, host)?;
        if let Some(id) = id {
            self.referrers.insert(host.to_string(), id);
        }
        Ok(id)
    }

    fn event(
        &mut self,
        conn: &ProjectConnection,
        name: &str,
    ) -> Result<Option<i64>, rusqlite::Error> {
        if let Some(id) = self.events.get(name) {
            return Ok(Some(*id));
        }
        let id = resolve_event_name_id(conn, name)?;
        if let Some(id) = id {
            self.events.insert(name.to_string(), id);
        }
        Ok(id)
    }
}

/// Our normalization can fold GA's distinct values into one (two capitalizations of
/// a host, several browsers into `other`), so every insert adds to an existing row.
fn write_row(
    conn: &ProjectConnection,
    import_id: i32,
    kind: ReportKind,
    date: &str,
    row: &ReportRow,
    timezone: &ProjectTimezone,
    lookups: &mut Lookups,
) -> Result<(), rusqlite::Error> {
    let dimension = |index: usize| row.dimensions.get(index).map_or("", String::as_str);
    let metric = |index: usize| row.metrics.get(index).copied().unwrap_or(0);
    match kind {
        ReportKind::Totals => {
            conn.execute(
                "INSERT INTO imported_totals
                    (import_id, date, sessions, pageviews, engaged_sessions, engagement_seconds)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT DO UPDATE SET
                    sessions = sessions + excluded.sessions,
                    pageviews = pageviews + excluded.pageviews,
                    engaged_sessions = engaged_sessions + excluded.engaged_sessions,
                    engagement_seconds = engagement_seconds + excluded.engagement_seconds",
                params![import_id, date, metric(0), metric(1), metric(2), metric(3)],
            )?;
        }
        ReportKind::Pages => {
            if let Some(path_id) = lookups.path(conn, dimension(1), dimension(2))? {
                conn.execute(
                    "INSERT INTO imported_pages
                        (import_id, date, path_id, sessions, pageviews, engagement_seconds)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT DO UPDATE SET
                        sessions = sessions + excluded.sessions,
                        pageviews = pageviews + excluded.pageviews,
                        engagement_seconds = engagement_seconds + excluded.engagement_seconds",
                    params![import_id, date, path_id, metric(0), metric(1), metric(2)],
                )?;
            }
        }
        ReportKind::LandingPages => {
            if let Some(path_id) = lookups.path(conn, dimension(1), dimension(2))? {
                conn.execute(
                    "INSERT INTO imported_landing_pages
                        (import_id, date, path_id, sessions, engaged_sessions, engagement_seconds)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT DO UPDATE SET
                        sessions = sessions + excluded.sessions,
                        engaged_sessions = engaged_sessions + excluded.engaged_sessions,
                        engagement_seconds = engagement_seconds + excluded.engagement_seconds",
                    params![import_id, date, path_id, metric(0), metric(1), metric(2)],
                )?;
            }
        }
        ReportKind::Sources => {
            if is_rollup(dimension(1)) {
                return Ok(());
            }
            let source = classify_source(dimension(1), dimension(2), dimension(3), dimension(4));
            write_source(conn, import_id, date, source, metric(0), timezone, lookups)?;
        }
        ReportKind::Geo => {
            let Some(country) = known_value(dimension(1))
                .filter(|code| code.len() == 2)
                .map(str::to_ascii_uppercase)
            else {
                return Ok(());
            };
            conn.execute(
                "INSERT INTO imported_geo (import_id, date, country, region, city, sessions)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT DO UPDATE SET sessions = sessions + excluded.sessions",
                params![
                    import_id,
                    date,
                    country,
                    known_value(dimension(2)).unwrap_or(""),
                    known_value(dimension(3)).unwrap_or(""),
                    metric(0)
                ],
            )?;
        }
        ReportKind::Technology => {
            conn.execute(
                "INSERT INTO imported_technology
                    (import_id, date, device, browser_family, os, sessions)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT DO UPDATE SET sessions = sessions + excluded.sessions",
                params![
                    import_id,
                    date,
                    device_slug(dimension(1)),
                    browser_slug(dimension(2)),
                    os_slug(dimension(3)),
                    metric(0)
                ],
            )?;
        }
        ReportKind::Events => {
            let Some(name) = known_value(dimension(1)) else {
                return Ok(());
            };
            let Some(path_id) = lookups.path(conn, dimension(2), dimension(3))? else {
                return Ok(());
            };
            if let Some(event_name_id) = lookups.event(conn, name)? {
                conn.execute(
                    "INSERT INTO imported_events
                        (import_id, date, event_name_id, path_id, sessions, event_count)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT DO UPDATE SET
                        sessions = sessions + excluded.sessions,
                        event_count = event_count + excluded.event_count",
                    params![
                        import_id,
                        date,
                        event_name_id,
                        path_id,
                        metric(0),
                        metric(1)
                    ],
                )?;
            }
        }
    }
    Ok(())
}

fn write_source(
    conn: &ProjectConnection,
    import_id: i32,
    date: &str,
    source: ImportedSource,
    sessions: i64,
    timezone: &ProjectTimezone,
    lookups: &mut Lookups,
) -> Result<(), rusqlite::Error> {
    let (referrer_id, channel) = match source {
        ImportedSource::Direct => (0, SourceGroup::Direct),
        ImportedSource::Referrer { host, channel } => match lookups.referrer(conn, &host)? {
            Some(id) => (id, channel),
            None => return Ok(()),
        },
        ImportedSource::Campaign {
            source,
            medium,
            campaign,
        } => {
            let tuple = LandingCampaignTuple {
                source,
                medium,
                campaign,
                ..LandingCampaignTuple::default()
            };
            let first_seen_at = date
                .parse()
                .ok()
                .and_then(|day| timezone.day_start(day))
                .unwrap_or(0)
                .saturating_mul(NANOS_PER_SECOND);
            if let Some(campaign_id) = resolve_landing_campaign_id(conn, &tuple, first_seen_at)? {
                conn.execute(
                    "INSERT INTO imported_campaigns (import_id, date, campaign_id, sessions)
                     VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT DO UPDATE SET sessions = sessions + excluded.sessions",
                    params![import_id, date, campaign_id, sessions],
                )?;
            }
            return Ok(());
        }
    };
    conn.execute(
        "INSERT INTO imported_referrers (import_id, date, referrer_id, channel, sessions)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT DO UPDATE SET sessions = sessions + excluded.sessions",
        params![import_id, date, referrer_id, channel.as_str(), sessions],
    )?;
    Ok(())
}

/// Sessions written so far, for the import list.
pub(super) fn sessions_by_import(
    conn: &ProjectConnection,
) -> Result<FxHashMap<i32, i64>, rusqlite::Error> {
    conn.with_query(
        "SELECT import_id, SUM(sessions) FROM imported_totals GROUP BY import_id",
        [],
        |mut rows| {
            let mut sessions = FxHashMap::default();
            while let Some(row) = rows.next()? {
                sessions.insert(row.get(0)?, row.get(1)?);
            }
            Ok(sessions)
        },
    )
}

pub(super) fn set_include_in_reports(
    conn: &ProjectConnection,
    include: bool,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO imported_data_settings (id, include_in_reports) VALUES (1, ?1)
         ON CONFLICT (id) DO UPDATE SET include_in_reports = excluded.include_in_reports",
        params![include],
    )?;
    Ok(())
}

/// Hostnames we've seen pages on, to suggest as the import's filter. None when
/// there are too many to list, since a partial list would silently narrow the import.
pub(super) fn known_hostnames(conn: &ProjectConnection) -> Result<Vec<String>, rusqlite::Error> {
    conn.with_query(
        "SELECT DISTINCT hostname FROM paths
         WHERE deleted_at IS NULL AND hostname != ''
         ORDER BY hostname",
        [],
        |mut rows| {
            let mut hostnames = Vec::new();
            while let Some(row) = rows.next()? {
                if hostnames.len() == MAX_HOSTNAMES {
                    return Ok(Vec::new());
                }
                hostnames.push(row.get(0)?);
            }
            Ok(hostnames)
        },
    )
}

/// When the project's own tracking starts.
pub(super) fn first_tracked_second(
    conn: &ProjectConnection,
) -> Result<Option<i64>, rusqlite::Error> {
    conn.query_row("SELECT MIN(timestamp_id) FROM pageviews", [], |row| {
        row.get::<_, Option<i64>>(0)
    })
    .map(|first| first.map(|nanos| nanos / NANOS_PER_SECOND))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::open_migrated_db;

    fn row(dimensions: &[&str], metrics: &[i64]) -> ReportRow {
        ReportRow {
            dimensions: dimensions.iter().map(|value| value.to_string()).collect(),
            metrics: metrics.to_vec(),
        }
    }

    fn seeded_import(conn: &ProjectConnection) {
        insert_import_row(conn, 7, "Acme", "2026-01-01", "2026-01-31").expect("import");
    }

    fn write(conn: &ProjectConnection, kind: ReportKind, rows: &[ReportRow]) -> bool {
        write_chunk(
            conn,
            7,
            kind,
            "2026-01-01",
            "2026-01-31",
            &ProjectTimezone::utc(),
            rows,
        )
        .expect("write chunk")
    }

    fn count(conn: &ProjectConnection, table: &str) -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .expect("count")
    }

    #[test]
    fn too_many_hostnames_suggest_none_rather_than_some() {
        let conn = open_migrated_db();
        for index in 0..MAX_HOSTNAMES {
            conn.execute(
                "INSERT INTO paths (path, hostname) VALUES ('/', ?1)",
                params![format!("h{index}.test")],
            )
            .expect("path");
        }
        assert_eq!(known_hostnames(&conn).expect("list").len(), MAX_HOSTNAMES);
        conn.execute(
            "INSERT INTO paths (path, hostname) VALUES ('/', 'one-more.test')",
            [],
        )
        .expect("path");
        assert!(known_hostnames(&conn).expect("list").is_empty());
    }

    #[test]
    fn running_a_chunk_twice_replaces_its_rows() {
        let conn = open_migrated_db();
        seeded_import(&conn);
        let rows = [row(&["20260105"], &[10, 30, 6, 120])];
        assert!(write(&conn, ReportKind::Totals, &rows));
        assert!(write(&conn, ReportKind::Totals, &rows));
        let sessions: i64 = conn
            .query_row("SELECT SUM(sessions) FROM imported_totals", [], |row| {
                row.get(0)
            })
            .expect("sum");
        assert_eq!(sessions, 10);
    }

    #[test]
    fn a_chunk_for_a_deleted_import_writes_nothing() {
        let conn = open_migrated_db();
        seeded_import(&conn);
        delete_import_rows(&conn, 7).expect("delete");
        assert!(!write(
            &conn,
            ReportKind::Totals,
            &[row(&["20260105"], &[10, 30, 6, 120])]
        ));
        assert_eq!(count(&conn, "imported_totals"), 0);
    }

    #[test]
    fn deleting_an_import_clears_every_table_but_only_its_own_rows() {
        let conn = open_migrated_db();
        seeded_import(&conn);
        insert_import_row(&conn, 8, "Other", "2025-01-01", "2025-01-31").expect("other");
        conn.execute_batch("INSERT INTO imported_totals VALUES (8, '2025-01-05', 1, 1, 1, 1);")
            .expect("other rows");
        let reports = [
            (ReportKind::Totals, row(&["20260105"], &[10, 30, 6, 120])),
            (
                ReportKind::Pages,
                row(&["20260105", "acme.test", "/a"], &[1, 2, 3]),
            ),
            (
                ReportKind::LandingPages,
                row(&["20260105", "acme.test", "/a"], &[1, 1, 3]),
            ),
            (
                ReportKind::Sources,
                row(
                    &["20260105", "(direct)", "(none)", "(direct)", "Direct"],
                    &[4],
                ),
            ),
            (
                ReportKind::Sources,
                row(&["20260105", "news", "email", "launch", "Email"], &[4]),
            ),
            (
                ReportKind::Geo,
                row(&["20260105", "US", "New York", "Albany"], &[5]),
            ),
            (
                ReportKind::Technology,
                row(&["20260105", "mobile", "Chrome", "Android"], &[6]),
            ),
            (
                ReportKind::Events,
                row(&["20260105", "signup", "acme.test", "/a"], &[1, 2]),
            ),
        ];
        for (kind, report_row) in &reports {
            write_row(
                &conn,
                7,
                *kind,
                "2026-01-05",
                report_row,
                &ProjectTimezone::utc(),
                &mut Lookups::default(),
            )
            .expect("write row");
        }
        for table in IMPORTED_TABLES {
            assert!(count(&conn, table) > 0, "{table} should have rows");
        }
        delete_import_rows(&conn, 7).expect("delete");
        for table in IMPORTED_TABLES {
            let own: i64 = conn
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE import_id = 7"),
                    [],
                    |row| row.get(0),
                )
                .expect("count");
            assert_eq!(own, 0, "{table} kept rows of the deleted import");
        }
        assert_eq!(count(&conn, "imported_totals"), 1);
        assert_eq!(count(&conn, "imports"), 1);
    }

    #[test]
    fn values_ga_folds_together_add_up_and_placeholders_are_dropped() {
        let conn = open_migrated_db();
        seeded_import(&conn);
        write(
            &conn,
            ReportKind::Pages,
            &[
                row(&["20260105", "ACME.test", "/a"], &[1, 2, 3]),
                row(&["20260105", "acme.test", "/a"], &[1, 2, 3]),
                row(&["20260105", "acme.test", "(other)"], &[9, 9, 9]),
            ],
        );
        let (rows, sessions): (i64, i64) = conn
            .query_row(
                "SELECT COUNT(*), SUM(sessions) FROM imported_pages",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("pages");
        assert_eq!((rows, sessions), (1, 2));
    }

    #[test]
    fn gas_rollup_row_is_not_direct_traffic() {
        let conn = open_migrated_db();
        seeded_import(&conn);
        write(
            &conn,
            ReportKind::Sources,
            &[row(
                &["20260105", "(other)", "(other)", "(other)", "(other)"],
                &[5_000],
            )],
        );
        assert_eq!(count(&conn, "imported_referrers"), 0);
        assert_eq!(count(&conn, "imported_campaigns"), 0);
    }

    #[test]
    fn sources_land_in_referrers_or_campaigns_never_both() {
        let conn = open_migrated_db();
        seeded_import(&conn);
        write(
            &conn,
            ReportKind::Sources,
            &[
                row(
                    &["20260105", "(direct)", "(none)", "(direct)", "Direct"],
                    &[1],
                ),
                row(
                    &[
                        "20260105",
                        "google",
                        "organic",
                        "(organic)",
                        "Organic Search",
                    ],
                    &[2],
                ),
                row(&["20260105", "news", "email", "launch", "Email"], &[4]),
            ],
        );
        let referrer_sessions: i64 = conn
            .query_row("SELECT SUM(sessions) FROM imported_referrers", [], |row| {
                row.get(0)
            })
            .expect("referrers");
        let campaign_sessions: i64 = conn
            .query_row("SELECT SUM(sessions) FROM imported_campaigns", [], |row| {
                row.get(0)
            })
            .expect("campaigns");
        assert_eq!((referrer_sessions, campaign_sessions), (3, 4));
    }
}
