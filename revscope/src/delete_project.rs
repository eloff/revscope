//! `DELETE /v2/session/projects/{id}` — deletes a project and all of its data.
//!
//! Nothing here can be undone: Postgres cascades the project's settings,
//! integrations and tokens, and the project's SQLite database is removed from
//! disk. The name stays reserved for this account (see `claim_project_name`).
//! The caller must name the project in the body as well as the path, so
//! a stray request for the wrong id can't delete anything. An account keeps at
//! least one project, since the dashboard and tracker setup assume one exists.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::auth::AuthError;
use crate::authorize::AdminProject;
use crate::db::open_main_database_write;
use crate::db::project::delete_project_database;
use crate::ga_import::{StrandedToken, revoke_stranded, take_project_tokens};
use crate::projects::lock_account_projects;
use crate::server_context::ServerContext;
use crate::user_access::lock_account_as_admin;
use crate::util::block_on;

#[derive(Debug, Deserialize)]
pub(crate) struct DeleteProjectRequest {
    /// Must match the project's name exactly.
    pub(crate) confirm_name: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct DeleteProjectResponse {
    pub(crate) deleted: String,
}

/// The project is gone by now, so a failure here can only leave a Google grant
/// standing, which its owner can still remove from their Google account.
fn revoke_project_tokens(server_context: &Arc<ServerContext>, tokens: Vec<StrandedToken>) {
    block_on(async {
        let conn = match open_main_database_write(server_context.clone()).await {
            Ok(conn) => conn,
            Err(error) => {
                log::warn!("couldn't revoke a deleted project's Google tokens: {error}");
                return;
            }
        };
        for token in tokens {
            if let Err(error) = revoke_stranded(server_context, &conn, token).await {
                log::warn!("couldn't revoke a deleted project's Google token: {error}");
            }
        }
    });
}

pub(crate) fn delete_project(
    server_context: Arc<ServerContext>,
    project: &AdminProject,
    request: DeleteProjectRequest,
) -> Result<DeleteProjectResponse, AuthError> {
    let user = project.admin().user();
    let project_key = project.key();

    let stranded = block_on(async {
        let conn = open_main_database_write(server_context.clone()).await?;
        conn.in_transaction(async |conn| {
            lock_account_as_admin(conn, &user.account_id, &user.user_id).await?;
            let account = lock_account_projects(conn, &user.account_id).await?;
            let name: String = conn
                .query_opt(
                    "SELECT name FROM projects
                     WHERE id = $1 AND account_id = $2::text::uuid",
                    &[&project_key, &user.account_id],
                )
                .await?
                .ok_or(AuthError::NotFound("project not found"))?
                .get(0);
            if request.confirm_name.trim() != name {
                return Err(AuthError::InvalidRequest(
                    "Type the project name exactly to confirm.",
                ));
            }
            if account.project_count <= 1 {
                return Err(AuthError::Conflict(
                    "This is the account's only project, so it can't be deleted.",
                ));
            }
            let stranded = take_project_tokens(conn, project_key).await?;
            conn.execute("DELETE FROM projects WHERE id = $1", &[&project_key])
                .await?;
            conn.execute(
                "INSERT INTO retired_project_names (name, account_id)
                 VALUES ($1, $2::text::uuid)
                 ON CONFLICT (name) DO UPDATE
                 SET account_id = EXCLUDED.account_id, retired_at = NOW()",
                &[&name, &user.account_id],
            )
            .await?;
            Ok(stranded)
        })
        .await
    })?;

    revoke_project_tokens(&server_context, stranded);

    // The project is already gone from Postgres, so failing here would only
    // make a retry answer 404; leave a stray file for an operator instead.
    if let Err(error) = delete_project_database(server_context.as_ref(), project.id()) {
        log::error!(
            "deleted project {} but could not remove its database: {error}",
            project.id()
        );
    }
    Ok(DeleteProjectResponse {
        deleted: project.id().to_string(),
    })
}
