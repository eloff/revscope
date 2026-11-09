//! `POST /v2/session/projects` — adds a project to the signed-in account.
//!
//! The project gets the name sent, or else one derived from the website, the
//! way signup names the first one. That name becomes its ingest path, so it is unique across every account.
//! Accounts on plans sold as "1 website", and accounts that picked a larger plan
//! but aren't paying for it, are held to the one they signed up with.

use std::sync::Arc;

pub(crate) use revscope_api::projects::{CreateProjectRequest, CreateProjectResponse};

use crate::auth::{AuthError, timezone_for_signup};
use crate::authorize::AdminAccount;
use crate::db::project::parse_uuid_key;
use crate::db::{insert_project, open_main_database_write};
use crate::projects::{
    PROJECT_NAME_TAKEN, claim_project_name, is_project_name_taken, lock_account_projects,
    new_project_name, plan_allows_multiple_projects,
};
use crate::server_context::ServerContext;
use crate::user_access::lock_account_as_admin;
use crate::util::block_on;

pub(crate) const SINGLE_PROJECT_PLAN: &str = "Adding more websites needs an active Growth plan.";

pub(crate) fn create_project(
    server_context: Arc<ServerContext>,
    admin: &AdminAccount,
    request: CreateProjectRequest,
) -> Result<CreateProjectResponse, AuthError> {
    let user = admin.user();
    let name = new_project_name(&request.website, request.project_name.as_deref())?;
    let timezone = timezone_for_signup(request.timezone.as_deref());

    let project_id = block_on(async {
        let conn = open_main_database_write(server_context.clone()).await?;
        conn.in_transaction(async |conn| {
            lock_account_as_admin(conn, &user.account_id, &user.user_id).await?;
            let account = lock_account_projects(conn, &user.account_id).await?;
            if account.project_count > 0
                && !plan_allows_multiple_projects(
                    &account.plan,
                    account.subscription_status.as_deref(),
                )
            {
                return Err(AuthError::Forbidden(SINGLE_PROJECT_PLAN));
            }
            let project_id: i32 = conn
                .query_one(
                    "INSERT INTO projects (account_id, name)
                     VALUES ($1::text::uuid, $2)
                     RETURNING id",
                    &[&user.account_id, &name],
                )
                .await
                .map_err(map_project_insert_error)?
                .get(0);
            claim_project_name(conn, &name, &user.account_id).await?;
            conn.execute(
                "INSERT INTO project_settings (project_id, timezone) VALUES ($1, $2)",
                &[&project_id, &timezone],
            )
            .await?;
            Ok(project_id)
        })
        .await
    })?;

    // The transaction committed, so the project exists whatever happens here;
    // a cache miss later would load the same identity from Postgres.
    if let Some(account_key) = parse_uuid_key(&user.account_id) {
        insert_project(
            server_context.as_ref(),
            project_id,
            name.clone(),
            account_key,
        );
    }
    Ok(CreateProjectResponse {
        id: project_id.to_string(),
        name,
    })
}

fn map_project_insert_error(error: tokio_postgres::Error) -> AuthError {
    if is_project_name_taken(&error) {
        return AuthError::Conflict(PROJECT_NAME_TAKEN);
    }
    error.into()
}
