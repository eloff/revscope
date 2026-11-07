//! Importing history from Google Analytics. Connect a credential to learn which
//! properties it can read, then start an import of one. Imports run on the server.

use revscope_api::ga_import::{
    ConnectRequest, ConnectResponse, DeletedImport, ImportView, ImportsResponse, OAuthUrlResponse,
    StartImportRequest,
};
use revscope_api::paths;
use serde::Serialize;

use super::value_or_stdin;
use crate::cli::{GaConnectArgs, GaImportIdArgs, GaStartArgs};
use crate::context::Context;
use crate::error::{CliError, Result};
use crate::output::{Render, Report, opt, table};

pub fn connect(context: &Context, args: &GaConnectArgs) -> Result<Report> {
    let path = &args.service_account_file;
    let service_account_json = if path == "-" {
        value_or_stdin("-", "service account key")?
    } else {
        std::fs::read_to_string(path)
            .map_err(|error| CliError::usage(format!("reading {path}: {error}")))?
    };
    let project = context.project()?;
    let connected: ConnectResponse = context.client.post(
        &paths::ga_import_connect(&project.id),
        &ConnectRequest::ServiceAccount {
            service_account_json,
        },
    )?;
    Report::of(&Connected(connected))
}

pub fn oauth_url(context: &Context) -> Result<Report> {
    let project = context.project()?;
    let url: OAuthUrlResponse = context
        .client
        .post_empty(&paths::ga_import_oauth_url(&project.id))?;
    Report::of(&AuthorizeUrl(url))
}

pub fn list(context: &Context) -> Result<Report> {
    let project = context.project()?;
    let imports: ImportsResponse = context.client.get(&paths::ga_imports(&project.id))?;
    Report::of(&Imports(imports))
}

pub fn start(context: &Context, args: &GaStartArgs) -> Result<Report> {
    let project = context.project()?;
    let started: ImportView = context.client.post(
        &paths::ga_imports(&project.id),
        &StartImportRequest {
            connection_id: args.connection.clone(),
            property_id: args.property.clone(),
            start_date: args.start.clone(),
            end_date: args.end.clone(),
            hostnames: args.hostname.clone(),
            allow_duplicates: args.allow_duplicates,
        },
    )?;
    Report::of(&OneImport(started))
}

pub fn cancel(context: &Context, args: &GaImportIdArgs) -> Result<Report> {
    let project = context.project()?;
    let cancelled: ImportView = context
        .client
        .post_empty(&paths::ga_import_cancel(&project.id, args.id))?;
    Report::of(&OneImport(cancelled))
}

pub fn delete(context: &Context, args: &GaImportIdArgs) -> Result<Report> {
    let project = context.project()?;
    let deleted: DeletedImport = context
        .client
        .delete(&paths::ga_import(&project.id, args.id))?;
    Report::of(&Deleted(deleted))
}

fn import_rows(imports: &[ImportView]) -> String {
    let rows: Vec<Vec<String>> = imports
        .iter()
        .map(|import| {
            vec![
                import.id.to_string(),
                import.property_name.clone(),
                format!("{} to {}", import.start_date, import.end_date),
                import.status.clone(),
                // `running` can be waiting too: an error means its last attempt failed.
                if import.status == "paused"
                    || (import.status == "running" && import.error.is_some())
                {
                    without_fractional_seconds(&import.resume_at)
                } else {
                    "-".to_string()
                },
                format!("{}/{}", import.chunks_done, import.chunks_total),
                import.sessions_imported.to_string(),
                opt(import.error.as_deref()).to_string(),
            ]
        })
        .collect();
    table(
        &[
            "ID",
            "PROPERTY",
            "RANGE",
            "STATUS",
            "NEXT ATTEMPT",
            "CHUNKS",
            "SESSIONS",
            "ERROR",
        ],
        &rows,
    )
}

/// The API sends Postgres's timestamp text, down to the microsecond.
fn without_fractional_seconds(timestamp: &str) -> String {
    let Some((seconds, fraction)) = timestamp.split_once('.') else {
        return timestamp.to_string();
    };
    let offset = fraction.trim_start_matches(|digit: char| digit.is_ascii_digit());
    format!("{seconds}{offset}")
}

#[derive(Serialize)]
#[serde(transparent)]
struct Connected(ConnectResponse);

impl Render for Connected {
    fn render(&self) -> String {
        let connected = &self.0;
        if connected.properties.is_empty() {
            return format!(
                "Connected ({}), but it can't read any properties. Add it as a viewer on the Google Analytics property.",
                connected.connection_id
            );
        }
        let rows: Vec<Vec<String>> = connected
            .properties
            .iter()
            .map(|property| {
                vec![
                    property.id.clone(),
                    property.name.clone(),
                    property.account.clone(),
                ]
            })
            .collect();
        format!(
            "Connected. Start an import with `revscope ga-import start --connection {} --property <ID> --start YYYY-MM-DD --end YYYY-MM-DD`.\n\n{}",
            connected.connection_id,
            table(&["ID", "PROPERTY", "ACCOUNT"], &rows)
        )
    }
}

#[derive(Serialize)]
#[serde(transparent)]
struct AuthorizeUrl(OAuthUrlResponse);

impl Render for AuthorizeUrl {
    fn render(&self) -> String {
        format!(
            "Open this in a browser and sign in with Google. The import continues in the dashboard's settings:\n\n{}",
            self.0.url
        )
    }
}

#[derive(Serialize)]
#[serde(transparent)]
struct Imports(ImportsResponse);

impl Render for Imports {
    fn render(&self) -> String {
        let imports = &self.0;
        let mut out = if imports.imports.is_empty() {
            "No imports yet.".to_string()
        } else {
            import_rows(&imports.imports)
        };
        out.push_str(&format!(
            "\n\nAllowed range: {} to {}. Suggested: {} to {}.",
            opt(imports.oldest_allowed_date.as_deref()),
            imports.latest_allowed_date,
            imports.default_start_date,
            imports.default_end_date,
        ));
        if let Some(first) = &imports.first_tracked_date {
            out.push_str(&format!(
                " Tracking began {first}, so an end date on or after it duplicates visits unless --allow-duplicates is given."
            ));
        }
        out
    }
}

#[derive(Serialize)]
#[serde(transparent)]
struct OneImport(ImportView);

impl Render for OneImport {
    fn render(&self) -> String {
        import_rows(std::slice::from_ref(&self.0))
    }
}

#[derive(Serialize)]
#[serde(transparent)]
struct Deleted(DeletedImport);

impl Render for Deleted {
    fn render(&self) -> String {
        format!("Deleted import {} and the data it brought in.", self.0.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortens_a_timestamp_to_seconds_and_keeps_its_offset() {
        assert_eq!(
            without_fractional_seconds("2026-11-07 14:32:10.482913+00"),
            "2026-11-07 14:32:10+00"
        );
        assert_eq!(
            without_fractional_seconds("2026-11-07 15:00:00+00"),
            "2026-11-07 15:00:00+00"
        );
    }
}
