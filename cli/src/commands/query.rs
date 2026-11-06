//! Reading a project's analytics database: SQL queries, the schema, and tables.
//!
//! The server only runs read-only statements on a read-only connection, so nothing
//! here can change the database.

use revscope_api::paths;
use revscope_api::query::{MAX_LIMIT, MAX_OFFSET, QueryRequest, QueryResponse};
use serde::Serialize;

use super::value_or_stdin;
use crate::cli::{QueryArgs, SchemaArgs};
use crate::context::{Context, Project};
use crate::error::{CliError, Result};
use crate::output::{Render, Report, cell, table};

fn run(
    context: &Context,
    project: &Project,
    sql: &str,
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<QueryResponse> {
    context.client.post(
        &paths::query_sql(&project.id),
        &QueryRequest {
            sql: sql.to_string(),
            limit,
            offset,
        },
    )
}

pub fn query(context: &Context, args: &QueryArgs) -> Result<Report> {
    let sql = value_or_stdin(&args.sql, "SQL")?;
    let project = context.project()?;
    if args.all {
        return Report::of(&fetch_all(context, &project, &sql, args.limit)?);
    }
    Report::of(&Rows(run(
        context,
        &project,
        &sql,
        args.limit,
        args.offset,
    )?))
}

/// Follows `has_more` page by page. The server's offset cap ends the walk, and the
/// result then keeps `has_more` set, so a truncated answer doesn't read as complete.
fn fetch_all(context: &Context, project: &Project, sql: &str, limit: Option<u32>) -> Result<Rows> {
    let mut all = run(
        context,
        project,
        sql,
        Some(limit.unwrap_or(MAX_LIMIT)),
        Some(0),
    )?;
    while all.has_more {
        let Ok(next) = u32::try_from(all.rows.len()) else {
            break;
        };
        if next > MAX_OFFSET {
            break;
        }
        let page = run(context, project, sql, Some(all.limit), Some(next))?;
        // The server always returns a row while there are more, so an empty page is a
        // surprise. Stop rather than loop, leaving `has_more` set: the answer is incomplete.
        if page.rows.is_empty() {
            break;
        }
        all.rows.extend(page.rows);
        all.has_more = page.has_more;
    }
    Ok(Rows(all))
}

pub fn tables(context: &Context) -> Result<Report> {
    let project = context.project()?;
    let result = run(
        context,
        &project,
        "SELECT name FROM sqlite_master WHERE type = 'table' AND substr(name, 1, 7) <> 'sqlite_' ORDER BY name",
        Some(MAX_LIMIT),
        None,
    )?;
    Report::of(&Tables {
        tables: first_column(&result),
    })
}

pub fn schema(context: &Context, args: &SchemaArgs) -> Result<Report> {
    // Checked before any request, since the name goes into a SQL string.
    if let Some(name) = args.table.as_deref().filter(|name| !is_identifier(name)) {
        return Err(CliError::usage(format!(
            "`{name}` is not a table name: use letters, digits and underscores only"
        )));
    }
    let project = context.project()?;
    let Some(table_name) = &args.table else {
        let sql = context.client.get_text(&paths::query_schema(&project.id))?;
        return Report::of(&Schema { table: None, sql });
    };
    let result = run(
        context,
        &project,
        &format!(
            "SELECT sql FROM sqlite_master WHERE tbl_name = '{table_name}' AND sql IS NOT NULL ORDER BY type DESC, name"
        ),
        Some(MAX_LIMIT),
        None,
    )?;
    let statements = first_column(&result);
    if statements.is_empty() {
        return Err(CliError::usage(format!(
            "no table named `{table_name}`; `revscope tables` lists them"
        )));
    }
    Report::of(&Schema {
        table: Some(table_name.clone()),
        sql: statements
            .iter()
            .map(|sql| format!("{sql};"))
            .collect::<Vec<_>>()
            .join("\n\n"),
    })
}

fn first_column(result: &QueryResponse) -> Vec<String> {
    result
        .rows
        .iter()
        .filter_map(|row| {
            row.first()
                .and_then(|value| value.as_str())
                .map(str::to_string)
        })
        .collect()
}

/// A plain SQL identifier: letters, digits and underscores, not starting with a digit.
fn is_identifier(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[derive(Serialize)]
#[serde(transparent)]
struct Rows(QueryResponse);

impl Render for Rows {
    fn render(&self) -> String {
        let result = &self.0;
        if result.columns.is_empty() {
            return "The statement returned no columns.".to_string();
        }
        let headers: Vec<&str> = result.columns.iter().map(String::as_str).collect();
        let rows: Vec<Vec<String>> = result
            .rows
            .iter()
            .map(|row| row.iter().map(cell).collect())
            .collect();
        let mut out = table(&headers, &rows);
        if rows.is_empty() {
            out.push_str("\n(no rows)");
        }
        if result.has_more {
            // The server can stop a page short of `limit` (byte, size or time cap), so
            // the next page starts after the rows actually returned.
            let next = u64::from(result.offset) + rows.len() as u64;
            if next > u64::from(MAX_OFFSET) {
                out.push_str(&format!(
                    "\n\nMore rows exist past the offset limit of {MAX_OFFSET}. Narrow the query with WHERE."
                ));
            } else {
                out.push_str(&format!(
                    "\n\nMore rows are available: add --offset {next} to see the next page."
                ));
            }
        }
        out
    }
}

#[derive(Serialize)]
struct Tables {
    tables: Vec<String>,
}

impl Render for Tables {
    fn render(&self) -> String {
        if self.tables.is_empty() {
            return "This project has no tables yet.".to_string();
        }
        self.tables.join("\n")
    }
}

#[derive(Serialize)]
struct Schema {
    #[serde(skip_serializing_if = "Option::is_none")]
    table: Option<String>,
    sql: String,
}

impl Render for Schema {
    fn render(&self) -> String {
        self.sql.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn response(rows: usize, limit: u32, offset: u32, has_more: bool) -> Rows {
        Rows(QueryResponse {
            columns: vec!["n".to_string()],
            rows: (0..rows).map(|n| vec![json!(n)]).collect(),
            limit,
            offset,
            has_more,
        })
    }

    #[test]
    fn table_names_are_checked_before_they_reach_sql() {
        assert!(is_identifier("page_views2"));
        assert!(!is_identifier(""));
        assert!(!is_identifier("events'; DROP TABLE events; --"));
        assert!(!is_identifier("2024_events"));
        assert!(!is_identifier("events name"));
    }

    #[test]
    fn a_page_cut_short_by_the_server_resumes_after_the_rows_it_returned() {
        assert!(
            response(3, 100, 40, true)
                .render()
                .ends_with("add --offset 43 to see the next page.")
        );
    }

    #[test]
    fn past_the_offset_limit_it_says_to_narrow_the_query() {
        assert!(
            response(100, 100, MAX_OFFSET, true)
                .render()
                .contains("Narrow the query")
        );
    }

    #[test]
    fn an_empty_result_says_so() {
        assert!(response(0, 100, 0, false).render().ends_with("(no rows)"));
    }
}
