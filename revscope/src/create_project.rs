//! `POST /v2/session/projects` — adds a project to the signed-in account.
//!
//! The project is named after the website the way signup names the first one,
//! and that name becomes its ingest path, so it is unique across every account.
//! Accounts on plans sold as "1 website" are held to the one they signed up with.

use std::sync::Arc;

pub(crate) use revscope_api::projects::{CreateProjectRequest, CreateProjectResponse};

use crate::auth::{AuthError, timezone_for_signup};
use crate::authorize::AdminAccount;
use crate::db::project::parse_uuid_key;
use crate::db::{insert_project, open_main_database_write};
use crate::projects::{
    PROJECT_NAME_TAKEN, check_new_project_name, claim_project_name, is_project_name_taken,
    lock_account_projects, plan_allows_multiple_projects, project_name_from_website,
};
use crate::server_context::ServerContext;
use crate::user_access::lock_account_as_admin;
use crate::util::block_on;

pub(crate) const SINGLE_PROJECT_PLAN: &str =
    "Your plan includes one website. Upgrade to Growth to add more.";

pub(crate) fn create_project(
    server_context: Arc<ServerContext>,
    admin: &AdminAccount,
    request: CreateProjectRequest,
) -> Result<CreateProjectResponse, AuthError> {
    let user = admin.user();
    let name = project_name_from_website(&request.website);
    check_new_project_name(&name)?;
    let timezone = timezone_for_signup(request.timezone.as_deref());

    let project_id = block_on(async {
        let conn = open_main_database_write(server_context.clone()).await?;
        conn.in_transaction(async |conn| {
            lock_account_as_admin(conn, &user.account_id, &user.user_id).await?;
            let account = lock_account_projects(conn, &user.account_id).await?;
            if account.project_count > 0 && !plan_allows_multiple_projects(&account.plan) {
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
