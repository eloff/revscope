//! The import section of Settings: the form's defaults and the imports so far.

use std::sync::Arc;

use jiff::ToSpan;
use jiff::civil::{Date, date};
pub(crate) use revscope_api::ga_import::{ImportView, ImportsResponse};

use crate::auth::{AuthError, map_row_error};
use crate::authorize::AdminProject;
use crate::db::main::Connection;
use crate::db::{open_main_database_read, open_project_database_read};
use crate::plan_retention::oldest_retained_date;
use crate::server_context::ServerContext;
use crate::timezone;
use crate::util::{block_on, unix_seconds};

use super::read::reports_include_imported;
use super::start_import::default_end_date;
use super::store;

/// GA4 launched here, so nothing older can exist. The default start for a plan
/// that keeps everything.
const GA4_LAUNCH: Date = date(2020, 10, 14);

const IMPORT_COLUMNS: &str = "id, property_name, start_date::text, end_date::text, hostnames,
    status, chunks_done, chunks_total, resume_at::text, error_text";

fn import_view(row: &tokio_postgres::Row) -> ImportView {
    ImportView {
        id: row.get(0),
        property_name: row.get(1),
        start_date: row.get(2),
        end_date: row.get(3),
        hostnames: row.get(4),
        status: row.get(5),
        chunks_done: row.get(6),
        chunks_total: row.get(7),
        resume_at: row.get(8),
        error: row.get(9),
        sessions_imported: 0,
    }
}

async fn load_imports(conn: &Connection, project_key: i32) -> Result<Vec<ImportView>, AuthError> {
    let rows = conn
        .query(
            &format!(
                "SELECT {IMPORT_COLUMNS} FROM ga_imports WHERE project_id = $1 ORDER BY id DESC"
            ),
            &[&project_key],
        )
        .await
        .map_err(AuthError::from)?;
    Ok(rows.iter().map(import_view).collect())
}

pub(crate) fn list_imports(
    server_context: Arc<ServerContext>,
    project: &AdminProject,
) -> Result<ImportsResponse, AuthError> {
    let timezone = timezone::project_timezone(server_context.clone(), project.key());
    let today = timezone
        .date_of(unix_seconds())
        .ok_or(AuthError::Temporary("couldn't read the current date"))?;
    let latest_allowed = today
        .checked_sub(1.day())
        .map_err(|_| AuthError::Temporary("couldn't read the current date"))?;
    let oldest_allowed = oldest_retained_date(&project.admin().user().plan, today);
    let default_start = oldest_allowed.unwrap_or(GA4_LAUNCH);

    let conn = open_project_database_read(server_context.as_ref(), project.id())?;
    let first_tracked = store::first_tracked_second(&conn)
        .map_err(map_row_error)?
        .and_then(|second| timezone.date_of(second));
    let include_in_reports = reports_include_imported(&conn).map_err(map_row_error)?;
    let sessions = store::sessions_by_import(&conn).map_err(map_row_error)?;

    let project_key = project.key();
    let context = server_context.clone();
    let mut imports = block_on(async move {
        let conn = open_main_database_read(context).await?;
        load_imports(&conn, project_key).await
    })?;
    for import in &mut imports {
        import.sessions_imported = sessions.get(&import.id).copied().unwrap_or(0);
    }
    let default_end = default_end_date(today, first_tracked)
        .ok_or(AuthError::Temporary("couldn't read the current date"))?;
    Ok(ImportsResponse {
        oauth_available: server_context.google_oauth.is_some(),
        oldest_allowed_date: oldest_allowed.map(|day| day.to_string()),
        latest_allowed_date: latest_allowed.to_string(),
        default_start_date: default_start.to_string(),
        default_end_date: default_end.to_string(),
        first_tracked_date: first_tracked.map(|day| day.to_string()),
        include_in_reports,
        imports,
    })
}

pub(super) fn load_import_view(
    server_context: Arc<ServerContext>,
    project: &AdminProject,
    import_id: i32,
) -> Result<ImportView, AuthError> {
    let project_key = project.key();
    block_on(async move {
        let conn = open_main_database_read(server_context).await?;
        let row = conn
            .query_opt(
                &format!(
                    "SELECT {IMPORT_COLUMNS} FROM ga_imports WHERE id = $1 AND project_id = $2"
                ),
                &[&import_id, &project_key],
            )
            .await
            .map_err(AuthError::from)?
            .ok_or(AuthError::NotFound("no such import"))?;
        Ok(import_view(&row))
    })
}
