/// Shared with error-log rate limiting so ingest DB failures match by prefix.
/// Logged details are `{INGEST_PERSIST_INTERNAL_ERROR}: {inner}`.
pub(crate) const INGEST_PERSIST_INTERNAL_ERROR: &str =
    "internal server error while persisting ingest payload";

#[derive(Debug)]
pub(super) enum PersistError {
    InvalidPayload,
    UnknownProject,
    Database(String),
    /// SQLite reported the database or a table locked by another connection.
    /// Nothing is wrong with the data, so a flush keeps its rows and retries
    /// instead of blaming the row it was writing.
    DatabaseBusy(String),
    /// A flush unwound. Kept distinct from [`Self::Database`] so the recorded
    /// `errors` row says the batch was abandoned by a bug rather than rejected
    /// by SQLite.
    Panic(String),
}

impl PersistError {
    pub(super) fn database(error: impl std::fmt::Display) -> Self {
        Self::Database(error.to_string())
    }

    pub(super) fn is_busy(&self) -> bool {
        matches!(self, Self::DatabaseBusy(_))
    }

    /// Value stored in `errors.error_type` when the error is recorded.
    pub(super) fn error_type(&self) -> &'static str {
        match self {
            Self::InvalidPayload => "invalid_payload",
            Self::UnknownProject => "unknown_project",
            Self::Database(_) => "database",
            Self::DatabaseBusy(_) => "database_busy",
            Self::Panic(_) => "panic",
        }
    }
}

impl std::fmt::Display for PersistError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPayload => formatter.write_str("invalid payload"),
            Self::UnknownProject => formatter.write_str("unknown project"),
            Self::Database(message) | Self::DatabaseBusy(message) | Self::Panic(message) => {
                formatter.write_str(message)
            }
        }
    }
}

impl From<rusqlite::Error> for PersistError {
    fn from(error: rusqlite::Error) -> Self {
        if crate::db::is_sqlite_lock_contention(&error) {
            Self::DatabaseBusy(error.to_string())
        } else {
            Self::database(error)
        }
    }
}

impl From<crate::db::DbError> for PersistError {
    fn from(error: crate::db::DbError) -> Self {
        if error.is_sqlite_lock_contention() {
            Self::DatabaseBusy(error.to_string())
        } else {
            Self::database(error)
        }
    }
}

impl From<tokio_postgres::Error> for PersistError {
    fn from(error: tokio_postgres::Error) -> Self {
        Self::Database(crate::db::PostgresError::from(error).to_string())
    }
}

impl From<crate::db::PostgresError> for PersistError {
    fn from(error: crate::db::PostgresError) -> Self {
        Self::Database(error.to_string())
    }
}

impl From<std::num::TryFromIntError> for PersistError {
    fn from(error: std::num::TryFromIntError) -> Self {
        Self::database(error)
    }
}

impl From<serde_json::Error> for PersistError {
    fn from(error: serde_json::Error) -> Self {
        Self::database(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbError;

    #[test]
    fn lock_errors_while_opening_are_classified_as_busy() {
        use crate::db::migrations::MigrationError;
        let sqlite = |code| rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(code), None);
        let migration_busy = DbError::ApplyMigrations(MigrationError::ApplySql {
            filename: "0002.sql".to_string(),
            source: sqlite(rusqlite::ffi::SQLITE_BUSY),
        });
        assert!(PersistError::from(migration_busy).is_busy());
        let open_locked = DbError::Open(sqlite(rusqlite::ffi::SQLITE_LOCKED));
        assert!(PersistError::from(open_locked).is_busy());
        let open_failed = DbError::Open(sqlite(rusqlite::ffi::SQLITE_CANTOPEN));
        assert!(!PersistError::from(open_failed).is_busy());
    }
}
