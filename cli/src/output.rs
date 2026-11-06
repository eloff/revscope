//! What a command prints: its result as text, or as JSON with `--json`.

use std::io::Write;

use serde::Serialize;
use serde_json::Value;

use crate::error::{CliError, Exit, Result};

/// Unlike `eprintln!`, doesn't panic when stderr is closed, with nothing left to tell.
pub fn print_stderr(text: &str) {
    let _ = writeln!(std::io::stderr(), "{text}");
}

/// A command's result, in both forms it can be printed in.
pub trait Render: Serialize {
    fn render(&self) -> String;

    /// The exit status when the command ran fine but its answer is a failure.
    fn exit(&self) -> Option<Exit> {
        None
    }
}

#[derive(Debug)]
pub struct Report {
    pub json: Value,
    pub text: String,
    pub exit: Option<Exit>,
}

impl Report {
    pub fn of<T: Render>(result: &T) -> Result<Self> {
        Ok(Self {
            json: serde_json::to_value(result)
                .map_err(|error| CliError::local(format!("encoding the result: {error}")))?,
            text: result.render(),
            exit: result.exit(),
        })
    }
}

/// Left-aligned columns separated by two spaces, the widest cell setting each
/// width. Cells stay whole, since a query's output should show what it returned;
/// line breaks are escaped so one cell can't break the rows after it.
pub fn table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let rows: Vec<Vec<String>> = rows
        .iter()
        .map(|row| row.iter().map(|cell| escape_cell(cell)).collect())
        .collect();
    let mut widths: Vec<usize> = headers
        .iter()
        .map(|header| header.chars().count())
        .collect();
    for row in &rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let mut lines = vec![
        line(headers.iter().copied(), &widths),
        line(
            widths
                .iter()
                .map(|width| "-".repeat(*width))
                .collect::<Vec<_>>()
                .iter()
                .map(String::as_str),
            &widths,
        ),
    ];
    lines.extend(
        rows.iter()
            .map(|row| line(row.iter().map(String::as_str), &widths)),
    );
    lines.join("\n")
}

fn line<'a>(cells: impl Iterator<Item = &'a str>, widths: &[usize]) -> String {
    let mut out = String::new();
    for (cell, width) in cells.zip(widths) {
        out.push_str(cell);
        out.push_str(&" ".repeat(width.saturating_sub(cell.chars().count()) + 2));
    }
    out.trim_end().to_string()
}

fn escape_cell(cell: &str) -> String {
    cell.replace('\r', "\\r").replace('\n', "\\n")
}

/// A JSON value as a cell: strings without quotes, null as empty.
pub fn cell(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

pub fn opt(value: Option<&str>) -> &str {
    value.unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn columns_line_up_on_the_widest_cell() {
        let rendered = table(
            &["name", "plan"],
            &[
                vec!["shop".into(), "free".into()],
                vec!["a-much-longer-name".into(), "growth".into()],
            ],
        );
        let lines: Vec<&str> = rendered.lines().collect();
        assert_eq!(lines[0], format!("{:<20}plan", "name"));
        assert_eq!(lines[2], format!("{:<20}free", "shop"));
        assert_eq!(lines[3], "a-much-longer-name  growth");
    }

    #[test]
    fn a_line_break_in_a_cell_cannot_break_the_table() {
        let rendered = table(&["body"], &[vec!["two\nlines".into()]]);
        assert_eq!(rendered.lines().count(), 3);
        assert!(rendered.ends_with("two\\nlines"));
    }

    #[test]
    fn json_cells_are_shown_without_quotes() {
        assert_eq!(cell(&json!("text")), "text");
        assert_eq!(cell(&json!(null)), "");
        assert_eq!(cell(&json!(42)), "42");
    }
}
