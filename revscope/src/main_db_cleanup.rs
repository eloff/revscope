//! A daily task that, on the hosted service, moves accounts whose subscription
//! ended without a webhook saying so onto the free plan and reports refunds
//! still owed, and that deletes rows the main Postgres database no longer needs:
//! expired tokens and sessions, old ingest errors, invites that lapsed, and MCP
//! client registrations that never got a token.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::sentry_report::report_error;
use crate::server_context::ServerContext;

const CLEANUP_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

static CLEANUP_TASK_STARTED: AtomicBool = AtomicBool::new(false);

pub fn start_main_db_cleanup_task(server_context: Arc<ServerContext>) {
    if CLEANUP_TASK_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }

    crate::task::spawn(server_context.clone(), async move {
        loop {
            run_main_db_cleanup(server_context.clone()).await;
            if wait_for_shutdown_or_cleanup_interval(&server_context).await {
                break;
            }
        }
    });
}

/// Each step runs even when an earlier one failed, since they touch unrelated
/// tables.
pub async fn run_main_db_cleanup(server_context: Arc<ServerContext>) {
    if let Err(error) = crate::tokens::delete_expired_tokens(server_context.clone()).await {
        report_error!("tokens.cleanup", "expired token cleanup failed: {error:?}");
    }
    if let Err(error) = crate::ingest::delete_expired_errors(server_context.clone()).await {
        report_error!("errors.cleanup", "expired error cleanup failed: {error:?}");
    }
    #[cfg(feature = "billing")]
    if let Err(error) = crate::billing::lapse::sweep(server_context.clone()).await {
        report_error!(
            "billing.lapse_sweep",
            "could not check for subscriptions that have ended: {error}"
        );
    }
    #[cfg(feature = "billing")]
    if let Err(error) = crate::billing::owed_refunds::report_unsettled(server_context.clone()).await
    {
        report_error!(
            "owed_refunds.report",
            "could not check for owed refunds: {error:?}"
        );
    }
    if let Err(error) = crate::pending_users::delete_lapsed_invites(server_context.clone()).await {
        report_error!(
            "pending_users.cleanup",
            "lapsed invite cleanup failed: {error:?}"
        );
    }
    if let Err(error) = crate::mcp::clients::delete_unused_clients(server_context).await {
        report_error!(
            "oauth_clients.cleanup",
            "unused MCP client cleanup failed: {error:?}"
        );
    }
}

async fn wait_for_shutdown_or_cleanup_interval(server_context: &ServerContext) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(CLEANUP_INTERVAL) => false,
        _ = server_context.wait_for_shutdown() => true,
    }
}
