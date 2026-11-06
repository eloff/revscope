//! The project's choice of whether reports include imported data.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::auth::{AuthError, map_row_error};
use crate::authorize::AdminProject;
use crate::db::open_project_database_write;
use crate::server_context::ServerContext;

use super::store;

#[derive(Deserialize, Serialize)]
pub(crate) struct ImportedDataSettings {
    include_in_reports: bool,
}

pub(crate) fn save_settings(
    server_context: Arc<ServerContext>,
    project: &AdminProject,
    settings: ImportedDataSettings,
) -> Result<ImportedDataSettings, AuthError> {
    let conn = open_project_database_write(server_context.as_ref(), project.id())?;
    store::set_include_in_reports(&conn, settings.include_in_reports).map_err(map_row_error)?;
    Ok(settings)
}
