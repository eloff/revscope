//! The commands. Each file is one command group.

pub mod account;
pub mod browser_login;
pub mod ga_import;
pub mod install;
pub mod integrations;
pub mod projects;
pub mod query;
pub mod settings;

use std::io::{BufRead, Read};

use crate::error::{CliError, Result};

/// The value as given, or, for `-`, all of stdin without its trailing line break,
/// so a PEM key or a multi-line statement arrives whole.
pub fn value_or_stdin(value: &str, what: &str) -> Result<String> {
    if value != "-" {
        return Ok(value.to_string());
    }
    let mut text = String::new();
    std::io::stdin()
        .lock()
        .read_to_string(&mut text)
        .map_err(|error| CliError::local(format!("reading the {what} from stdin: {error}")))?;
    let text = text.trim_end_matches(['\n', '\r']).to_string();
    if text.trim().is_empty() {
        return Err(CliError::usage(format!("the {what} on stdin is empty")));
    }
    Ok(text)
}

/// The first line of stdin, without its line break. Passwords are one line.
pub fn stdin_line(what: &str) -> Result<String> {
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|error| CliError::local(format!("reading the {what} from stdin: {error}")))?;
    let line = line.trim_end_matches(['\n', '\r']).to_string();
    if line.trim().is_empty() {
        return Err(CliError::usage(format!("the {what} on stdin is empty")));
    }
    Ok(line)
}
