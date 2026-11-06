//! Dump the project SQLite schema (tables and indices) as a single SQL file.

use std::sync::Arc;

use crate::auth::AuthError;
use crate::authorize::AuthorizedProject;
use crate::db::{
    ProjectConnection, open_project_database_read_for, parse_project_id, require_project,
};
use crate::download_db::DEMO_PROJECT_ID;
use crate::server_context::ServerContext;

pub(crate) fn schema(
    server_context: Arc<ServerContext>,
    _project: &AuthorizedProject,
) -> Result<String, AuthError> {
    cached_schema(server_context).map(|sql| sql.to_string())
}

/// The same dump the schema endpoint serves, memoized for the process lifetime.
/// Always sourced from the demo DB so project-specific tables never leak into
/// the cache. The chat agent puts it in the prompt of every step of every turn.
pub(crate) fn cached_schema(server_context: Arc<ServerContext>) -> Result<Arc<str>, AuthError> {
    if let Some(dump) = server_context.schema_cache.get() {
        return Ok(dump.clone());
    }

    let demo_key =
        parse_project_id(DEMO_PROJECT_ID).map_err(|error| AuthError::Db(error.to_string()))?;
    let demo = require_project(server_context.clone(), demo_key)
        .map_err(|error| AuthError::Db(error.to_string()))?;
    let conn = open_project_database_read_for(server_context.as_ref(), &demo)
        .map_err(|error| AuthError::Db(error.to_string()))?;
    let dumped: Arc<str> = dump_schema(&conn)
        .map_err(|error| AuthError::Query(error.to_string()))?
        .into();
    // Concurrent first-time callers may both dump; the loser of the race keeps
    // whoever won. Failures leave the cell empty so the next call can retry.
    let _ = server_context.schema_cache.set(dumped);
    Ok(server_context
        .schema_cache
        .get()
        .expect("schema_cache set above or by a racer")
        .clone())
}

/// Brief agent-oriented docs for the companion query endpoint.
const QUERY_API_COMMENT: &str = "\
-- Query API (POST /v2/query/projects/{id}/sql) returns JSON array of rows which are arrays of selected column values.
-- JSON body: {\"sql\":\"...\",\"limit\":100,\"offset\":0}
-- Accepts one read-only SELECT, WITH, or EXPLAIN sqlite query.
-- Limits: SQL <= 128 KiB; results <= 500 rows; offset <= 5000 (via request json fields).
-- BLOBs are returned as base64. limit/offset are optional (default limit 100).
-- WARNING: Always use ORDER BY so pagination is stable. Paginate with the
-- endpoint's limit/offset json fields — do not put LIMIT/OFFSET in the SQL.
";

/// Concatenate CREATE statements for all tables and indices with non-null SQL.
fn dump_schema(conn: &ProjectConnection) -> Result<String, rusqlite::Error> {
    let mut statement = conn.prepare(
        "SELECT sql FROM sqlite_master
         WHERE type IN ('table', 'index')
           AND sql IS NOT NULL
         ORDER BY CASE type WHEN 'table' THEN 0 ELSE 1 END, name",
    )?;
    let mut rows = statement.query([])?;
    let mut statements = String::new();
    while let Some(row) = rows.next()? {
        let sql: String = row.get(0)?;
        if !statements.is_empty() {
            statements.push_str(";\n\n");
        }
        statements.push_str(sql.trim());
    }
    if !statements.is_empty() {
        statements.push_str(";\n");
    }
    Ok(format!("{QUERY_API_COMMENT}\n{statements}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::project::in_memory_test_connection;

    #[test]
    fn dump_schema_includes_tables_and_named_indices() {
        let conn = in_memory_test_connection();
        conn.execute(
            "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
            [],
        )
        .expect("create table");
        conn.execute("CREATE INDEX t_name ON t (name)", [])
            .expect("create index");

        let sql = dump_schema(&conn).expect("dump");
        assert!(sql.starts_with(QUERY_API_COMMENT));
        assert!(sql.contains("CREATE TABLE t"));
        assert!(sql.contains("CREATE INDEX t_name"));
        // Autoindexes for PRIMARY KEY have null sql and must be omitted.
        assert!(!sql.to_lowercase().contains("autoindex"));
        assert!(sql.ends_with(";\n"));
    }

    #[test]
    fn dump_schema_orders_tables_before_indices() {
        let conn = in_memory_test_connection();
        conn.execute("CREATE TABLE z (id INTEGER)", [])
            .expect("create z");
        conn.execute("CREATE TABLE a (id INTEGER)", [])
            .expect("create a");
        conn.execute("CREATE INDEX z_id ON z (id)", [])
            .expect("create index");

        let sql = dump_schema(&conn).expect("dump");
        let table_a = sql.find("CREATE TABLE a").expect("table a");
        let table_z = sql.find("CREATE TABLE z").expect("table z");
        let index_z = sql.find("CREATE INDEX z_id").expect("index");
        assert!(table_a < table_z);
        assert!(table_z < index_z);
    }
}
