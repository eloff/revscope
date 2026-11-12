//! Ad-hoc read-only SQL against a project database.
//!
//! The caller supplies one statement; pagination is applied by this module
//! rather than by the statement, so `limit`/`offset` stay enforceable. The
//! statement runs on a read connection (`PRAGMA query_only = 1`), must parse as
//! a single statement, must start with a read-only keyword, and must report
//! itself read-only via `sqlite3_stmt_readonly`.
//!
//! Before prepare, the connection gets tighter `sqlite3_limit` values aimed at
//! prepare-time memory and stack blow-ups that the wall-clock progress handler
//! cannot catch. The connection is closed afterward rather than returned to the
//! pool, so those limits never leak onto dashboard queries.
//!
//! An authorizer limits the statement to reading the project's own tables and
//! views and calling functions that reveal nothing about the server: SQLite's
//! `pragma_*` and `dbstat` tables would otherwise expose the database's path on
//! disk and how SQLite was built. How many statements may run at once, per
//! project, per demo reader and across the server, is set in
//! [`query_slots`](crate::query_slots).
//!
//! Execution also stops early when wall-clock time or approximate buffered
//! response size exceeds a cap, returning whatever rows were collected with
//! `has_more` set so the caller can paginate.

use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use fxhash::FxHashSet;
use parking_lot::Mutex;
use revscope_api::query::{DEFAULT_LIMIT, MAX_LIMIT, MAX_OFFSET};
pub(crate) use revscope_api::query::{QueryRequest, QueryResponse};
use rusqlite::ErrorCode;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::limits::Limit;
use rusqlite::types::ValueRef;
use serde_json::{Number, Value};

use crate::auth::AuthError;
use crate::authorize::{AuthorizedProject, is_demo_project_id};
use crate::db::{ProjectConnection, open_project_database_read};
use crate::query_slots::SlotOwner;
use crate::server_context::ServerContext;

const MAX_SQL_LEN: usize = 128 * 1024;
const QUERY_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
/// Approximate bytes charged for nulls, integers, and floats.
const FIXED_FIELD_BYTES: usize = 8;
/// SQLite VM ops between progress-handler checks for the wall-clock timeout.
const PROGRESS_HANDLER_OPS: std::os::raw::c_int = 1000;

/// Max bytes in any string/blob value (and related row payload) SQLite will allocate.
const SQLITE_LIMIT_LENGTH: i32 = 2_000_000;
/// Align with [`MAX_SQL_LEN`] so app validation wins over SQLite's message.
const SQLITE_LIMIT_SQL_LENGTH: i32 = MAX_SQL_LEN as i32;
const SQLITE_LIMIT_COLUMN: i32 = 128;
const SQLITE_LIMIT_EXPR_DEPTH: i32 = 50;
const SQLITE_LIMIT_COMPOUND_SELECT: i32 = 10;
/// Prepare-time bytecode size cap; wall-clock timeout covers executed ops.
const SQLITE_LIMIT_VDBE_OP: i32 = 2_500_000;

/// Statements that cannot change the database or the connection. Anything else
/// (including `PRAGMA`, which would outlive the request on a pooled
/// connection) is rejected before the statement is prepared.
const ALLOWED_LEADING_KEYWORDS: [&str; 4] = ["SELECT", "WITH", "VALUES", "EXPLAIN"];

/// Built-in tables a statement may read besides the project's own. These two
/// compute only over their arguments; the rest (`pragma_*`, `dbstat`) describe
/// the server. SQLite reports `sqlite_schema` by its old name.
const READABLE_BUILT_IN_TABLES: [&str; 3] = ["sqlite_master", "json_each", "json_tree"];

/// Functions that load code or reveal the SQLite build. Every `sqlite_*`
/// function is denied too, since they report the build and record offsets.
const DENIED_FUNCTIONS: [&str; 3] = ["load_extension", "fts3_tokenizer", "fts5_source_id"];

/// The error for a statement that would change the database or connection.
const READ_ONLY_MESSAGE: &str = "query must be read-only";

#[derive(Debug)]
pub(crate) enum QueryError {
    Auth(AuthError),
    Invalid(String),
    /// The project or server already has as many statements running as it may.
    Busy,
}

impl QueryError {
    /// Wording safe to hand back to whoever wrote the SQL, so a model can read
    /// its own mistake and try again.
    pub(crate) fn client_message(&self) -> String {
        match self {
            Self::Invalid(message) => message.clone(),
            Self::Busy => "too many queries are running; try again in a few seconds".to_string(),
            Self::Auth(_) => "the query could not be run".to_string(),
        }
    }
}

pub(crate) fn query(
    server_context: Arc<ServerContext>,
    project: &AuthorizedProject,
    reader: &str,
    request: QueryRequest,
) -> Result<QueryResponse, QueryError> {
    run_project_query(
        server_context,
        project.id(),
        reader,
        &request.sql,
        request.limit,
        request.offset,
    )
}

/// Validates and runs `sql` against a project database. Callers are responsible
/// for having authorized access to `project_id`; the chat agent does so once per
/// turn rather than once per statement the model writes. `reader` is a stable
/// key for who is asking, such as a user id or client network, and is what the
/// public demo charges slots to.
pub(crate) fn run_project_query(
    server_context: Arc<ServerContext>,
    project_id: &str,
    reader: &str,
    sql: &str,
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<QueryResponse, QueryError> {
    let sql = sql.trim();
    validate_sql(sql)?;
    let limit = limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let offset = offset.unwrap_or(0).min(MAX_OFFSET);
    let _slot = server_context
        .ad_hoc_query_slots
        .try_acquire(slot_owner(project_id, reader))
        .ok_or(QueryError::Busy)?;

    let mut conn = open_project_database_read(server_context.as_ref(), project_id)
        .map_err(|error| QueryError::Auth(AuthError::Db(error.to_string())))?;
    let result = run_query(
        &conn,
        sql,
        limit,
        offset,
        QueryLimits {
            timeout: QUERY_TIMEOUT,
            max_response_bytes: MAX_RESPONSE_BYTES,
        },
    );
    // Always discard: ad-hoc limits must not leak onto pooled dashboard connections.
    conn.close();
    result
}

fn slot_owner(project_id: &str, reader: &str) -> SlotOwner {
    if is_demo_project_id(project_id) {
        SlotOwner::DemoReader(reader.to_string())
    } else {
        SlotOwner::Project(project_id.to_string())
    }
}

#[derive(Clone, Copy)]
struct QueryLimits {
    timeout: Duration,
    max_response_bytes: usize,
}

/// Holds the authorizer, and later the progress handler, for one statement and
/// clears both when dropped, so neither outlives the request on a pooled
/// connection. Created before the statement is prepared and dropped after it,
/// because SQLite reruns the authorizer if it has to prepare the statement again.
struct AdHocHooks<'a> {
    conn: &'a ProjectConnection,
    /// The authorizer's first refusal, as the error message to return.
    denied: Arc<Mutex<Option<String>>>,
}

impl<'a> AdHocHooks<'a> {
    fn install(conn: &'a ProjectConnection) -> Result<Self, rusqlite::Error> {
        let readable = readable_tables(conn)?;
        let denied = Arc::new(Mutex::new(None));
        let record = denied.clone();
        conn.authorizer(Some(move |context: AuthContext<'_>| {
            match refusal(&readable, &context) {
                None => Authorization::Allow,
                Some(reason) => {
                    record.lock().get_or_insert(reason);
                    Authorization::Deny
                }
            }
        }))?;
        Ok(Self { conn, denied })
    }

    fn denied(&self) -> Option<String> {
        self.denied.lock().clone()
    }
}

impl Drop for AdHocHooks<'_> {
    fn drop(&mut self) {
        let _ = self.conn.progress_handler(0, None::<fn() -> bool>);
        let _ = self
            .conn
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    }
}

/// Lowercased names of the project's tables and views and of
/// [`READABLE_BUILT_IN_TABLES`]. SQLite reports table names as the schema spells
/// them, whatever case the statement used.
fn readable_tables(conn: &ProjectConnection) -> Result<FxHashSet<String>, rusqlite::Error> {
    let mut statement =
        conn.prepare("SELECT name FROM main.sqlite_schema WHERE type IN ('table', 'view')")?;
    let mut tables = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .map(|name| name.map(|name| name.to_ascii_lowercase()))
        .collect::<Result<FxHashSet<_>, _>>()?;
    tables.extend(READABLE_BUILT_IN_TABLES.map(str::to_string));
    Ok(tables)
}

/// The error message for why the authorizer refuses this step of the
/// statement, or `None` to allow it. Anything other than selecting, recursing,
/// reading a readable table and calling an allowed function is refused, so
/// nothing new SQLite adds slips by.
fn refusal(readable: &FxHashSet<String>, context: &AuthContext<'_>) -> Option<String> {
    match context.action {
        AuthAction::Select | AuthAction::Recursive => None,
        // A read that names no column, as in `count(*)`, has no database name.
        AuthAction::Read { table_name, .. } => {
            let in_main = matches!(context.database_name, None | Some("main"));
            (!in_main || !readable.contains(&table_name.to_ascii_lowercase()))
                .then(|| format!("queries may not read {table_name}"))
        }
        AuthAction::Function { function_name } => {
            let name = function_name.to_ascii_lowercase();
            (name.starts_with("sqlite_") || DENIED_FUNCTIONS.contains(&name.as_str()))
                .then(|| format!("queries may not call {function_name}()"))
        }
        // Everything else writes, or changes the connection or transaction.
        _ => Some(READ_ONLY_MESSAGE.to_string()),
    }
}

/// Tighten construct limits that catch prepare-time memory/stack blow-ups the
/// progress handler cannot. `ATTACH` is already denied from connection harden.
/// No worker threads keeps a sort to one core, the one its slot accounts for.
fn apply_ad_hoc_sqlite_limits(conn: &ProjectConnection) -> Result<(), rusqlite::Error> {
    conn.set_limit(Limit::SQLITE_LIMIT_WORKER_THREADS, 0)?;
    conn.set_limit(Limit::SQLITE_LIMIT_LENGTH, SQLITE_LIMIT_LENGTH)?;
    conn.set_limit(Limit::SQLITE_LIMIT_SQL_LENGTH, SQLITE_LIMIT_SQL_LENGTH)?;
    conn.set_limit(Limit::SQLITE_LIMIT_COLUMN, SQLITE_LIMIT_COLUMN)?;
    conn.set_limit(Limit::SQLITE_LIMIT_EXPR_DEPTH, SQLITE_LIMIT_EXPR_DEPTH)?;
    conn.set_limit(
        Limit::SQLITE_LIMIT_COMPOUND_SELECT,
        SQLITE_LIMIT_COMPOUND_SELECT,
    )?;
    conn.set_limit(Limit::SQLITE_LIMIT_VDBE_OP, SQLITE_LIMIT_VDBE_OP)?;
    Ok(())
}

fn validate_sql(sql: &str) -> Result<(), QueryError> {
    if sql.is_empty() {
        return Err(QueryError::Invalid("sql is required".to_string()));
    }
    if sql.len() > MAX_SQL_LEN {
        return Err(QueryError::Invalid(format!(
            "sql must be at most {MAX_SQL_LEN} bytes"
        )));
    }
    if !is_single_statement(sql) {
        return Err(QueryError::Invalid(
            "sql must contain a single statement".to_string(),
        ));
    }
    if !starts_with_allowed_keyword(sql) {
        return Err(QueryError::Invalid(format!(
            "sql must start with one of: {}",
            ALLOWED_LEADING_KEYWORDS.join(", ")
        )));
    }
    Ok(())
}

/// Runs `sql` and serializes at most `limit` rows, skipping the first `offset`.
/// Stops early on wall-clock timeout or approximate response size, setting
/// `has_more` so the caller can request the next page.
fn run_query(
    conn: &ProjectConnection,
    sql: &str,
    limit: u32,
    offset: u32,
    limits: QueryLimits,
) -> Result<QueryResponse, QueryError> {
    apply_ad_hoc_sqlite_limits(conn).map_err(execution_error)?;
    let hooks = AdHocHooks::install(conn).map_err(execution_error)?;

    // Deliberately not `prepare_cached`: the statement cache is sized for the
    // fixed dashboard queries and ad-hoc SQL would evict them.
    let mut statement = conn.prepare(sql).map_err(|error| match hooks.denied() {
        Some(refusal) => QueryError::Invalid(refusal),
        None => QueryError::Invalid(format!("invalid query: {error}")),
    })?;
    if !statement.readonly() {
        return Err(QueryError::Invalid(READ_ONLY_MESSAGE.to_string()));
    }
    let columns: Vec<String> = statement
        .column_names()
        .into_iter()
        .map(str::to_string)
        .collect();

    let started = Instant::now();
    let deadline = started + limits.timeout;
    conn.progress_handler(
        PROGRESS_HANDLER_OPS,
        Some(move || Instant::now() >= deadline),
    )
    .map_err(execution_error)?;

    let mut rows = statement.query([]).map_err(map_start_error)?;
    let mut skipped = 0_u32;
    let mut serialized: Vec<Vec<Value>> = Vec::new();
    let mut buffered_bytes = 0_usize;
    let mut has_more = false;
    loop {
        let row = match rows.next() {
            Ok(Some(row)) => row,
            Ok(None) => break,
            Err(error) if is_interrupted(&error) => {
                if serialized.is_empty() {
                    return Err(timeout_error());
                }
                has_more = true;
                break;
            }
            Err(error) => return Err(execution_error(error)),
        };
        if skipped < offset {
            skipped += 1;
            continue;
        }
        if serialized.len() as u32 >= limit {
            has_more = true;
            break;
        }

        let mut values = Vec::with_capacity(columns.len());
        let mut row_bytes = 0_usize;
        for index in 0..columns.len() {
            let value = row.get_ref(index).map_err(execution_error)?;
            row_bytes = row_bytes.saturating_add(estimate_value_bytes(value));
            values.push(value_to_json(value));
        }

        // Always accept at least one row so a single oversized row cannot stall
        // pagination; further rows stop once the soft cap would be exceeded.
        if !serialized.is_empty()
            && buffered_bytes.saturating_add(row_bytes) > limits.max_response_bytes
        {
            has_more = true;
            break;
        }

        buffered_bytes = buffered_bytes.saturating_add(row_bytes);
        serialized.push(values);

        if buffered_bytes >= limits.max_response_bytes || started.elapsed() >= limits.timeout {
            has_more = true;
            break;
        }
    }

    Ok(QueryResponse {
        columns,
        rows: serialized,
        limit,
        offset,
        has_more,
    })
}

fn estimate_value_bytes(value: ValueRef<'_>) -> usize {
    match value {
        ValueRef::Null | ValueRef::Integer(_) | ValueRef::Real(_) => FIXED_FIELD_BYTES,
        ValueRef::Text(text) => text.len(),
        ValueRef::Blob(blob) => blob.len(),
    }
}

/// Statement failures surface to the caller: they describe the SQL they sent.
fn execution_error(error: rusqlite::Error) -> QueryError {
    QueryError::Invalid(format!("query failed: {error}"))
}

fn timeout_error() -> QueryError {
    QueryError::Invalid(format!(
        "query timed out after {} seconds",
        QUERY_TIMEOUT.as_secs()
    ))
}

fn map_start_error(error: rusqlite::Error) -> QueryError {
    if is_interrupted(&error) {
        timeout_error()
    } else {
        execution_error(error)
    }
}

fn is_interrupted(error: &rusqlite::Error) -> bool {
    error.sqlite_error_code() == Some(ErrorCode::OperationInterrupted)
}

fn value_to_json(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(integer) => Value::Number(integer.into()),
        // NaN and infinity have no JSON representation.
        ValueRef::Real(real) => Number::from_f64(real).map_or(Value::Null, Value::Number),
        ValueRef::Text(text) => Value::String(String::from_utf8_lossy(text).into_owned()),
        ValueRef::Blob(blob) => Value::String(BASE64.encode(blob)),
    }
}

/// True when `sql` holds at most one statement. SQLite prepares only the first
/// statement of a string and silently ignores whatever follows, so a trailing
/// `; DROP ...` is rejected here rather than quietly dropped.
///
/// Unterminated quotes and comments are accepted so that `prepare` reports the
/// syntax error, which is a better message than a statement-count complaint.
fn is_single_statement(sql: &str) -> bool {
    let bytes = sql.as_bytes();
    let mut index = 0;
    let mut statement_ended = false;
    while index < bytes.len() {
        let byte = bytes[index];
        match byte {
            b';' => {
                statement_ended = true;
                index += 1;
            }
            b'\'' | b'"' | b'`' => {
                if statement_ended {
                    return false;
                }
                match skip_quoted(bytes, index, byte) {
                    Some(end) => index = end,
                    None => return true,
                }
            }
            b'[' => {
                if statement_ended {
                    return false;
                }
                match skip_bracket_identifier(bytes, index) {
                    Some(end) => index = end,
                    None => return true,
                }
            }
            _ => match skip_trivia_at(bytes, index) {
                Some(end) => index = end,
                None => {
                    if statement_ended {
                        return false;
                    }
                    index += 1;
                }
            },
        }
    }
    true
}

fn starts_with_allowed_keyword(sql: &str) -> bool {
    let bytes = sql.as_bytes();
    let mut index = 0;
    while let Some(end) = skip_trivia_at(bytes, index) {
        index = end;
    }
    let keyword_end = bytes[index..]
        .iter()
        .position(|byte| !byte.is_ascii_alphabetic())
        .map_or(bytes.len(), |offset| index + offset);
    let keyword = &sql[index..keyword_end];
    ALLOWED_LEADING_KEYWORDS
        .iter()
        .any(|allowed| keyword.eq_ignore_ascii_case(allowed))
}

/// Index just past the whitespace run or comment starting at `index`, or `None`
/// when `index` is not trivia. An unterminated comment consumes the remainder.
fn skip_trivia_at(bytes: &[u8], index: usize) -> Option<usize> {
    match bytes.get(index)? {
        byte if byte.is_ascii_whitespace() => Some(index + 1),
        b'-' if bytes.get(index + 1) == Some(&b'-') => Some(
            bytes[index..]
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(bytes.len(), |offset| index + offset + 1),
        ),
        b'/' if bytes.get(index + 1) == Some(&b'*') => Some(
            bytes[index + 2..]
                .windows(2)
                .position(|window| window == b"*/")
                .map_or(bytes.len(), |offset| index + offset + 4),
        ),
        _ => None,
    }
}

/// Index just past the string or quoted identifier opened at `index`, honouring
/// SQLite's doubled-delimiter escape. `None` when the quote is unterminated.
fn skip_quoted(bytes: &[u8], index: usize, delimiter: u8) -> Option<usize> {
    let mut cursor = index + 1;
    while cursor < bytes.len() {
        if bytes[cursor] != delimiter {
            cursor += 1;
        } else if bytes.get(cursor + 1) == Some(&delimiter) {
            cursor += 2;
        } else {
            return Some(cursor + 1);
        }
    }
    None
}

/// Index just past the `[quoted identifier]` opened at `index`, or `None` when
/// it is unterminated. Brackets have no escape form.
fn skip_bracket_identifier(bytes: &[u8], index: usize) -> Option<usize> {
    bytes[index..]
        .iter()
        .position(|byte| *byte == b']')
        .map(|offset| index + offset + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::project::in_memory_test_connection;
    use crate::download_db::DEMO_PROJECT_ID;
    use crate::query_slots::QuerySlot;

    fn default_limits() -> QueryLimits {
        QueryLimits {
            timeout: QUERY_TIMEOUT,
            max_response_bytes: MAX_RESPONSE_BYTES,
        }
    }

    fn seeded_connection() -> ProjectConnection {
        let conn = in_memory_test_connection();
        conn.execute(
            "CREATE TABLE t (id INTEGER, name TEXT, score REAL, raw BLOB)",
            [],
        )
        .expect("create table");
        for id in 1..=5_i64 {
            conn.execute(
                "INSERT INTO t (id, name, score, raw) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![id, format!("row{id}"), id as f64 / 2.0, vec![id as u8]],
            )
            .expect("insert row");
        }
        conn
    }

    fn invalid_message(error: QueryError) -> String {
        match error {
            QueryError::Invalid(message) => message,
            other => panic!("expected invalid query, got {other:?}"),
        }
    }

    #[test]
    fn returns_columns_and_typed_rows() {
        let conn = seeded_connection();
        let response = run_query(
            &conn,
            "SELECT id, name, score, raw, NULL FROM t",
            2,
            0,
            default_limits(),
        )
        .expect("run query");
        assert_eq!(response.columns, vec!["id", "name", "score", "raw", "NULL"]);
        assert_eq!(response.rows.len(), 2);
        assert_eq!(response.rows[0][0], Value::Number(1.into()));
        assert_eq!(response.rows[0][1], Value::String("row1".to_string()));
        assert_eq!(
            response.rows[0][2],
            Value::Number(Number::from_f64(0.5).unwrap())
        );
        assert_eq!(response.rows[0][3], Value::String(BASE64.encode([1_u8])));
        assert_eq!(response.rows[0][4], Value::Null);
        assert!(response.has_more);
    }

    #[test]
    fn applies_offset_and_reports_end_of_results() {
        let conn = seeded_connection();
        let response = run_query(
            &conn,
            "SELECT id FROM t ORDER BY id",
            10,
            3,
            default_limits(),
        )
        .expect("run query");
        assert_eq!(
            response.rows,
            vec![vec![Value::Number(4.into())], vec![Value::Number(5.into())]]
        );
        assert!(!response.has_more);
        assert_eq!(response.offset, 3);
    }

    #[test]
    fn rejects_statements_that_write() {
        let conn = seeded_connection();
        let error = run_query(
            &conn,
            "INSERT INTO t (id) VALUES (9)",
            10,
            0,
            default_limits(),
        )
        .unwrap_err();
        assert_eq!(invalid_message(error), "query must be read-only");
    }

    #[test]
    fn reports_sqlite_errors_as_invalid_queries() {
        let conn = seeded_connection();
        let error = run_query(&conn, "SELECT * FROM missing", 10, 0, default_limits()).unwrap_err();
        assert!(
            invalid_message(error).starts_with("invalid query:"),
            "prepare failures should be reported to the caller"
        );
    }

    #[test]
    fn stops_when_response_size_cap_is_reached() {
        let conn = seeded_connection();
        // Three 40-byte names: first two fit under a 100-byte cap (80), third would
        // push past it, so we keep two rows and advertise another page.
        for id in 1..=3_i64 {
            conn.execute(
                "UPDATE t SET name = ?1 WHERE id = ?2",
                rusqlite::params!["x".repeat(40), id],
            )
            .expect("widen name");
        }
        let response = run_query(
            &conn,
            "SELECT name FROM t ORDER BY id",
            10,
            0,
            QueryLimits {
                timeout: QUERY_TIMEOUT,
                max_response_bytes: 100,
            },
        )
        .expect("run query");
        assert_eq!(response.rows.len(), 2);
        assert!(response.has_more);
    }

    #[test]
    fn still_returns_a_single_row_that_exceeds_the_size_cap() {
        let conn = seeded_connection();
        conn.execute(
            "UPDATE t SET name = ?1 WHERE id = 1",
            rusqlite::params!["x".repeat(200)],
        )
        .expect("widen name");
        let response = run_query(
            &conn,
            "SELECT name FROM t WHERE id = 1",
            10,
            0,
            QueryLimits {
                timeout: QUERY_TIMEOUT,
                max_response_bytes: 50,
            },
        )
        .expect("run query");
        assert_eq!(response.rows.len(), 1);
        // Cap already reached after the first row; advertise pagination even
        // though this particular filter has no further matches.
        assert!(response.has_more);
    }

    #[test]
    fn rejects_expressions_deeper_than_the_sqlite_limit() {
        let conn = seeded_connection();
        let depth = (SQLITE_LIMIT_EXPR_DEPTH as usize) + 1;
        let mut sql = String::from("SELECT ");
        for _ in 0..depth {
            sql.push_str("(1+");
        }
        sql.push('0');
        for _ in 0..depth {
            sql.push(')');
        }
        let error = run_query(&conn, &sql, 10, 0, default_limits()).unwrap_err();
        assert!(
            invalid_message(error).starts_with("invalid query:"),
            "over-deep expressions should fail at prepare"
        );
    }

    #[test]
    fn rejects_compound_selects_beyond_the_sqlite_limit() {
        let conn = seeded_connection();
        let terms = (SQLITE_LIMIT_COMPOUND_SELECT as usize) + 1;
        let sql = (0..terms)
            .map(|_| "SELECT 1")
            .collect::<Vec<_>>()
            .join(" UNION ALL ");
        let error = run_query(&conn, &sql, 10, 0, default_limits()).unwrap_err();
        assert!(
            invalid_message(error).starts_with("invalid query:"),
            "too many compound SELECT terms should fail at prepare"
        );
    }

    #[test]
    fn rejects_result_sets_wider_than_the_sqlite_column_limit() {
        let conn = seeded_connection();
        let columns = (SQLITE_LIMIT_COLUMN as usize) + 1;
        let sql = format!(
            "SELECT {}",
            (0..columns)
                .map(|index| index.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
        let error = run_query(&conn, &sql, 10, 0, default_limits()).unwrap_err();
        assert!(
            invalid_message(error).starts_with("invalid query:"),
            "too many result columns should fail at prepare"
        );
    }

    #[test]
    fn applies_ad_hoc_sqlite_limits_on_the_connection() {
        let conn = seeded_connection();
        apply_ad_hoc_sqlite_limits(&conn).expect("apply sqlite limits");
        assert_eq!(
            conn.limit(Limit::SQLITE_LIMIT_LENGTH)
                .expect("read length limit"),
            SQLITE_LIMIT_LENGTH
        );
        assert_eq!(
            conn.limit(Limit::SQLITE_LIMIT_SQL_LENGTH)
                .expect("read sql length limit"),
            SQLITE_LIMIT_SQL_LENGTH
        );
        assert_eq!(
            conn.limit(Limit::SQLITE_LIMIT_COLUMN)
                .expect("read column limit"),
            SQLITE_LIMIT_COLUMN
        );
        assert_eq!(
            conn.limit(Limit::SQLITE_LIMIT_EXPR_DEPTH)
                .expect("read expr depth limit"),
            SQLITE_LIMIT_EXPR_DEPTH
        );
        assert_eq!(
            conn.limit(Limit::SQLITE_LIMIT_COMPOUND_SELECT)
                .expect("read compound select limit"),
            SQLITE_LIMIT_COMPOUND_SELECT
        );
        assert_eq!(
            conn.limit(Limit::SQLITE_LIMIT_WORKER_THREADS)
                .expect("read worker threads limit"),
            0
        );
        assert_eq!(
            conn.limit(Limit::SQLITE_LIMIT_VDBE_OP)
                .expect("read vdbe op limit"),
            SQLITE_LIMIT_VDBE_OP
        );
    }

    #[test]
    fn stops_when_wall_clock_timeout_elapses_between_rows() {
        let conn = seeded_connection();
        let response = run_query(
            &conn,
            "SELECT id FROM t ORDER BY id",
            10,
            0,
            QueryLimits {
                // Soft stop after the first buffered row; a simple five-row
                // select usually finishes before the progress handler fires.
                timeout: Duration::ZERO,
                max_response_bytes: MAX_RESPONSE_BYTES,
            },
        )
        .expect("run query");
        assert_eq!(response.rows.len(), 1);
        assert!(response.has_more);
    }

    #[test]
    fn times_out_expensive_query_before_returning_rows() {
        let conn = seeded_connection();
        // Aggregate so SQLite does the heavy work before yielding a row; the
        // progress handler then interrupts with an empty buffer.
        let error = run_query(
            &conn,
            "WITH RECURSIVE r(i) AS (
                SELECT 1
                UNION ALL
                SELECT i + 1 FROM r WHERE i < 100000000
             )
             SELECT MAX(i) FROM r",
            10,
            0,
            QueryLimits {
                timeout: Duration::from_millis(50),
                max_response_bytes: MAX_RESPONSE_BYTES,
            },
        )
        .unwrap_err();
        assert_eq!(
            invalid_message(error),
            format!("query timed out after {} seconds", QUERY_TIMEOUT.as_secs())
        );
    }

    #[test]
    fn refuses_built_in_tables_that_describe_the_server() {
        let conn = seeded_connection();
        for (sql, refused) in [
            (
                "SELECT * FROM pragma_database_list",
                "read pragma_database_list",
            ),
            (
                "SELECT * FROM pragma_compile_options",
                "read pragma_compile_options",
            ),
            ("SELECT * FROM dbstat", "read dbstat"),
            (
                "SELECT * FROM temp.sqlite_schema",
                "read sqlite_temp_master",
            ),
            ("SELECT sqlite_version()", "call sqlite_version()"),
            ("SELECT SQLITE_SOURCE_ID()", "call sqlite_source_id()"),
            ("SELECT load_extension('x')", "call load_extension()"),
            ("SELECT fts5_source_id()", "call fts5_source_id()"),
            ("SELECT fts3_tokenizer('simple')", "call fts3_tokenizer()"),
        ] {
            let error = run_query(&conn, sql, 10, 0, default_limits()).unwrap_err();
            assert_eq!(
                invalid_message(error),
                format!("queries may not {refused}"),
                "{sql}"
            );
        }
    }

    #[test]
    fn allows_the_project_tables_views_and_ordinary_sql() {
        let conn = seeded_connection();
        conn.execute("CREATE VIEW names AS SELECT lower(name) AS name FROM t", [])
            .expect("create view");
        for sql in [
            "SELECT upper(name), COUNT(*) FROM T GROUP BY 1",
            "SELECT name FROM names",
            "SELECT id, row_number() OVER (ORDER BY id) FROM t",
            "SELECT count(*) FROM t",
            "SELECT 1 FROM t",
            "WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3) \
             SELECT n FROM r",
            "SELECT value FROM json_each('[1, 2]')",
            "SELECT name FROM sqlite_schema",
            "EXPLAIN QUERY PLAN SELECT id FROM t",
            "VALUES (1)",
        ] {
            run_query(&conn, sql, 10, 0, default_limits())
                .unwrap_or_else(|error| panic!("{sql}: {}", error.client_message()));
        }
    }

    #[test]
    fn clears_the_authorizer_after_the_statement() {
        let conn = seeded_connection();
        run_query(&conn, "SELECT id FROM t", 10, 0, default_limits()).expect("run query");
        conn.prepare("SELECT * FROM pragma_database_list")
            .expect("later statements on the connection are not restricted");
    }

    #[test]
    fn refuses_a_project_already_running_its_share_of_statements() {
        let server_context = Arc::new(ServerContext::new_in_memory("query-busy"));
        let _held = hold_every_slot(&server_context, SlotOwner::Project("2".to_string()));
        let error = run_project_query(
            server_context.clone(),
            "2",
            "someone else",
            "SELECT 1",
            None,
            None,
        )
        .unwrap_err();
        assert!(matches!(error, QueryError::Busy), "{error:?}");
    }

    #[test]
    fn charges_the_demo_to_each_reader() {
        let server_context = Arc::new(ServerContext::new_in_memory("query-demo-busy"));
        let _held = hold_every_slot(
            &server_context,
            SlotOwner::DemoReader("203.0.113.1".to_string()),
        );
        let refused = run_project_query(
            server_context.clone(),
            DEMO_PROJECT_ID,
            "203.0.113.1",
            "SELECT 1",
            None,
            None,
        )
        .unwrap_err();
        assert!(matches!(refused, QueryError::Busy), "{refused:?}");
        // The demo database isn't loaded in this context, so the other reader
        // gets past its slot and fails to open it instead.
        let other = run_project_query(
            server_context.clone(),
            DEMO_PROJECT_ID,
            "203.0.113.2",
            "SELECT 1",
            None,
            None,
        )
        .unwrap_err();
        assert!(matches!(other, QueryError::Auth(_)), "{other:?}");
    }

    /// Takes slots for `owner` until it is refused one.
    fn hold_every_slot(server_context: &ServerContext, owner: SlotOwner) -> Vec<QuerySlot<'_>> {
        let held: Vec<_> =
            std::iter::from_fn(|| server_context.ad_hoc_query_slots.try_acquire(owner.clone()))
                .collect();
        assert!(!held.is_empty(), "the owner had a slot to take");
        held
    }

    #[test]
    fn accepts_a_single_statement_with_semicolons_in_literals() {
        assert!(is_single_statement("SELECT 1"));
        assert!(is_single_statement("SELECT 1;"));
        assert!(is_single_statement("SELECT 1;  \n -- trailing\n"));
        assert!(is_single_statement("SELECT ';' AS x"));
        assert!(is_single_statement("SELECT 'it''s; fine' AS x"));
        assert!(is_single_statement(r#"SELECT "a;b" FROM t"#));
        assert!(is_single_statement("SELECT [a;b] FROM t"));
        assert!(is_single_statement("SELECT 1 /* ; */"));
        assert!(is_single_statement("SELECT 1 -- ;\n"));
    }

    #[test]
    fn rejects_trailing_statements() {
        assert!(!is_single_statement("SELECT 1; SELECT 2"));
        assert!(!is_single_statement("SELECT 1;DROP TABLE t"));
        assert!(!is_single_statement("SELECT 1; -- comment\n DELETE FROM t"));
        assert!(!is_single_statement("SELECT 'x'; SELECT 'y'"));
    }

    #[test]
    fn allows_only_read_only_leading_keywords() {
        assert!(starts_with_allowed_keyword("select 1"));
        assert!(starts_with_allowed_keyword(
            "  /* note */ WITH x AS (SELECT 1) SELECT * FROM x"
        ));
        assert!(starts_with_allowed_keyword("-- note\nEXPLAIN SELECT 1"));
        assert!(starts_with_allowed_keyword("VALUES (1)"));
        assert!(!starts_with_allowed_keyword("PRAGMA table_list"));
        assert!(!starts_with_allowed_keyword("BEGIN"));
        assert!(!starts_with_allowed_keyword("selectish"));
    }

    #[test]
    fn validate_sql_rejects_empty_and_oversized_statements() {
        assert_eq!(
            invalid_message(validate_sql("").unwrap_err()),
            "sql is required"
        );
        let long_sql = format!("SELECT '{}'", "x".repeat(MAX_SQL_LEN));
        assert!(
            invalid_message(validate_sql(&long_sql).unwrap_err())
                .starts_with("sql must be at most")
        );
    }
}
