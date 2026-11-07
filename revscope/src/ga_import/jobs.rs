//! Import jobs in the main database.

use jiff::civil::Date;

use crate::db::main::Connection;

use crate::server_context::ServerContext;

use super::api;
use super::credential::GaCredential;

const CREDENTIAL_COLUMNS: &str = "encrypted_client_email, encrypted_secret, oauth_subject";
/// Both are picked up once `resume_at` passes, so `paused` only tells the UI the
/// wait is for GA's quota. `running` can also be waiting, on a retry or on settings.
const ACTIVE: &str = "('running', 'paused')";

#[derive(Debug, Clone)]
pub(super) struct DueImport {
    pub(super) id: i32,
    pub(super) project_id: i32,
    pub(super) property_id: String,
    pub(super) start: Date,
    pub(super) end: Date,
    pub(super) hostnames: Vec<String>,
    pub(super) credential: Option<GaCredential>,
    pub(super) chunks_done: i32,
    pub(super) failed_attempts: i32,
}

/// Imports whose next chunk may run, least recently attempted first, so one big
/// import can't starve the others.
pub(super) async fn load_due_imports(
    conn: &Connection,
    limit: i64,
) -> Result<Vec<DueImport>, tokio_postgres::Error> {
    let rows = conn
        .query(
            &format!(
                "SELECT id, project_id, property_id, start_date::text, end_date::text, hostnames,
                        encrypted_client_email, encrypted_secret, chunks_done, failed_attempts
                 FROM ga_imports
                 WHERE status IN {ACTIVE} AND resume_at <= NOW()
                 ORDER BY resume_at
                 LIMIT $1"
            ),
            &[&limit],
        )
        .await?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            let start: &str = row.get(3);
            let end: &str = row.get(4);
            let email: Option<&[u8]> = row.get(6);
            let secret: Option<&[u8]> = row.get(7);
            Some(DueImport {
                id: row.get(0),
                project_id: row.get(1),
                property_id: row.get(2),
                start: start.parse().ok()?,
                end: end.parse().ok()?,
                hostnames: row.get(5),
                credential: secret.and_then(|secret| GaCredential::decrypt(email, secret)),
                chunks_done: row.get(8),
                failed_attempts: row.get(9),
            })
        })
        .collect())
}

/// Moves past a finished chunk. Conditional on the import still being active at
/// this position, so a cancel that landed mid-chunk stays cancelled.
pub(super) async fn advance(
    conn: &Connection,
    import_id: i32,
    chunks_done: i32,
) -> Result<bool, tokio_postgres::Error> {
    let updated = conn
        .execute(
            &format!(
                "UPDATE ga_imports
                 SET chunks_done = $2 + 1,
                     failed_attempts = 0,
                     status = 'running',
                     resume_at = NOW(),
                     error_text = NULL
                 WHERE id = $1 AND chunks_done = $2 AND status IN {ACTIVE}"
            ),
            &[&import_id, &chunks_done],
        )
        .await?;
    Ok(updated > 0)
}

/// A refresh token that has been taken out of storage, still to be revoked.
#[derive(Debug)]
pub(crate) struct StrandedToken {
    refresh_token: String,
    subject: Option<String>,
}

fn stranded_token(row: &tokio_postgres::Row) -> Option<StrandedToken> {
    let email: Option<&[u8]> = row.get(0);
    let secret: Option<&[u8]> = row.get(1);
    match GaCredential::decrypt(email, secret?)? {
        GaCredential::OAuth { refresh_token } => Some(StrandedToken {
            refresh_token,
            subject: row.get(2),
        }),
        GaCredential::ServiceAccount { .. } => None,
    }
}

/// Revokes a token we're done with, unless the same Google account has another
/// credential still in use: revoking ends the account's whole grant to our client,
/// which would fail its other imports. A token whose account we don't know is left
/// to the user to remove from their Google account, for the same reason.
pub(crate) async fn revoke_stranded(
    server_context: &ServerContext,
    conn: &Connection,
    token: StrandedToken,
) -> Result<(), tokio_postgres::Error> {
    let Some(subject) = token.subject else {
        return Ok(());
    };
    let in_use: bool = conn
        .query_one(
            "SELECT EXISTS (
                SELECT 1 FROM ga_import_connections
                WHERE oauth_subject = $1 AND encrypted_secret IS NOT NULL
             ) OR EXISTS (
                SELECT 1 FROM ga_imports
                WHERE oauth_subject = $1 AND encrypted_secret IS NOT NULL
             )",
            &[&subject],
        )
        .await?
        .get(0);
    if !in_use {
        api::revoke_refresh_token(server_context, &token.refresh_token).await;
    }
    Ok(())
}

/// Takes every credential a project holds, for a delete that's about to cascade
/// them away. Call inside the transaction that deletes the project.
pub(crate) async fn take_project_tokens(
    conn: &Connection,
    project_id: i32,
) -> Result<Vec<StrandedToken>, tokio_postgres::Error> {
    let mut tokens = Vec::new();
    for table in ["ga_import_connections", "ga_imports"] {
        let rows = conn
            .query(
                &format!("SELECT {CREDENTIAL_COLUMNS} FROM {table} WHERE project_id = $1"),
                &[&project_id],
            )
            .await?;
        tokens.extend(rows.iter().filter_map(stranded_token));
    }
    Ok(tokens)
}

/// Stops an import for good and clears its credentials, returning a refresh token
/// to revoke if it held one. `None` when it had already stopped.
pub(super) async fn finish(
    conn: &Connection,
    import_id: i32,
    project_id: i32,
    status: &str,
    error_text: Option<&str>,
) -> Result<Option<Option<StrandedToken>>, tokio_postgres::Error> {
    let row = conn
        .query_opt(
            &format!(
                "WITH stopping AS (
                    SELECT id, {CREDENTIAL_COLUMNS}
                    FROM ga_imports
                    WHERE id = $1 AND project_id = $2 AND status IN {ACTIVE}
                    FOR UPDATE
                 )
                 UPDATE ga_imports g
                 SET status = $3,
                     error_text = $4,
                     finished_at = NOW(),
                     encrypted_client_email = NULL,
                     encrypted_secret = NULL
                 FROM stopping
                 WHERE g.id = stopping.id
                 RETURNING stopping.encrypted_client_email, stopping.encrypted_secret,
                           stopping.oauth_subject"
            ),
            &[&import_id, &project_id, &status, &error_text],
        )
        .await?;
    Ok(row.map(|row| stranded_token(&row)))
}

/// Out of quota: waits for GA's hourly refill.
pub(super) async fn pause_until_next_hour(
    conn: &Connection,
    import_id: i32,
) -> Result<(), tokio_postgres::Error> {
    conn.execute(
        &format!(
            "UPDATE ga_imports
             SET status = 'paused',
                 resume_at = date_trunc('hour', NOW()) + INTERVAL '1 hour',
                 error_text = NULL
             WHERE id = $1 AND status IN {ACTIVE}"
        ),
        &[&import_id],
    )
    .await?;
    Ok(())
}

/// Waits for an operator to fix the OAuth client settings. Unlike [`retry_later`]
/// this never gives up, since the import's credential is fine.
pub(super) async fn wait_for_settings(
    conn: &Connection,
    import_id: i32,
    delay_seconds: i64,
    error_text: &str,
) -> Result<(), tokio_postgres::Error> {
    conn.execute(
        &format!(
            "UPDATE ga_imports
             SET status = 'running',
                 resume_at = NOW() + make_interval(secs => $2::bigint::double precision),
                 error_text = $3
             WHERE id = $1 AND status IN {ACTIVE}"
        ),
        &[&import_id, &delay_seconds, &error_text],
    )
    .await?;
    Ok(())
}

pub(super) async fn retry_later(
    conn: &Connection,
    import_id: i32,
    delay_seconds: i64,
    error_text: &str,
) -> Result<(), tokio_postgres::Error> {
    conn.execute(
        &format!(
            "UPDATE ga_imports
             SET failed_attempts = failed_attempts + 1,
                 status = 'running',
                 resume_at = NOW() + make_interval(secs => $2::bigint::double precision),
                 error_text = $3
             WHERE id = $1 AND status IN {ACTIVE}"
        ),
        &[&import_id, &delay_seconds, &error_text],
    )
    .await?;
    Ok(())
}

/// Takes the expired connections, returning their refresh tokens for revoking.
pub(super) async fn take_expired_connections(
    conn: &Connection,
) -> Result<Vec<StrandedToken>, tokio_postgres::Error> {
    let rows = conn
        .query(
            &format!(
                "DELETE FROM ga_import_connections
                 WHERE expires_at <= NOW()
                 RETURNING {CREDENTIAL_COLUMNS}"
            ),
            &[],
        )
        .await?;
    Ok(rows.iter().filter_map(stranded_token).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pausing_for_quota_clears_the_last_attempts_error() {
        let (_server_context, conn) = crate::test_utils::main_test_db("ga-pause").await;
        let account_id = crate::test_utils::insert_test_account(&conn, "ga_pause").await;
        let project_id: i32 = conn
            .query_one(
                "INSERT INTO projects (account_id, name) VALUES ($1::text::uuid, $2) RETURNING id",
                &[&account_id, &format!("p{}", account_id.replace('-', ""))],
            )
            .await
            .expect("insert project")
            .get(0);
        let import_id: i32 = conn
            .query_one(
                "INSERT INTO ga_imports
                    (project_id, property_id, property_name, start_date, end_date, chunks_total)
                 VALUES ($1, 'p', 'Property', '2026-01-01', '2026-01-31', 1)
                 RETURNING id",
                &[&project_id],
            )
            .await
            .expect("insert import")
            .get(0);
        retry_later(&conn, import_id, 60, "GA was unavailable")
            .await
            .expect("record a failed attempt");

        pause_until_next_hour(&conn, import_id)
            .await
            .expect("pause");

        let row = conn
            .query_one(
                "SELECT status, error_text FROM ga_imports WHERE id = $1",
                &[&import_id],
            )
            .await
            .expect("load import");
        let (status, error): (String, Option<String>) = (row.get(0), row.get(1));
        assert_eq!(status, "paused");
        assert_eq!(error, None);
        conn.execute(
            "DELETE FROM accounts WHERE id = $1::text::uuid",
            &[&account_id],
        )
        .await
        .expect("delete account");
    }
}
