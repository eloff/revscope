use include_dir::{Dir, include_dir};
use log::{info, warn};
use std::error::Error as StdError;
use std::fmt;
use std::sync::OnceLock;
use tokio_postgres::Client as PostgresClient;

use crate::db::DbError;

static PROJECT_MIGRATIONS_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/migrations");
static MAIN_MIGRATIONS_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/migrations-main");

static AVAILABLE_PROJECT_MIGRATIONS: OnceLock<Vec<Migration>> = OnceLock::new();
static AVAILABLE_MAIN_MIGRATIONS: OnceLock<Vec<Migration>> = OnceLock::new();

/// Identifies the main migrations in Postgres's per-database advisory lock space.
const MAIN_MIGRATIONS_LOCK_KEY: i64 = 0x7265_7673_636f_7065;

/// Later main migrations must leave transaction control to the runner.
const LAST_MIGRATION_WITH_ITS_OWN_TRANSACTION: u32 = 10;

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
    let available = main_migrations()
        .map_err(|error| DbError::MainPostgresMigrationsNotReady(error.to_string()))?;
    apply_main_migration_list(client, available).await
}

/// Runs one process's migrations at a time, so instances starting together during
/// a deploy can't apply the same one twice. A session lock, because it has to span
/// each migration's own transaction. Postgres releases it once it notices the
/// connection has died, so the wait has no limit of its own and gets logged.
async fn apply_main_migration_list(
    client: &PostgresClient,
    available: &[Migration],
) -> Result<(), DbError> {
    let acquired: bool = client
        .query_one(
            "SELECT pg_try_advisory_lock($1)",
            &[&MAIN_MIGRATIONS_LOCK_KEY],
        )
        .await
        .map_err(DbError::main_postgres_migrations)?
        .get(0);
    if !acquired {
        info!("Waiting for another process to finish applying main database migrations");
        client
            .execute("SELECT pg_advisory_lock($1)", &[&MAIN_MIGRATIONS_LOCK_KEY])
            .await
            .map_err(DbError::main_postgres_migrations)?;
    }
    let result = apply_pending_main_migrations(client, available).await;
    if let Err(error) = client
        .execute(
            "SELECT pg_advisory_unlock($1)",
            &[&MAIN_MIGRATIONS_LOCK_KEY],
        )
        .await
    {
        warn!("unable to release the main migrations lock: {error}");
    }
    result
}

async fn apply_pending_main_migrations(
    client: &PostgresClient,
    available: &[Migration],
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
        apply_main_migration(client, migration).await?;
    }
    info!("Finished applying main database migrations");

    Ok(())
}

/// Records a migration and applies it in one transaction, so a crash can't leave it
/// applied but unrecorded, which would re-run it on the next start.
///
/// Recording first is what covers 00008 to 00010, which have their own `BEGIN` and
/// `COMMIT` and may already be applied, so can't be edited. Their `BEGIN` only draws
/// a warning, and their `COMMIT` commits the record too.
async fn apply_main_migration(
    client: &PostgresClient,
    migration: &Migration,
) -> Result<(), DbError> {
    client
        .batch_execute("BEGIN")
        .await
        .map_err(DbError::main_postgres_migrations)?;
    let number = migration.number as i32;
    if let Err(error) = client
        .execute(
            "INSERT INTO migrations (migration_number) VALUES ($1)",
            &[&number],
        )
        .await
    {
        if let Err(rollback_error) = client.batch_execute("ROLLBACK").await {
            warn!("unable to roll back main migration: {rollback_error}");
        }
        return Err(DbError::main_postgres_migrations(error));
    }
    let outcome = run_recorded_migration(client, migration).await;
    if outcome.is_ok() {
        return client
            .batch_execute("COMMIT")
            .await
            .map_err(DbError::main_postgres_migrations);
    }
    if let Err(rollback_error) = client.batch_execute("ROLLBACK").await {
        warn!("unable to roll back main migration: {rollback_error}");
    }
    // A migration that committed before failing committed its record too. Removing
    // it keeps the next start from skipping a migration that didn't finish, unless
    // the connection is gone too, which only a test against Postgres can rule out.
    if let Err(error) = client
        .execute(
            "DELETE FROM migrations WHERE migration_number = $1",
            &[&number],
        )
        .await
    {
        warn!("unable to clear the record of a failed main migration: {error}");
    }
    outcome
}

async fn run_recorded_migration(
    client: &PostgresClient,
    migration: &Migration,
) -> Result<(), DbError> {
    let transaction_id = current_transaction_id(client).await?;
    client
        .batch_execute(migration.sql)
        .await
        .map_err(DbError::main_postgres_migrations)?;
    if migration.number > LAST_MIGRATION_WITH_ITS_OWN_TRANSACTION
        && current_transaction_id(client).await? != transaction_id
    {
        return Err(DbError::MainPostgresMigrationsNotReady(format!(
            "{} ended the transaction it runs in; migrations must leave that to the runner, \
             and anything it committed needs undoing by hand",
            migration.name
        )));
    }
    Ok(())
}

/// The open transaction's id, or `None` if it hasn't been assigned one by a write.
async fn current_transaction_id(client: &PostgresClient) -> Result<Option<String>, DbError> {
    client
        .query_one("SELECT pg_current_xact_id_if_assigned()::text", &[])
        .await
        .map(|row| row.get(0))
        .map_err(DbError::main_postgres_migrations)
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

    /// A connection whose unqualified tables and types live in a schema of its own,
    /// which stands in for a new database.
    async fn scratch_connection(schema: &str) -> PostgresClient {
        crate::test_env::load_e2e_env();
        let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        let (client, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
            .await
            .expect("connect to postgres");
        tokio::spawn(connection);
        client
            .batch_execute(&format!(
                "CREATE SCHEMA IF NOT EXISTS {schema}; SET search_path TO {schema}"
            ))
            .await
            .expect("create scratch schema");
        client
    }

    async fn drop_schema(client: &PostgresClient, schema: &str) {
        client
            .batch_execute(&format!("DROP SCHEMA {schema} CASCADE"))
            .await
            .expect("drop scratch schema");
    }

    /// A schema name no other run of this test is using.
    fn scratch_schema(label: &str) -> String {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock is after the unix epoch")
            .as_nanos();
        format!("migration_test_{label}_{nanos:x}")
    }

    fn migration(number: u32, sql: &'static str) -> Migration {
        Migration {
            number,
            name: "test.sql",
            sql,
        }
    }

    async fn table_exists(client: &PostgresClient, name: &str) -> bool {
        client
            .query_one("SELECT to_regclass($1)::text IS NOT NULL", &[&name])
            .await
            .expect("look up table")
            .get(0)
    }

    async fn recorded(client: &PostgresClient) -> Vec<i32> {
        client
            .query(
                "SELECT migration_number FROM migrations ORDER BY migration_number",
                &[],
            )
            .await
            .expect("list recorded migrations")
            .iter()
            .map(|row| row.get(0))
            .collect()
    }

    /// Runs `sql` as a migration that kills its own connection, as a crash would,
    /// and returns a new connection to look at what's left.
    async fn crash_during(schema: &str, sql: &'static str) -> PostgresClient {
        let client = scratch_connection(schema).await;
        assert!(
            apply_main_migration_list(&client, &[migration(1, sql)])
                .await
                .is_err()
        );
        scratch_connection(schema).await
    }

    #[tokio::test]
    async fn a_crash_during_a_migration_leaves_nothing_behind() {
        let schema = scratch_schema("crash");
        let client = crash_during(
            &schema,
            "CREATE TABLE half_done (id INTEGER);
             SELECT pg_terminate_backend(pg_backend_pid());",
        )
        .await;

        assert!(!table_exists(&client, "half_done").await);
        assert!(recorded(&client).await.is_empty());
        drop_schema(&client, &schema).await;
    }

    #[tokio::test]
    async fn a_crash_after_a_migrations_own_commit_leaves_it_recorded() {
        let schema = scratch_schema("own_commit");
        let client = crash_during(
            &schema,
            "BEGIN;
             CREATE TABLE committed_by_itself (id INTEGER);
             COMMIT;
             SELECT pg_terminate_backend(pg_backend_pid());",
        )
        .await;

        assert!(table_exists(&client, "committed_by_itself").await);
        assert_eq!(recorded(&client).await, [1]);
        drop_schema(&client, &schema).await;
    }

    #[tokio::test]
    async fn a_new_migration_that_ends_the_transaction_keeps_being_refused() {
        for (label, sql) in [
            (
                "commit",
                "CREATE TABLE IF NOT EXISTS ended (id INTEGER); COMMIT",
            ),
            (
                "end",
                "START TRANSACTION; CREATE TABLE IF NOT EXISTS ended (id INTEGER); END",
            ),
            ("rollback", "CREATE TABLE ended (id INTEGER); ROLLBACK"),
            (
                "rollback_reopen",
                "ROLLBACK; BEGIN; CREATE TABLE ended (id INTEGER)",
            ),
            (
                "commit_reopen",
                "COMMIT; BEGIN; CREATE TABLE IF NOT EXISTS ended (id INTEGER)",
            ),
            (
                "commit_chain",
                "COMMIT AND CHAIN; CREATE TABLE IF NOT EXISTS ended (id INTEGER)",
            ),
        ] {
            let schema = scratch_schema(label);
            let client = scratch_connection(&schema).await;
            let number = LAST_MIGRATION_WITH_ITS_OWN_TRANSACTION + 1;
            for start in ["first", "second"] {
                let error = apply_main_migration_list(&client, &[migration(number, sql)])
                    .await
                    .expect_err(label);
                assert!(
                    error.to_string().contains("ended the transaction"),
                    "{label}: {start} start: {error}"
                );
                assert!(recorded(&client).await.is_empty(), "{label}: {start} start");
            }
            drop_schema(&client, &schema).await;
        }
    }

    #[tokio::test]
    async fn a_failure_after_a_migrations_own_commit_is_not_recorded() {
        let schema = scratch_schema("commit_fail");
        let client = scratch_connection(&schema).await;
        let number = LAST_MIGRATION_WITH_ITS_OWN_TRANSACTION + 1;
        let list = [migration(number, "COMMIT; SELECT 1 / 0")];
        for _start in ["first", "second"] {
            assert!(apply_main_migration_list(&client, &list).await.is_err());
            assert!(recorded(&client).await.is_empty());
        }
        drop_schema(&client, &schema).await;
    }

    #[tokio::test]
    async fn a_failed_start_keeps_a_record_it_did_not_write() {
        let schema = scratch_schema("other_record");
        let client = scratch_connection(&schema).await;
        apply_main_migration_list(&client, &[migration(1, "SELECT 1")])
            .await
            .expect("apply first");

        // Already recorded, as by a process that doesn't take the lock.
        assert!(
            apply_main_migration(&client, &migration(1, "SELECT 1"))
                .await
                .is_err()
        );

        assert_eq!(recorded(&client).await, [1]);
        drop_schema(&client, &schema).await;
    }

    #[tokio::test]
    async fn a_failed_migration_keeps_earlier_ones_and_is_retried() {
        let schema = scratch_schema("retry");
        let client = scratch_connection(&schema).await;

        let failing = [
            migration(1, "CREATE TABLE first_table (id INTEGER)"),
            migration(2, "CREATE TABLE second_table (id INTEGER); SELECT 1 / 0"),
        ];
        assert!(apply_main_migration_list(&client, &failing).await.is_err());
        assert_eq!(recorded(&client).await, [1]);
        assert!(!table_exists(&client, "second_table").await);

        let fixed = [
            migration(1, "CREATE TABLE first_table (id INTEGER)"),
            migration(2, "CREATE TABLE second_table (id INTEGER)"),
        ];
        apply_main_migration_list(&client, &fixed)
            .await
            .expect("retry after the fix");
        assert_eq!(recorded(&client).await, [1, 2]);
        drop_schema(&client, &schema).await;
    }

    #[tokio::test]
    async fn processes_starting_together_each_apply_a_migration_once() {
        let schema = scratch_schema("concurrent");
        let first = scratch_connection(&schema).await;
        let second = scratch_connection(&schema).await;
        let list = [migration(
            1,
            "SELECT pg_sleep(0.1); CREATE TABLE only_once (id INTEGER)",
        )];

        let (first_result, second_result) = tokio::join!(
            apply_main_migration_list(&first, &list),
            apply_main_migration_list(&second, &list),
        );

        first_result.expect("first process");
        second_result.expect("second process");
        assert_eq!(recorded(&first).await, [1]);
        drop_schema(&first, &schema).await;
    }

    #[tokio::test]
    async fn every_main_migration_applies_to_a_new_database() {
        let schema = scratch_schema("all");
        let client = scratch_connection(&schema).await;
        let available = main_migrations().expect("main migrations");

        apply_main_migration_list(&client, available)
            .await
            .expect("apply every migration");
        apply_main_migration_list(&client, available)
            .await
            .expect("nothing left to apply");

        assert_eq!(recorded(&client).await.len(), available.len());
        drop_schema(&client, &schema).await;
    }

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
        let through_ip: Vec<Migration> = available
            .iter()
            .filter(|migration| migration.number <= 10)
            .cloned()
            .collect();
        apply_missing_migrations_with_list(&conn, "test", &through_ip).expect("apply 00010");
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
    fn erasing_useragent_ips_merges_rows_and_clears_them_from_disk_at_the_sweep() {
        let directory = crate::test_utils::TestDataDir::new("erase-useragent-ips");
        let path = directory.path().join("1.db");
        let conn = rusqlite::Connection::open(&path).expect("open");
        let available = project_migrations().expect("project migrations");
        let before: Vec<Migration> = available
            .iter()
            .filter(|migration| migration.number < 14)
            .cloned()
            .collect();
        apply_missing_migrations_with_list(&conn, "test", &before).expect("apply up to 00013");
        conn.execute_batch(
            "INSERT INTO crawler_paths (id, path, hostname) VALUES (1, '/', 'example.com');
             INSERT INTO useragents (id, useragent, operator, category, ip) VALUES
                (1, 'mystery', NULL, NULL, '203.0.113.77'),
                (2, 'mystery', NULL, NULL, '198.51.100.88'),
                (3, 'mystery', 'openai', 'ai_crawler', NULL),
                (4, 'other', NULL, NULL, '192.0.2.99');
             INSERT INTO crawler_views_by_hour
                (hour, source, useragent_id, path_id, estimated_count, verified) VALUES
                (3600, 0, 1, 1, 3, 0),
                (3600, 0, 2, 1, 4, 0),
                (3600, 1, 3, 1, 5, 1),
                (7200, 0, 2, 1, 6, 0),
                (3600, 0, 4, 1, 7, 0);",
        )
        .expect("seed");

        // `conn` stays open through the sweep like a cached connection would, so
        // only the sweep's own checkpoint can clear the WAL.
        crate::project_db_sweep::sweep_project_databases(directory.path(), || false);

        let stored_ips: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM useragents WHERE ip IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .expect("count ips");
        assert_eq!(stored_ips, 0);
        let merged: (i64, Option<String>) = conn
            .query_row(
                "SELECT COUNT(*), MAX(operator) FROM useragents WHERE useragent = 'mystery'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("merged");
        assert_eq!(merged, (1, Some("openai".to_string())));
        let mut statement = conn
            .prepare(
                "SELECT ua.useragent, cv.hour, cv.source, cv.verified, cv.estimated_count
                 FROM crawler_views_by_hour cv JOIN useragents ua ON ua.id = cv.useragent_id
                 ORDER BY 1, 2, 3",
            )
            .expect("prepare");
        let views: Vec<(String, i64, i64, i64, i64)> = statement
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("rows");
        assert_eq!(
            views,
            [
                ("mystery".to_string(), 3600, 0, 0, 7),
                ("mystery".to_string(), 3600, 1, 1, 5),
                ("mystery".to_string(), 7200, 0, 0, 6),
                ("other".to_string(), 3600, 0, 0, 7),
            ]
        );

        assert_addresses_not_on_disk(&path, &["203.0.113.77", "198.51.100.88", "192.0.2.99"]);

        // The previous binary still drains after the migration, and must keep
        // working, but what it writes has to go at the next sweep.
        conn.execute(
            "INSERT INTO useragents (useragent, operator, category, ip)
             VALUES ('mystery', NULL, NULL, '203.0.113.200')
             ON CONFLICT(useragent, COALESCE(ip, '')) DO UPDATE SET
                operator = COALESCE(useragents.operator, excluded.operator)",
            [],
        )
        .expect("old binary's upsert");
        let late_row = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO crawler_views_by_hour
                (hour, source, useragent_id, path_id, estimated_count, verified)
             VALUES (3600, 0, ?1, 1, 2, 0)",
            [late_row],
        )
        .expect("old binary's view");
        drop(statement);

        crate::project_db_sweep::sweep_project_databases(directory.path(), || false);

        let mystery: (i64, i64) = conn
            .query_row(
                "SELECT COUNT(DISTINCT ua.id), SUM(cv.estimated_count)
                 FROM crawler_views_by_hour cv JOIN useragents ua ON ua.id = cv.useragent_id
                 WHERE ua.useragent = 'mystery' AND cv.hour = 3600 AND cv.source = 0",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("merged late view");
        assert_eq!(mystery, (1, 9));
        assert_addresses_not_on_disk(&path, &["203.0.113.200"]);
    }

    fn assert_addresses_not_on_disk(database: &std::path::Path, addresses: &[&str]) {
        for file in [database.to_path_buf(), database.with_extension("db-wal")] {
            let bytes = std::fs::read(&file).unwrap_or_default();
            for address in addresses {
                assert!(
                    !bytes
                        .windows(address.len())
                        .any(|window| window == address.as_bytes()),
                    "{address} is still in {}",
                    file.display()
                );
            }
        }
    }

    #[test]
    fn sweep_stops_when_asked() {
        let directory = crate::test_utils::TestDataDir::new("sweep-stops");
        let conn = rusqlite::Connection::open(directory.path().join("1.db")).expect("open");
        let available = project_migrations().expect("project migrations");
        let before: Vec<Migration> = available
            .iter()
            .filter(|migration| migration.number < 14)
            .cloned()
            .collect();
        apply_missing_migrations_with_list(&conn, "test", &before).expect("apply up to 00013");

        crate::project_db_sweep::sweep_project_databases(directory.path(), || true);

        let last: i64 = conn
            .query_row("SELECT MAX(migration_number) FROM migrations", [], |row| {
                row.get(0)
            })
            .expect("last migration");
        assert_eq!(last, 13);
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
