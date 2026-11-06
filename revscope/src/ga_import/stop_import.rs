//! Cancelling a running import and deleting a stopped one.

use std::sync::Arc;

use revscope_api::ga_import::DeletedImport;

use crate::auth::{AuthError, map_row_error};
use crate::authorize::AdminProject;
use crate::db::{open_main_database_write, open_project_database_write};
use crate::server_context::ServerContext;
use crate::util::block_on;

use super::jobs;
use super::list_imports::{ImportView, load_import_view};
use super::store;

pub(crate) fn parse_import_id(target: &str) -> Result<i32, &'static str> {
    crate::http::query_param(target, "id")
        .and_then(|value| value.parse().ok())
        .ok_or("id is required")
}

pub(crate) fn cancel_import(
    server_context: Arc<ServerContext>,
    project: &AdminProject,
    import_id: i32,
) -> Result<ImportView, AuthError> {
    let project_key = project.key();
    let context = server_context.clone();
    block_on(async move {
        let conn = open_main_database_write(context.clone()).await?;
        let stopped = jobs::finish(&conn, import_id, project_key, "cancelled", None)
            .await
            .map_err(AuthError::from)?;
        match stopped {
            Some(Some(token)) => jobs::revoke_stranded(&context, &conn, token)
                .await
                .map_err(AuthError::from)?,
            Some(None) => {}
            None => return Err(AuthError::Conflict("this import isn't running")),
        }
        Ok::<(), AuthError>(())
    })?;
    load_import_view(server_context, project, import_id)
}

/// Removes the rows and then forgets the job, inside one Postgres transaction: a
/// failure leaves the job listed so the delete can be tried again.
pub(crate) fn delete_import(
    server_context: Arc<ServerContext>,
    project: &AdminProject,
    import_id: i32,
) -> Result<DeletedImport, AuthError> {
    let project_key = project.key();
    let project_id = project.id().to_string();
    block_on(async move {
        let conn = open_main_database_write(server_context.clone()).await?;
        conn.in_transaction(async |conn| {
            let row = conn
                .query_opt(
                    "SELECT status FROM ga_imports WHERE id = $1 AND project_id = $2 FOR UPDATE",
                    &[&import_id, &project_key],
                )
                .await?
                .ok_or(AuthError::NotFound("no such import"))?;
            let status: &str = row.get(0);
            if matches!(status, "running" | "paused") {
                return Err(AuthError::Conflict("cancel this import before deleting it"));
            }
            let project_conn = open_project_database_write(server_context.as_ref(), &project_id)?;
            store::delete_import_rows(&project_conn, import_id).map_err(map_row_error)?;
            conn.execute("DELETE FROM ga_imports WHERE id = $1", &[&import_id])
                .await?;
            Ok(())
        })
        .await
    })?;
    Ok(DeletedImport { id: import_id })
}
