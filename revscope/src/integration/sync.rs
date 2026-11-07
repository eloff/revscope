use std::error::Error as StdError;
use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use serde::Serialize;
use serde_json::Value;
use tokio_postgres::types::ToSql;

use super::cloudflare;
use super::dodo;
use super::gsc;
use super::lemon_squeezy;
use super::polar;
use super::shopify;
use super::square;
use super::stripe;
use crate::db::{
    DbError, PostgresError, ProjectCache, open_main_database_read, open_main_database_write,
    parse_project_id,
};
use crate::sentry_report::report_error;
use crate::server_context::ServerContext;

const SYNC_IDLE_INTERVAL: Duration = Duration::from_secs(10);

/// Load project identity into [`ServerContext::project_cache`] before SQLite opens.
pub(crate) async fn ensure_project_cached(
    server_context: Arc<ServerContext>,
    project_id: &str,
) -> SyncResult<Arc<ProjectCache>> {
    let project_key = parse_project_id(project_id)?;
    crate::projects::fetch_project(server_context, project_key)
        .await?
        .ok_or_else(|| SyncError::Db(DbError::ProjectDatabaseNotFound(project_id.to_string())))
}

pub fn start_integration_sync_task(server_context: Arc<ServerContext>) {
    crate::task::spawn(server_context.clone(), async move {
        run_integration_sync_loop(server_context).await;
    });
}

async fn run_integration_sync_loop(server_context: Arc<ServerContext>) {
    while !server_context.shutdown_requested() {
        match sync_due_integrations(server_context.clone()).await {
            Ok(true) => {}
            Ok(false) => {
                if sleep_or_shutdown(&server_context, SYNC_IDLE_INTERVAL).await {
                    break;
                }
            }
            Err(error) => {
                report_error!(
                    "integration_sync.scheduler",
                    "integration sync scheduler failed: {error}"
                );
                if sleep_or_shutdown(&server_context, SYNC_IDLE_INTERVAL).await {
                    break;
                }
            }
        }
    }
}

async fn sync_due_integrations(server_context: Arc<ServerContext>) -> SyncResult<bool> {
    let stripe_integrations = stripe::load_due_integrations(server_context.clone()).await?;
    let shopify_integrations = shopify::load_due_integrations(server_context.clone()).await?;
    let square_integrations = square::load_due_integrations(server_context.clone()).await?;
    let lemon_squeezy_integrations =
        lemon_squeezy::load_due_integrations(server_context.clone()).await?;
    let gsc_integrations = gsc::load_due_integrations(server_context.clone()).await?;
    let cloudflare_integrations = cloudflare::load_due_integrations(server_context.clone()).await?;
    let dodo_integrations = dodo::load_due_integrations(server_context.clone()).await?;
    let polar_integrations = polar::load_due_integrations(server_context.clone()).await?;
    let did_work = !stripe_integrations.is_empty()
        || !shopify_integrations.is_empty()
        || !square_integrations.is_empty()
        || !lemon_squeezy_integrations.is_empty()
        || !gsc_integrations.is_empty()
        || !cloudflare_integrations.is_empty()
        || !dodo_integrations.is_empty()
        || !polar_integrations.is_empty();
    // `&&` skips the remaining providers once shutdown cuts one short.
    let _ = sync_each(
        &server_context,
        stripe_integrations,
        stripe::run_sync_with_retries,
    )
    .await
        && sync_each(
            &server_context,
            shopify_integrations,
            shopify::run_sync_with_retries,
        )
        .await
        && sync_each(
            &server_context,
            square_integrations,
            square::run_sync_with_retries,
        )
        .await
        && sync_each(
            &server_context,
            lemon_squeezy_integrations,
            lemon_squeezy::run_sync_with_retries,
        )
        .await
        && sync_each(
            &server_context,
            gsc_integrations,
            gsc::run_sync_with_retries,
        )
        .await
        && sync_each(
            &server_context,
            cloudflare_integrations,
            cloudflare::run_sync_with_retries,
        )
        .await
        && sync_each(
            &server_context,
            dodo_integrations,
            dodo::run_sync_with_retries,
        )
        .await
        && sync_each(
            &server_context,
            polar_integrations,
            polar::run_sync_with_retries,
        )
        .await;
    Ok(did_work)
}

/// Syncs each integration in turn, returning false if shutdown cut it short.
async fn sync_each<T, Fut>(
    server_context: &Arc<ServerContext>,
    integrations: Vec<T>,
    run_sync: impl Fn(Arc<ServerContext>, T) -> Fut,
) -> bool
where
    Fut: Future<Output = ()>,
{
    for integration in integrations {
        if server_context.shutdown_requested() {
            return false;
        }
        run_sync(server_context.clone(), integration).await;
    }
    true
}

/// Records the start of a sync, or returns `None` without recording anything if
/// the credentials changed since `credentials_version` was read.
pub(crate) async fn create_integration_sync_row(
    server_context: Arc<ServerContext>,
    integration_id: i32,
    credentials_version: i64,
    sync_kind: SyncKind,
) -> SyncResult<Option<SystemTime>> {
    let conn = open_main_database_write(server_context).await?;
    let row = conn
        .query_opt(
            "INSERT INTO integration_syncs (integration_id, sync_kind, credentials_version)
             SELECT id, $2, credentials_version
             FROM integrations
             WHERE id = $1 AND credentials_version = $3
             RETURNING started_at",
            &[&integration_id, &sync_kind.as_str(), &credentials_version],
        )
        .await?;
    Ok(row.map(|row| row.get(0)))
}

/// Whether the integration still has the credentials a sync started with. A
/// sync that fails to find out carries on, since the final write checks again.
pub(crate) async fn credentials_are_current(
    server_context: Arc<ServerContext>,
    integration_id: i32,
    credentials_version: i64,
) -> bool {
    let current = async {
        let conn = open_main_database_read(server_context).await?;
        let row = conn
            .query_opt(
                "SELECT 1 FROM integrations WHERE id = $1 AND credentials_version = $2",
                &[&integration_id, &credentials_version],
            )
            .await?;
        SyncResult::Ok(row.is_some())
    };
    current.await.unwrap_or(true)
}

/// Records the outcome of a sync attempt and updates the denormalized scheduler
/// state on `integrations` (`last_sync_completed_at`, and on success
/// `last_sync_success_at` / `last_full_sync_success_at`) in a single statement.
///
/// `sync_kind` is the kind the run actually achieved, which can be narrower than
/// the kind it started as: a provider that only covered part of a full window
/// reports [`SyncKind::Incremental`] so `last_full_sync_success_at` stays put and
/// the run is retried on the throttle interval rather than a day later.
///
/// If the credentials changed since `credentials_version` was read, only the
/// `integration_syncs` row is updated. The scheduler state now belongs to the new
/// credentials, and restoring it would delay their first sync.
pub(crate) async fn complete_integration_sync(
    server_context: Arc<ServerContext>,
    integration_id: i32,
    credentials_version: i64,
    sync_kind: SyncKind,
    started_at: SystemTime,
    counts: SyncCounts,
    error_text: Option<&str>,
) -> SyncResult<()> {
    let conn = open_main_database_write(server_context).await?;
    let params: [&(dyn ToSql + Sync); 8] = [
        &integration_id,
        &started_at,
        &counts.records_inserted,
        &counts.records_updated,
        &counts.records_deleted,
        &error_text,
        &sync_kind.as_str(),
        &credentials_version,
    ];
    conn.execute(
        "WITH updated_sync AS (
            UPDATE integration_syncs
            SET completed_at = NOW(),
                sync_kind = $7,
                records_inserted = $3,
                records_updated = $4,
                records_deleted = $5,
                error_text = $6
            WHERE integration_id = $1 AND started_at = $2
            RETURNING integration_id
         )
         UPDATE integrations SET
            last_sync_completed_at = NOW(),
            last_sync_success_at = CASE WHEN $6 IS NULL THEN $2 ELSE last_sync_success_at END,
            last_full_sync_success_at = CASE
                WHEN $6 IS NULL AND $7 = 'full' THEN $2
                ELSE last_full_sync_success_at
            END
         WHERE id = (SELECT integration_id FROM updated_sync)
            AND credentials_version = $8",
        &params,
    )
    .await?;
    Ok(())
}

pub(crate) async fn sleep_or_shutdown(server_context: &ServerContext, duration: Duration) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(duration) => false,
        _ = server_context.wait_for_shutdown() => true,
    }
}

pub(crate) fn serialize_metadata<T: Serialize>(metadata: &T) -> SyncResult<Option<String>> {
    let value = serde_json::to_value(metadata)?;
    match &value {
        Value::Null => Ok(None),
        Value::Object(values) if values.is_empty() => Ok(None),
        Value::Array(values) if values.is_empty() => Ok(None),
        _ => Ok(Some(serde_json::to_string(&value)?)),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SyncKind {
    Incremental,
    Full,
}

impl SyncKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Incremental => "incremental",
            Self::Full => "full",
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SyncCounts {
    pub(crate) records_inserted: i32,
    pub(crate) records_updated: i32,
    pub(crate) records_deleted: i32,
}

pub(crate) type SyncResult<T> = Result<T, SyncError>;

#[derive(Debug)]
pub(crate) enum SyncError {
    Db(crate::db::DbError),
    MainDb(PostgresError),
    ProjectDb(rusqlite::Error),
    Stripe(::stripe::StripeError),
    Dodo(::dodopayments::Error),
    Http(reqwest::Error),
    Json(serde_json::Error),
    MissingStripeSecret,
    MissingDodoSecret,
    MissingPolarSecret,
    MissingLemonSqueezySecret,
    InvalidStoredSecret,
    InvalidPolarSecret,
    InvalidLemonSqueezySecret,
    MissingShopifyCredential(&'static str),
    InvalidShopifyCredential(&'static str),
    Shopify(String),
    MissingSquareSecret,
    InvalidSquareSecret,
    Square(String),
    MissingGscCredential(&'static str),
    InvalidGscCredential(&'static str),
    Gsc(String),
    MissingCloudflareCredential(&'static str),
    InvalidCloudflareCredential(&'static str),
    Cloudflare(String),
    Polar(String),
    LemonSqueezy(String),
    InvalidProviderData(String),
    MissingUpsertedRow(&'static str),
    CredentialsChanged,
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Db(error) => write!(f, "{error}"),
            Self::MainDb(error) => write!(f, "{error}"),
            Self::ProjectDb(error) => write!(f, "{error}"),
            Self::Stripe(error) => write!(f, "{error}"),
            Self::Dodo(error) => write!(f, "{error}"),
            Self::Http(error) => write!(f, "{error}"),
            Self::Json(error) => write!(f, "{error}"),
            Self::MissingStripeSecret => write!(f, "stripe integration has no stored secret key"),
            Self::MissingDodoSecret => write!(f, "Dodo integration has no stored API key"),
            Self::MissingPolarSecret => write!(f, "Polar integration has no stored access token"),
            Self::MissingLemonSqueezySecret => {
                write!(f, "Lemon Squeezy integration has no stored API key")
            }
            Self::InvalidStoredSecret => {
                write!(f, "integration secret could not be decrypted")
            }
            Self::InvalidPolarSecret => {
                write!(f, "Polar integration access token could not be decrypted")
            }
            Self::InvalidLemonSqueezySecret => {
                write!(
                    f,
                    "Lemon Squeezy integration API key could not be decrypted"
                )
            }
            Self::MissingShopifyCredential(label) => {
                write!(f, "Shopify integration has no stored {label}")
            }
            Self::InvalidShopifyCredential(label) => {
                write!(f, "Shopify integration {label} could not be decrypted")
            }
            Self::Shopify(error) => write!(f, "{error}"),
            Self::MissingSquareSecret => write!(f, "Square integration has no stored access token"),
            Self::InvalidSquareSecret => {
                write!(f, "Square integration access token could not be decrypted")
            }
            Self::Square(error) => write!(f, "{error}"),
            Self::MissingGscCredential(label) => {
                write!(f, "GSC integration has no stored {label}")
            }
            Self::InvalidGscCredential(label) => {
                write!(f, "GSC integration {label} could not be decrypted")
            }
            Self::Gsc(error) => write!(f, "{error}"),
            Self::MissingCloudflareCredential(label) => {
                write!(f, "Cloudflare integration has no stored {label}")
            }
            Self::InvalidCloudflareCredential(label) => {
                write!(f, "Cloudflare integration {label} could not be decrypted")
            }
            Self::Cloudflare(error) => write!(f, "{error}"),
            Self::Polar(error) => write!(f, "{error}"),
            Self::LemonSqueezy(error) => write!(f, "{error}"),
            Self::InvalidProviderData(error) => write!(f, "{error}"),
            Self::MissingUpsertedRow(table) => write!(f, "upserted row was missing from {table}"),
            Self::CredentialsChanged => write!(f, "the credentials changed during this sync"),
        }
    }
}

impl StdError for SyncError {}

impl From<crate::db::DbError> for SyncError {
    fn from(error: crate::db::DbError) -> Self {
        Self::Db(error)
    }
}

impl From<tokio_postgres::Error> for SyncError {
    fn from(error: tokio_postgres::Error) -> Self {
        Self::MainDb(error.into())
    }
}

impl From<PostgresError> for SyncError {
    fn from(error: PostgresError) -> Self {
        Self::MainDb(error)
    }
}

impl From<rusqlite::Error> for SyncError {
    fn from(error: rusqlite::Error) -> Self {
        Self::ProjectDb(error)
    }
}

impl From<::stripe::StripeError> for SyncError {
    fn from(error: ::stripe::StripeError) -> Self {
        Self::Stripe(error)
    }
}

impl From<::dodopayments::Error> for SyncError {
    fn from(error: ::dodopayments::Error) -> Self {
        Self::Dodo(error)
    }
}

impl From<reqwest::Error> for SyncError {
    fn from(error: reqwest::Error) -> Self {
        Self::Http(error)
    }
}

impl From<serde_json::Error> for SyncError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_metadata_is_stored_as_null() {
        assert_eq!(serialize_metadata(&Value::Null).unwrap(), None);
        assert_eq!(serialize_metadata(&None::<Value>).unwrap(), None);
        assert_eq!(serialize_metadata(&json!({})).unwrap(), None);
        assert_eq!(serialize_metadata(&json!([])).unwrap(), None);
    }

    #[test]
    fn non_empty_metadata_is_stored_as_json() {
        assert_eq!(
            serialize_metadata(&json!({ "plan": "pro" })).unwrap(),
            Some(r#"{"plan":"pro"}"#.to_string())
        );
        assert_eq!(
            serialize_metadata(&json!(["a"])).unwrap(),
            Some(r#"["a"]"#.to_string())
        );
        // A scalar is unusual but still kept.
        assert_eq!(
            serialize_metadata(&json!("note")).unwrap(),
            Some(r#""note""#.to_string())
        );
    }

    /// The scheduler state `complete_integration_sync` maintains on `integrations`.
    #[derive(Debug, PartialEq)]
    struct SchedulerState {
        completed_at: Option<SystemTime>,
        success_at: Option<SystemTime>,
        full_success_at: Option<SystemTime>,
    }

    /// One `integration_syncs` row's outcome.
    #[derive(Debug, PartialEq)]
    struct SyncRecord {
        completed: bool,
        sync_kind: String,
        records: (i32, i32, i32),
        error_text: Option<String>,
    }

    /// A Stripe integration on a fresh account and project. Returns the
    /// account id, for cleanup, and the integration id.
    async fn insert_integration(conn: &crate::db::main::Connection, tag: &str) -> (String, i32) {
        let account_id = crate::test_utils::insert_test_account(conn, tag).await;
        let project_name = format!("p{}", account_id.replace('-', ""));
        let project_id: i32 = conn
            .query_one(
                "INSERT INTO projects (account_id, name) VALUES ($1::text::uuid, $2)
                 RETURNING id",
                &[&account_id, &project_name],
            )
            .await
            .expect("insert project")
            .get(0);
        let integration_id: i32 = conn
            .query_one(
                "INSERT INTO integrations (project_id, integration_type)
                 VALUES ($1, 'stripe') RETURNING id",
                &[&project_id],
            )
            .await
            .expect("insert integration")
            .get(0);
        (account_id, integration_id)
    }

    async fn scheduler_state(
        conn: &crate::db::main::Connection,
        integration_id: i32,
    ) -> SchedulerState {
        let row = conn
            .query_one(
                "SELECT last_sync_completed_at, last_sync_success_at, last_full_sync_success_at
                 FROM integrations WHERE id = $1",
                &[&integration_id],
            )
            .await
            .expect("load integration");
        SchedulerState {
            completed_at: row.get(0),
            success_at: row.get(1),
            full_success_at: row.get(2),
        }
    }

    async fn sync_record(
        conn: &crate::db::main::Connection,
        integration_id: i32,
        started_at: SystemTime,
    ) -> SyncRecord {
        let row = conn
            .query_one(
                "SELECT completed_at IS NOT NULL, sync_kind,
                        records_inserted, records_updated, records_deleted, error_text
                 FROM integration_syncs WHERE integration_id = $1 AND started_at = $2",
                &[&integration_id, &started_at],
            )
            .await
            .expect("load sync");
        SyncRecord {
            completed: row.get(0),
            sync_kind: row.get(1),
            records: (row.get(2), row.get(3), row.get(4)),
            error_text: row.get(5),
        }
    }

    async fn delete_account(conn: &crate::db::main::Connection, account_id: &str) {
        conn.execute(
            "DELETE FROM accounts WHERE id = $1::text::uuid",
            &[&account_id],
        )
        .await
        .expect("delete account");
    }

    async fn run_sync(
        server_context: &Arc<ServerContext>,
        integration_id: i32,
        started_as: SyncKind,
        completed_as: SyncKind,
        error_text: Option<&str>,
    ) -> SystemTime {
        let started_at =
            create_integration_sync_row(server_context.clone(), integration_id, 0, started_as)
                .await
                .expect("start sync")
                .expect("credentials are current");
        let counts = SyncCounts {
            records_inserted: 3,
            records_updated: 2,
            records_deleted: 1,
        };
        complete_integration_sync(
            server_context.clone(),
            integration_id,
            0,
            completed_as,
            started_at,
            counts,
            error_text,
        )
        .await
        .expect("complete sync");
        started_at
    }

    #[tokio::test]
    async fn a_successful_full_sync_advances_both_success_cursors() {
        let (server_context, conn) = crate::test_utils::main_test_db("sync-full").await;
        let (account_id, integration_id) = insert_integration(&conn, "sync_full").await;

        let started_at = run_sync(
            &server_context,
            integration_id,
            SyncKind::Full,
            SyncKind::Full,
            None,
        )
        .await;

        assert_eq!(
            sync_record(&conn, integration_id, started_at).await,
            SyncRecord {
                completed: true,
                sync_kind: "full".to_string(),
                records: (3, 2, 1),
                error_text: None,
            }
        );
        let state = scheduler_state(&conn, integration_id).await;
        assert!(state.completed_at.is_some());
        assert_eq!(state.success_at, Some(started_at));
        assert_eq!(state.full_success_at, Some(started_at));
        delete_account(&conn, &account_id).await;
    }

    #[tokio::test]
    async fn a_full_sync_that_covered_only_part_of_its_window_leaves_the_full_cursor() {
        let (server_context, conn) = crate::test_utils::main_test_db("sync-narrowed").await;
        let (account_id, integration_id) = insert_integration(&conn, "sync_narrowed").await;
        let full_at = run_sync(
            &server_context,
            integration_id,
            SyncKind::Full,
            SyncKind::Full,
            None,
        )
        .await;

        let narrowed_at = run_sync(
            &server_context,
            integration_id,
            SyncKind::Full,
            SyncKind::Incremental,
            None,
        )
        .await;

        assert_eq!(
            sync_record(&conn, integration_id, narrowed_at)
                .await
                .sync_kind,
            "incremental"
        );
        let state = scheduler_state(&conn, integration_id).await;
        assert_eq!(state.success_at, Some(narrowed_at));
        assert_eq!(state.full_success_at, Some(full_at));
        delete_account(&conn, &account_id).await;
    }

    #[tokio::test]
    async fn a_failed_sync_records_its_error_and_keeps_both_success_cursors() {
        let (server_context, conn) = crate::test_utils::main_test_db("sync-failed").await;
        let (account_id, integration_id) = insert_integration(&conn, "sync_failed").await;
        let full_at = run_sync(
            &server_context,
            integration_id,
            SyncKind::Full,
            SyncKind::Full,
            None,
        )
        .await;
        let before = scheduler_state(&conn, integration_id).await;

        let failed_at = run_sync(
            &server_context,
            integration_id,
            SyncKind::Full,
            SyncKind::Full,
            Some("provider said no"),
        )
        .await;

        let record = sync_record(&conn, integration_id, failed_at).await;
        assert!(record.completed);
        assert_eq!(record.error_text.as_deref(), Some("provider said no"));
        let after = scheduler_state(&conn, integration_id).await;
        assert!(
            after.completed_at > before.completed_at,
            "a failure still throttles the next attempt"
        );
        assert_eq!(after.success_at, Some(full_at));
        assert_eq!(after.full_success_at, Some(full_at));
        delete_account(&conn, &account_id).await;
    }

    async fn save_new_stripe_key(
        conn: &crate::db::main::Connection,
        integration_id: i32,
        key: &str,
    ) {
        let project_id: i32 = conn
            .query_one(
                "SELECT project_id FROM integrations WHERE id = $1",
                &[&integration_id],
            )
            .await
            .expect("load integration")
            .get(0);
        crate::settings::upsert_integration(
            conn,
            project_id,
            &revscope_api::settings::IntegrationInput {
                integration_type: "stripe".to_string(),
                external_account_id: None,
                api_key: Some(key.to_string()),
                secret: None,
            },
        )
        .await
        .expect("save new credential");
    }

    #[tokio::test]
    async fn a_sync_finishing_after_a_credential_change_leaves_the_new_credential_due() {
        let (server_context, conn) = crate::test_utils::main_test_db("credentials-race").await;
        let (account_id, integration_id) = insert_integration(&conn, "credentials_race").await;
        run_sync(
            &server_context,
            integration_id,
            SyncKind::Full,
            SyncKind::Full,
            None,
        )
        .await;
        let started_at =
            create_integration_sync_row(server_context.clone(), integration_id, 0, SyncKind::Full)
                .await
                .expect("start sync")
                .expect("credentials are current");

        save_new_stripe_key(&conn, integration_id, "sk_new").await;
        complete_integration_sync(
            server_context.clone(),
            integration_id,
            0,
            SyncKind::Full,
            started_at,
            SyncCounts::default(),
            None,
        )
        .await
        .expect("complete sync");

        assert_eq!(
            scheduler_state(&conn, integration_id).await,
            SchedulerState {
                completed_at: None,
                success_at: None,
                full_success_at: None,
            }
        );
        let still_running: i64 = conn
            .query_one(
                "SELECT COUNT(*) FROM integration_syncs
                 WHERE integration_id = $1 AND completed_at IS NULL",
                &[&integration_id],
            )
            .await
            .expect("count open sync rows")
            .get(0);
        assert_eq!(still_running, 0, "the sync's own record should be closed");
        delete_account(&conn, &account_id).await;
    }

    #[tokio::test]
    async fn the_reported_status_covers_only_the_current_credential() {
        let (server_context, conn) = crate::test_utils::main_test_db("credentials-status").await;
        let (account_id, integration_id) = insert_integration(&conn, "credentials_status").await;
        save_new_stripe_key(&conn, integration_id, "sk_first").await;
        let row = conn
            .query_one(
                "SELECT project_id, credentials_version FROM integrations WHERE id = $1",
                &[&integration_id],
            )
            .await
            .expect("load integration");
        let (project_id, version): (i32, i64) = (row.get(0), row.get(1));
        assert!(version > 0);
        let succeeded_at = create_integration_sync_row(
            server_context.clone(),
            integration_id,
            version,
            SyncKind::Full,
        )
        .await
        .expect("start sync")
        .expect("credentials are current");
        complete_integration_sync(
            server_context.clone(),
            integration_id,
            version,
            SyncKind::Full,
            succeeded_at,
            SyncCounts::default(),
            None,
        )
        .await
        .expect("complete sync");
        let failed_at = create_integration_sync_row(
            server_context.clone(),
            integration_id,
            version,
            SyncKind::Full,
        )
        .await
        .expect("start sync")
        .expect("credentials are current");
        complete_integration_sync(
            server_context.clone(),
            integration_id,
            version,
            SyncKind::Full,
            failed_at,
            SyncCounts::default(),
            Some("key was revoked"),
        )
        .await
        .expect("complete sync");

        let before = crate::settings::load_integrations(&conn, project_id)
            .await
            .expect("load status");
        assert_eq!(
            before[0].last_sync_error.as_deref(),
            Some("key was revoked")
        );
        assert!(before[0].last_sync_attempted_at.is_some());
        assert!(before[0].last_synced_at.is_some());

        save_new_stripe_key(&conn, integration_id, "sk_second").await;

        let after = crate::settings::load_integrations(&conn, project_id)
            .await
            .expect("load status");
        assert_eq!(after[0].last_sync_error, None);
        assert_eq!(after[0].last_finished_sync_error, None);
        assert_eq!(after[0].last_sync_attempted_at, None);
        assert_eq!(after[0].last_synced_at, None);
        delete_account(&conn, &account_id).await;
    }

    #[tokio::test]
    async fn a_running_sync_does_not_hide_the_last_error() {
        let (server_context, conn) = crate::test_utils::main_test_db("status-running").await;
        let (account_id, integration_id) = insert_integration(&conn, "status_running").await;
        let project_id: i32 = conn
            .query_one(
                "SELECT project_id FROM integrations WHERE id = $1",
                &[&integration_id],
            )
            .await
            .expect("load integration")
            .get(0);
        run_sync(
            &server_context,
            integration_id,
            SyncKind::Full,
            SyncKind::Full,
            Some("key was revoked"),
        )
        .await;
        let status = async || {
            crate::settings::load_integrations(&conn, project_id)
                .await
                .expect("load status")
                .remove(0)
        };
        assert_eq!(
            status().await.last_finished_sync_error.as_deref(),
            Some("key was revoked")
        );

        let started_at =
            create_integration_sync_row(server_context.clone(), integration_id, 0, SyncKind::Full)
                .await
                .expect("start sync")
                .expect("credentials are current");
        let running = status().await;
        assert_eq!(
            running.last_finished_sync_error.as_deref(),
            Some("key was revoked")
        );
        // What released CLIs read: no outcome yet for the attempt they see.
        assert_eq!(running.last_sync_error, None);

        complete_integration_sync(
            server_context.clone(),
            integration_id,
            0,
            SyncKind::Full,
            started_at,
            SyncCounts::default(),
            None,
        )
        .await
        .expect("complete sync");
        assert_eq!(status().await.last_finished_sync_error, None);
        delete_account(&conn, &account_id).await;
    }

    #[tokio::test]
    async fn a_queued_sync_with_old_credentials_does_not_start() {
        let (server_context, conn) = crate::test_utils::main_test_db("credentials-queued").await;
        let (account_id, integration_id) = insert_integration(&conn, "credentials_queued").await;
        assert!(credentials_are_current(server_context.clone(), integration_id, 0).await);

        save_new_stripe_key(&conn, integration_id, "sk_new").await;

        assert!(!credentials_are_current(server_context.clone(), integration_id, 0).await);
        let started =
            create_integration_sync_row(server_context.clone(), integration_id, 0, SyncKind::Full)
                .await
                .expect("start sync");
        assert_eq!(started, None);
        delete_account(&conn, &account_id).await;
    }

    #[tokio::test]
    async fn completing_a_sync_that_was_never_started_changes_nothing() {
        let (server_context, conn) = crate::test_utils::main_test_db("sync-unknown").await;
        let (account_id, integration_id) = insert_integration(&conn, "sync_unknown").await;

        complete_integration_sync(
            server_context.clone(),
            integration_id,
            0,
            SyncKind::Full,
            SystemTime::UNIX_EPOCH,
            SyncCounts::default(),
            None,
        )
        .await
        .expect("complete sync");

        assert_eq!(
            scheduler_state(&conn, integration_id).await,
            SchedulerState {
                completed_at: None,
                success_at: None,
                full_success_at: None,
            }
        );
        delete_account(&conn, &account_id).await;
    }
}
