use include_dir::{Dir, include_dir};
use log::info;
use std::error::Error as StdError;
use std::fmt;
use std::sync::OnceLock;
use tokio_postgres::Client as PostgresClient;

use crate::db::DbError;

static PROJECT_MIGRATIONS_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/migrations");
static MAIN_MIGRATIONS_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/migrations-main");

static AVAILABLE_PROJECT_MIGRATIONS: OnceLock<Vec<Migration>> = OnceLock::new();
static AVAILABLE_MAIN_MIGRATIONS: OnceLock<Vec<Migration>> = OnceLock::new();

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct Migration {
    number: u32,
    name: &'static str,
    sql: &'static str,
}

#[derive(Debug)]
pub enum MigrationError {
    NoMigrationsFound {
        target: &'static str,
    },
    MigrationFilenameMissingNumericPrefix {
        filename: String,
    },
    ParseMigrationNumber {
        filename: String,
        source: std::num::ParseIntError,
    },
    MigrationNotUtf8 {
        filename: String,
    },
    EnsureMigrationsTable(rusqlite::Error),
    QueryLastApplied(rusqlite::Error),
    ReadLastAppliedValue(rusqlite::Error),
    ApplySql {
        filename: String,
        source: rusqlite::Error,
    },
    RecordApplied {
        filename: String,
        source: rusqlite::Error,
    },
}

impl fmt::Display for MigrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoMigrationsFound { target } => {
                write!(f, "no embedded {target} migrations found")
            }
            Self::MigrationFilenameMissingNumericPrefix { filename } => {
                write!(
                    f,
                    "migration filename must start with a numeric prefix: {filename}"
                )
            }
            Self::ParseMigrationNumber { filename, source } => {
                write!(
                    f,
                    "failed to parse migration number from {filename}: {source}"
                )
            }
            Self::MigrationNotUtf8 { filename } => {
                write!(f, "migration {filename} is not valid UTF-8")
            }
            Self::EnsureMigrationsTable(error) => {
                write!(f, "failed to ensure migrations table: {error}")
            }
            Self::QueryLastApplied(error) => {
                write!(f, "failed to query last applied migration: {error}")
            }
            Self::ReadLastAppliedValue(error) => {
                write!(f, "failed to read last applied migration value: {error}")
            }
            Self::ApplySql { filename, source } => {
                write!(f, "failed to apply migration {filename}: {source}")
            }
            Self::RecordApplied { filename, source } => {
                write!(f, "failed to record applied migration {filename}: {source}")
            }
        }
    }
}

impl StdError for MigrationError {}

/// Discover and cache embedded project/main migrations. Fails if either set is empty.
pub fn initialize_available_migrations() -> Result<(), MigrationError> {
    let _ = project_migrations()?;
    let _ = main_migrations()?;
    Ok(())
}

fn project_migrations() -> Result<&'static [Migration], MigrationError> {
    initialize_migrations(
        &AVAILABLE_PROJECT_MIGRATIONS,
        &PROJECT_MIGRATIONS_DIR,
        "project",
    )
}

fn main_migrations() -> Result<&'static [Migration], MigrationError> {
    initialize_migrations(&AVAILABLE_MAIN_MIGRATIONS, &MAIN_MIGRATIONS_DIR, "main")
}

fn initialize_migrations(
    cache: &'static OnceLock<Vec<Migration>>,
    dir: &'static Dir<'static>,
    target: &'static str,
) -> Result<&'static [Migration], MigrationError> {
    if let Some(migrations) = cache.get() {
        return Ok(migrations.as_slice());
    }
    let scanned = scan_embedded_migrations(dir, target)?;
    Ok(cache.get_or_init(|| scanned).as_slice())
}

pub fn apply_missing_project_migrations(conn: &rusqlite::Connection) -> Result<(), MigrationError> {
    apply_missing_migrations_with_list(conn, "project", project_migrations()?)
}

pub async fn apply_missing_main_postgres_migrations(
    client: &PostgresClient,
) -> Result<(), DbError> {
    info!("Ensuring migrations tracking table exists");
    client
        .batch_execute(
            "CREATE TABLE IF NOT EXISTS migrations (
                migration_number INTEGER PRIMARY KEY,
                applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
        )
        .await
        .map_err(DbError::main_postgres_migrations)?;

    let available = main_migrations()
        .map_err(|error| DbError::MainPostgresMigrationsNotReady(error.to_string()))?;
    let Some(start_index) = first_pending_migration_index(
        available,
        query_last_applied_main_postgres_migration(client).await?,
    ) else {
        info!("Main database schema is already up to date");
        return Ok(());
    };
    info!(
        "Applying {} pending main database migration(s)",
        available.len() - start_index
    );

    for migration in &available[start_index..] {
        info!(
            "Applying main migration {} ({})",
            migration.number, migration.name
        );
        client
            .batch_execute(migration.sql)
            .await
            .map_err(DbError::main_postgres_migrations)?;
        client
            .execute(
                "INSERT INTO migrations (migration_number) VALUES ($1)",
                &[&(migration.number as i32)],
            )
            .await
            .map_err(DbError::main_postgres_migrations)?;
    }
    info!("Finished applying main database migrations");

    Ok(())
}

async fn query_last_applied_main_postgres_migration(
    client: &PostgresClient,
) -> Result<Option<u32>, DbError> {
    let last = client
        .query_opt(
            "SELECT migration_number FROM migrations ORDER BY migration_number DESC LIMIT 1",
            &[],
        )
        .await
        .map_err(DbError::main_postgres_migrations)?
        .and_then(|row| u32::try_from(row.get::<usize, i32>(0)).ok());
    Ok(last)
}

fn apply_missing_migrations_with_list(
    conn: &rusqlite::Connection,
    db_name: &str,
    available: &[Migration],
) -> Result<(), MigrationError> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS migrations (
            migration_number INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        )",
        [],
    )
    .map_err(MigrationError::EnsureMigrationsTable)?;

    let last_applied = query_last_applied_migration(conn)?;
    let Some(start_index) = first_pending_migration_index(available, last_applied) else {
        return Ok(());
    };

    let pending_count = available.len() - start_index;
    let starting_from = available[start_index].number;
    info!("applying {pending_count} migrations to {db_name} starting from {starting_from}");

    for migration in &available[start_index..] {
        apply_one_migration(conn, migration)?;
    }

    Ok(())
}

/// `00001` sets `PRAGMA journal_mode`, which SQLite rejects inside a transaction.
const UNTRANSACTIONAL_MIGRATION: u32 = 1;

fn apply_one_migration(
    conn: &rusqlite::Connection,
    migration: &Migration,
) -> Result<(), MigrationError> {
    if migration.number == UNTRANSACTIONAL_MIGRATION {
        return apply_and_record_migration(conn, migration);
    }

    // `Connection::transaction*` needs `&mut self`; callers only have `&Connection`.
    conn.execute("BEGIN IMMEDIATE", [])
        .map_err(|source| MigrationError::ApplySql {
            filename: migration.name.to_string(),
            source,
        })?;
    let result = apply_migration_in_open_transaction(conn, migration).and_then(|()| {
        conn.execute("COMMIT", [])
            .map(|_| ())
            .map_err(|source| MigrationError::ApplySql {
                filename: migration.name.to_string(),
                source,
            })
    });
    if result.is_err() {
        let _ = conn.execute("ROLLBACK", []);
    }
    result
}

fn apply_migration_in_open_transaction(
    conn: &rusqlite::Connection,
    migration: &Migration,
) -> Result<(), MigrationError> {
    // Another process may have applied this file while we waited for BEGIN IMMEDIATE.
    if query_last_applied_migration(conn)?.is_some_and(|last| last >= migration.number) {
        return Ok(());
    }
    apply_and_record_migration(conn, migration)
}

fn apply_and_record_migration(
    conn: &rusqlite::Connection,
    migration: &Migration,
) -> Result<(), MigrationError> {
    if migration.number == 7 {
        drop_column_if_exists(conn, "landings", "useragent_id").map_err(|source| {
            MigrationError::ApplySql {
                filename: migration.name.to_string(),
                source,
            }
        })?;
    }
    conn.execute_batch(migration.sql)
        .map_err(|source| MigrationError::ApplySql {
            filename: migration.name.to_string(),
            source,
        })?;
    conn.execute(
        "INSERT INTO migrations (migration_number) VALUES (?1)",
        [i64::from(migration.number)],
    )
    .map_err(|source| MigrationError::RecordApplied {
        filename: migration.name.to_string(),
        source,
    })?;
    Ok(())
}

fn query_last_applied_migration(
    conn: &rusqlite::Connection,
) -> Result<Option<u32>, MigrationError> {
    let mut stmt = conn
        .prepare("SELECT migration_number FROM migrations ORDER BY migration_number DESC LIMIT 1")
        .map_err(MigrationError::QueryLastApplied)?;

    match stmt.query_row([], |row| row.get::<_, i64>(0)) {
        Ok(migration_number) => Ok(u32::try_from(migration_number).ok()),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(error) => Err(MigrationError::ReadLastAppliedValue(error)),
    }
}

fn first_pending_migration_index(
    available: &[Migration],
    last_applied: Option<u32>,
) -> Option<usize> {
    if available.is_empty() {
        return None;
    }
    let start_index = match last_applied {
        Some(last) => available.partition_point(|migration| migration.number <= last),
        None => 0,
    };
    (start_index < available.len()).then_some(start_index)
}

fn scan_embedded_migrations(
    dir: &'static Dir<'static>,
    target: &'static str,
) -> Result<Vec<Migration>, MigrationError> {
    let mut migrations = Vec::new();
    for file in dir.files() {
        let path = file.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("sql") {
            continue;
        }
        let Some(filename) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let number = parse_migration_number(filename)?;
        let sql = file
            .contents_utf8()
            .ok_or_else(|| MigrationError::MigrationNotUtf8 {
                filename: filename.to_string(),
            })?;
        migrations.push(Migration {
            number,
            name: filename,
            sql,
        });
    }

    if migrations.is_empty() {
        return Err(MigrationError::NoMigrationsFound { target });
    }

    migrations.sort();
    Ok(migrations)
}

/// `ALTER TABLE … DROP COLUMN` fails when the column is already gone. Demo DBs
/// generated from a 00001 that omitted `landings.useragent_id` hit that on 00007.
fn drop_column_if_exists(
    conn: &rusqlite::Connection,
    table: &str,
    column: &str,
) -> Result<(), rusqlite::Error> {
    debug_assert!(
        table
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
            && column
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_'),
        "drop_column_if_exists is only for hardcoded identifiers"
    );
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = stmt.query([])?;
    let mut present = false;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            present = true;
            break;
        }
    }
    drop(rows);
    drop(stmt);
    if present {
        conn.execute(&format!("ALTER TABLE {table} DROP COLUMN {column}"), [])?;
    }
    Ok(())
}

fn parse_migration_number(filename: &str) -> Result<u32, MigrationError> {
    let prefix_len = filename
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .count();

    if prefix_len == 0 {
        return Err(MigrationError::MigrationFilenameMissingNumericPrefix {
            filename: filename.to_string(),
        });
    }

    let prefix = &filename[..prefix_len];
    prefix
        .parse::<u32>()
        .map_err(|source| MigrationError::ParseMigrationNumber {
            filename: filename.to_string(),
            source,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_embedded_project_and_main_migrations() {
        let project = project_migrations().expect("project migrations");
        assert!(
            project
                .iter()
                .any(|m| m.number == 1 && m.name == "00001_initial.sql"),
            "expected 00001_initial.sql in project migrations, got {project:?}"
        );
        assert!(!project[0].sql.is_empty());

        let main = main_migrations().expect("main migrations");
        assert!(
            main.iter()
                .any(|m| m.number == 1 && m.name == "00001_initial.sql"),
            "expected 00001_initial.sql in main migrations, got {main:?}"
        );
        assert!(
            main.iter()
                .any(|m| m.number == 2 && m.name == "00002_project_settings.sql"),
            "expected 00002_project_settings.sql in main migrations, got {main:?}"
        );
        assert!(!main[0].sql.is_empty());
    }

    #[test]
    fn applies_only_missing_migrations_based_on_last_applied() {
        let available = [
            Migration {
                number: 1,
                name: "001_create_users.sql",
                sql: "CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT NOT NULL);",
            },
            Migration {
                number: 2,
                name: "002_add_name.sql",
                sql: "ALTER TABLE users ADD COLUMN name TEXT;",
            },
        ];

        let conn = rusqlite::Connection::open_in_memory().expect("open in-memory connection");
        conn.execute_batch(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT NOT NULL);
            CREATE TABLE migrations (
                migration_number INTEGER PRIMARY KEY,
                applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );
            INSERT INTO migrations (migration_number) VALUES (1);",
        )
        .expect("seed migrations table");

        apply_missing_migrations_with_list(&conn, "test", &available)
            .expect("apply missing migrations");

        let mut stmt = conn
            .prepare("PRAGMA table_info(users)")
            .expect("table info query");
        let mut column_names = Vec::new();
        let mut rows = stmt.query([]).expect("query table info");
        while let Some(row) = rows.next().expect("table info row") {
            let name: String = row.get(1).expect("column name");
            column_names.push(name);
        }
        assert!(column_names.iter().any(|name| name == "name"));

        let mut stmt = conn
            .prepare("SELECT migration_number FROM migrations ORDER BY migration_number")
            .expect("migrations query");
        let mut rows = stmt.query([]).expect("query migrations");
        let mut applied = Vec::new();
        while let Some(row) = rows.next().expect("migration row") {
            applied.push(row.get::<_, i64>(0).expect("migration number"));
        }
        assert_eq!(applied, vec![1, 2]);
    }

    #[test]
    fn first_pending_index_skips_already_applied() {
        let available = [
            Migration {
                number: 1,
                name: "001.sql",
                sql: "",
            },
            Migration {
                number: 2,
                name: "002.sql",
                sql: "",
            },
        ];
        assert_eq!(first_pending_migration_index(&available, None), Some(0));
        assert_eq!(first_pending_migration_index(&available, Some(1)), Some(1));
        assert_eq!(first_pending_migration_index(&available, Some(2)), None);
    }

    fn table_columns(conn: &rusqlite::Connection, table: &str) -> Vec<String> {
        let mut stmt = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .expect("table info");
        let mut rows = stmt.query([]).expect("query");
        let mut names = Vec::new();
        while let Some(row) = rows.next().expect("row") {
            names.push(row.get::<_, String>(1).expect("name"));
        }
        names
    }

    #[test]
    fn crawler_migration_drops_landings_useragent_id_when_present() {
        let conn = rusqlite::Connection::open_in_memory().expect("open");
        apply_missing_project_migrations(&conn).expect("apply");
        assert!(
            !table_columns(&conn, "landings")
                .iter()
                .any(|name| name == "useragent_id")
        );
        let table_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'crawler_views_by_hour'",
                [],
                |row| row.get(0),
            )
            .expect("table exists");
        assert_eq!(table_exists, 1);
        let crawler_paths: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'crawler_paths'",
                [],
                |row| row.get(0),
            )
            .expect("crawler_paths exists");
        assert_eq!(crawler_paths, 1);
    }

    #[test]
    fn crawler_migration_applies_when_landings_never_had_useragent_id() {
        let conn = rusqlite::Connection::open_in_memory().expect("open");
        let available = project_migrations().expect("project migrations");
        let before_crawlers: Vec<Migration> = available
            .iter()
            .filter(|migration| migration.number < 7)
            .cloned()
            .collect();
        apply_missing_migrations_with_list(&conn, "test", &before_crawlers)
            .expect("apply 00001-00006");
        conn.execute("ALTER TABLE landings DROP COLUMN useragent_id", [])
            .expect("simulate 00001 that omitted the column");
        apply_missing_project_migrations(&conn).expect("apply 00007+");
        assert!(
            !table_columns(&conn, "landings")
                .iter()
                .any(|name| name == "useragent_id")
        );
        let operators: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('useragents') WHERE name = 'operator'",
                [],
                |row| row.get(0),
            )
            .expect("operator column");
        assert_eq!(operators, 1);
    }

    #[test]
    fn crawler_paths_migration_rewrites_views_off_paths() {
        let conn = rusqlite::Connection::open_in_memory().expect("open");
        let available = project_migrations().expect("project migrations");
        let through_crawler_views: Vec<Migration> = available
            .iter()
            .filter(|migration| migration.number <= 7)
            .cloned()
            .collect();
        apply_missing_migrations_with_list(&conn, "test", &through_crawler_views)
            .expect("apply 00001-00007");
        conn.execute(
            "INSERT INTO paths (path, hostname) VALUES ('/blog', 'example.com')",
            [],
        )
        .expect("path");
        conn.execute(
            "INSERT INTO useragents (useragent) VALUES ('gptbot/1.4')",
            [],
        )
        .expect("useragent");
        conn.execute(
            "INSERT INTO crawler_views_by_hour (
                hour, source, useragent_id, path_id, estimated_count
             ) VALUES (3600, 1, 1, 1, 9)",
            [],
        )
        .expect("view");
        apply_missing_project_migrations(&conn).expect("apply 00008+");
        let (path, hostname, count): (String, String, i64) = conn
            .query_row(
                "SELECT p.path, p.hostname, cv.estimated_count
                 FROM crawler_views_by_hour cv
                 JOIN crawler_paths p ON p.id = cv.path_id",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("rewritten view");
        assert_eq!(path, "/blog");
        assert_eq!(hostname, "example.com");
        assert_eq!(count, 9);
        let human_paths: i64 = conn
            .query_row("SELECT COUNT(*) FROM paths", [], |row| row.get(0))
            .expect("paths remain");
        assert_eq!(human_paths, 1);
        let fk: String = conn
            .query_row(
                "SELECT \"table\" FROM pragma_foreign_key_list('crawler_views_by_hour')
                 WHERE \"from\" = 'path_id'",
                [],
                |row| row.get(0),
            )
            .expect("fk");
        assert_eq!(fk, "crawler_paths");
    }

    #[test]
    fn useragents_ip_migration_replaces_unique_useragent() {
        let conn = rusqlite::Connection::open_in_memory().expect("open");
        let available = project_migrations().expect("project migrations");
        let through_verified: Vec<Migration> = available
            .iter()
            .filter(|migration| migration.number <= 9)
            .cloned()
            .collect();
        apply_missing_migrations_with_list(&conn, "test", &through_verified)
            .expect("apply 00001-00009");
        conn.execute(
            "INSERT INTO useragents (useragent) VALUES ('gptbot/1.4')",
            [],
        )
        .expect("useragent");
        conn.execute(
            "INSERT INTO crawler_paths (path, hostname) VALUES ('/robots.txt', 'example.com')",
            [],
        )
        .expect("path");
        conn.execute(
            "INSERT INTO crawler_views_by_hour (
                hour, source, useragent_id, path_id, estimated_count, verified
             ) VALUES (3600, 0, 1, 1, 3, 0)",
            [],
        )
        .expect("view");
        apply_missing_project_migrations(&conn).expect("apply 00010+");
        assert!(
            table_columns(&conn, "useragents")
                .iter()
                .any(|name| name == "ip")
        );
        let count: i64 = conn
            .query_row(
                "SELECT estimated_count FROM crawler_views_by_hour WHERE useragent_id = 1",
                [],
                |row| row.get(0),
            )
            .expect("view survived");
        assert_eq!(count, 3);
        conn.execute(
            "INSERT INTO useragents (useragent, ip) VALUES ('gptbot/1.4', '203.0.113.10')",
            [],
        )
        .expect("same ua with ip");
        let duplicate = conn.execute(
            "INSERT INTO useragents (useragent, ip) VALUES ('gptbot/1.4', '203.0.113.10')",
            [],
        );
        assert!(duplicate.is_err());
        let rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM useragents WHERE useragent = 'gptbot/1.4'",
                [],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(rows, 2);
        let fk: String = conn
            .query_row(
                "SELECT \"table\" FROM pragma_foreign_key_list('crawler_views_by_hour')
                 WHERE \"from\" = 'useragent_id'",
                [],
                |row| row.get(0),
            )
            .expect("fk");
        assert_eq!(fk, "useragents");
    }

    #[test]
    fn failed_migration_rolls_back_sql_and_is_not_recorded() {
        let available = [
            Migration {
                number: 1,
                name: "001_create_users.sql",
                sql: "CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT NOT NULL);",
            },
            Migration {
                number: 2,
                name: "002_partial.sql",
                sql: "ALTER TABLE users ADD COLUMN name TEXT;
                      INSERT INTO table_that_does_not_exist (id) VALUES (1);",
            },
        ];
        let conn = rusqlite::Connection::open_in_memory().expect("open");
        apply_missing_migrations_with_list(&conn, "test", &available)
            .expect_err("migration 2 should fail");
        assert!(
            !table_columns(&conn, "users")
                .iter()
                .any(|name| name == "name"),
            "ADD COLUMN from the failed migration must roll back"
        );
        let applied: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM migrations WHERE migration_number = 2",
                [],
                |row| row.get(0),
            )
            .expect("count migration 2");
        assert_eq!(applied, 0);
        let last: i64 = conn
            .query_row("SELECT MAX(migration_number) FROM migrations", [], |row| {
                row.get(0)
            })
            .expect("last applied");
        assert_eq!(last, 1);
    }
}
