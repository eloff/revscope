use std::error::Error as StdError;
use std::fmt;
use std::ops::Deref;
use std::sync::Arc;
use std::sync::OnceLock;

use crate::db::migrations::MigrationError;
use crate::server_context::ServerContext;

pub mod main;
pub mod migrations;
pub mod project;
pub mod types;

pub const DATA_DIR: &str = "data";

/// Matches `idx_campaigns_attribution_tuple` in `migrations/00001_initial.sql`.
pub const CAMPAIGNS_INSERT_ON_CONFLICT: &str =
    "ON CONFLICT(source, medium, campaign, COALESCE(network, '')) DO NOTHING";

/// Wrapper around [`tokio_postgres::Error`] whose [`Display`] surfaces the real
/// message. The raw postgres error's `Display` is often just `"db error"`.
#[derive(Debug)]
pub struct PostgresError(tokio_postgres::Error);

impl PostgresError {
    pub fn into_inner(self) -> tokio_postgres::Error {
        self.0
    }
}

impl From<tokio_postgres::Error> for PostgresError {
    fn from(error: tokio_postgres::Error) -> Self {
        Self(error)
    }
}

impl Deref for PostgresError {
    type Target = tokio_postgres::Error;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<tokio_postgres::Error> for PostgresError {
    fn as_ref(&self) -> &tokio_postgres::Error {
        &self.0
    }
}

impl fmt::Display for PostgresError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Prefer the source chain; Display on the outer error is usually "db error".
        if let Some(source) = self.0.source() {
            write!(f, "{source}")
        } else {
            write!(f, "{}", self.0)
        }
    }
}

impl StdError for PostgresError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.0.source()
    }
}

#[derive(Debug)]
pub enum DbError {
    InvalidProjectId(String),
    ProjectDatabaseNotFound(String),
    ProjectDatabaseAlreadyExists(String),
    CreateDataDir(std::io::Error),
    RemoveProjectDatabase(std::io::Error),
    Open(rusqlite::Error),
    HardenConnection(rusqlite::Error),
    SetWal(rusqlite::Error),
    EnableForeignKeys(rusqlite::Error),
    SetQueryOnly(rusqlite::Error),
    ApplyMigrations(MigrationError),
    MissingMainPostgresUrlEnv(&'static str),
    MainPostgresConnect(PostgresError),
    MainPostgresMigrations(PostgresError),
    MainPostgresMigrationsNotReady(String),
    MainPostgresQuery(PostgresError),
    ProjectDbCheckoutTimeout,
}

/// Whether SQLite refused because another connection holds a lock, which a
/// later retry can get past.
pub(crate) fn is_sqlite_lock_contention(error: &rusqlite::Error) -> bool {
    matches!(
        error.sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
    )
}

impl DbError {
    /// Whether opening or preparing the connection failed on another
    /// connection's lock (see [`is_sqlite_lock_contention`]).
    pub(crate) fn is_sqlite_lock_contention(&self) -> bool {
        match self {
            Self::Open(error)
            | Self::HardenConnection(error)
            | Self::SetWal(error)
            | Self::EnableForeignKeys(error)
            | Self::SetQueryOnly(error) => is_sqlite_lock_contention(error),
            // Migrations run on the first checkout and are the one step of
            // opening that reads and writes the database.
            Self::ApplyMigrations(
                MigrationError::EnsureMigrationsTable(error)
                | MigrationError::QueryLastApplied(error)
                | MigrationError::ReadLastAppliedValue(error)
                | MigrationError::ApplySql { source: error, .. }
                | MigrationError::RecordApplied { source: error, .. },
            ) => is_sqlite_lock_contention(error),
            _ => false,
        }
    }

    pub(crate) fn main_postgres_connect(error: impl Into<PostgresError>) -> Self {
        Self::MainPostgresConnect(error.into())
    }

    pub(crate) fn main_postgres_migrations(error: impl Into<PostgresError>) -> Self {
        Self::MainPostgresMigrations(error.into())
    }

    pub(crate) fn main_postgres_query(error: impl Into<PostgresError>) -> Self {
        Self::MainPostgresQuery(error.into())
    }
}

impl fmt::Display for DbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidProjectId(project_id) => {
                write!(f, "project id must be a non-negative i32: {project_id}")
            }
            Self::ProjectDatabaseNotFound(project_id) => {
                write!(f, "project database does not exist: {project_id}")
            }
            Self::ProjectDatabaseAlreadyExists(project_id) => {
                write!(f, "project database already exists: {project_id}")
            }
            Self::CreateDataDir(error) => write!(f, "failed to create data directory: {error}"),
            Self::RemoveProjectDatabase(error) => {
                write!(f, "failed to remove project database: {error}")
            }
            Self::Open(error) => write!(f, "failed to open database: {error}"),
            Self::HardenConnection(error) => {
                write!(f, "failed to harden database connection: {error}")
            }
            Self::SetWal(error) => write!(f, "failed to set journal mode to wal: {error}"),
            Self::EnableForeignKeys(error) => {
                write!(f, "failed to enable foreign keys pragma: {error}")
            }
            Self::SetQueryOnly(error) => {
                write!(f, "failed to set query_only pragma: {error}")
            }
            Self::ApplyMigrations(error) => {
                write!(f, "failed to apply database migrations: {error}")
            }
            Self::MissingMainPostgresUrlEnv(env_name) => {
                write!(f, "missing postgres url env var: {env_name}")
            }
            Self::MainPostgresConnect(error) => {
                write!(f, "failed to connect to main postgres database: {error}")
            }
            Self::MainPostgresMigrations(error) => {
                write!(f, "failed to apply main postgres migrations: {error}")
            }
            Self::MainPostgresMigrationsNotReady(message) => {
                write!(f, "main postgres migrations are not ready: {message}")
            }
            Self::MainPostgresQuery(error) => {
                write!(f, "failed to run main postgres query: {error}")
            }
            Self::ProjectDbCheckoutTimeout => {
                write!(f, "timed out waiting for project database checkout")
            }
        }
    }
}

impl StdError for DbError {}

static DEFAULT_SERVER_CONTEXT: OnceLock<Arc<ServerContext>> = OnceLock::new();

pub fn default_server_context() -> &'static Arc<ServerContext> {
    DEFAULT_SERVER_CONTEXT.get_or_init(|| Arc::new(ServerContext::new(DATA_DIR)))
}

pub fn start_db_cache_expiry_task() {
    project::start_db_cache_expiry_task_for(default_server_context().clone());
}

pub use main::{
    apply_main_migrations, open_main_database_read, open_main_database_write,
    try_open_main_database_write,
};
pub use project::{
    Connection as ProjectConnection, ProjectCache, create_project_database, get_project,
    insert_project, open_project_database_read, open_project_database_read_for,
    open_project_database_write, open_project_database_write_for, parse_project_id,
    project_database_path, remove_project_database, require_project,
    try_open_project_database_write,
};
pub(crate) use types::{ClientPageId, SqlCompactStr};
