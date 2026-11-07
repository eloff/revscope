//! Applies pending migrations to every project database at startup and daily.
//!
//! Migrations otherwise run when something opens a project, so a project nobody
//! visits would keep what a migration exists to erase. A checkpoint follows
//! because erased rows stay in the WAL, and in the main file's old pages, until
//! one runs, which a busy project's long-lived connection can put off for a long
//! time.
//!
//! It repeats because one pass can't be the last word. A previous release can
//! outlast any wait: its connections stay open while they have traffic, it can
//! be rolled back to, and it isn't in `previous_pids` once it stops listening.
//! A database can also be too busy on the day.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use log::{info, warn};
use rusqlite::{Connection, OpenFlags, Transaction, TransactionBehavior};

use crate::download_snapshots::process_alive;
use crate::projects::{is_valid_project_id, normalize_project_id};
use crate::sentry_report::report_error;
use crate::server_context::ServerContext;

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const SWEEP_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const HANDOVER_WAIT: Duration = Duration::from_secs(10 * 60);
const HANDOVER_POLL: Duration = Duration::from_secs(1);
const ERASE_USERAGENT_IPS: &str = include_str!("../migrations/00014_erase_useragents_ip.sql");
const ERASE_USERAGENT_IPS_MIGRATION: i64 = 14;

/// `previous_pids` are the processes this one just told to exit. The first pass
/// gives them a while to finish, because they keep flushing while they drain and
/// the previous release can write the IPs this erases.
pub fn start_project_db_sweep_task(server_context: Arc<ServerContext>, previous_pids: Vec<u32>) {
    let Some(data_dir) = server_context.data_dir().map(Path::to_path_buf) else {
        return;
    };
    crate::task::spawn(server_context.clone(), async move {
        wait_for_processes_to_exit(&server_context, &previous_pids, HANDOVER_WAIT).await;
        while !server_context.shutdown_requested() {
            let context = server_context.clone();
            let directory = data_dir.clone();
            let pass = crate::task::spawn_blocking(server_context.clone(), move || {
                sweep_project_databases(&directory, || context.shutdown_requested());
            })
            .await;
            if let Err(error) = pass {
                report_error!("project_db.sweep", "project database sweep failed: {error}");
            }
            tokio::select! {
                _ = tokio::time::sleep(SWEEP_INTERVAL) => {}
                _ = server_context.wait_for_shutdown() => {}
            }
        }
    });
}

/// Returns once `pids` have exited, after `limit`, or at shutdown, whichever is
/// first.
async fn wait_for_processes_to_exit(context: &ServerContext, pids: &[u32], limit: Duration) {
    let started = Instant::now();
    while pids.iter().copied().any(process_alive) {
        if started.elapsed() >= limit {
            warn!("starting to sweep project databases while {pids:?} still drain");
            return;
        }
        tokio::select! {
            _ = tokio::time::sleep(HANDOVER_POLL) => {}
            _ = context.wait_for_shutdown() => return,
        }
    }
}

#[derive(Default)]
struct SweepTotals {
    swept: u32,
    failed: u32,
    checkpoints_blocked: u32,
}

/// Failures are per project, so one bad database doesn't stop the rest. Each
/// database is opened directly rather than through the connection cache, which
/// only holds projects loaded from the main database.
pub(crate) fn sweep_project_databases(data_dir: &Path, should_stop: impl Fn() -> bool) {
    let entries = match std::fs::read_dir(data_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => {
            report_error!(
                "project_db.sweep",
                "listing {} failed: {error}",
                data_dir.display()
            );
            return;
        }
    };
    let mut totals = SweepTotals::default();
    for entry in entries {
        if should_stop() {
            info!("project database sweep stopped early");
            return;
        }
        let path = match entry {
            Ok(entry) => entry.path(),
            Err(error) => {
                totals.failed += 1;
                report_error!("project_db.sweep", "reading a directory entry: {error}");
                continue;
            }
        };
        if !is_project_database(&path) {
            continue;
        }
        match sweep_one(&path) {
            Ok(Swept::Done) => totals.swept += 1,
            Ok(Swept::CheckpointBlocked) => {
                totals.swept += 1;
                totals.checkpoints_blocked += 1;
                warn!(
                    "a reader blocked the checkpoint of {}; erased rows stay on disk until the next sweep",
                    path.display()
                );
            }
            Ok(Swept::Skipped) => {}
            Err(error) => {
                totals.failed += 1;
                report_error!(
                    "project_db.sweep",
                    "sweeping {} failed: {error}",
                    path.display()
                );
            }
        }
    }
    info!(
        "project database sweep finished: {} swept, {} failed, {} checkpoints blocked",
        totals.swept, totals.failed, totals.checkpoints_blocked
    );
}

/// Only the canonical form: `007.db` and `+1.db` name no project.
fn is_project_database(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension == "db")
        && path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .is_some_and(|stem| is_valid_project_id(stem) && normalize_project_id(stem) == stem)
}

enum Swept {
    Done,
    CheckpointBlocked,
    Skipped,
}

fn sweep_one(path: &Path) -> Result<Swept, Box<dyn std::error::Error>> {
    // Without SQLITE_OPEN_CREATE, a project deleted since the directory listing
    // stays deleted.
    let conn = match Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(conn) => conn,
        Err(_) if !path.exists() => return Ok(Swept::Skipped),
        Err(error) => return Err(error.into()),
    };
    conn.busy_timeout(BUSY_TIMEOUT)?;
    if !has_applied_a_migration(&conn)? {
        return Ok(Swept::Skipped);
    }
    crate::db::migrations::apply_missing_project_migrations(&conn)?;
    erase_useragent_ips_written_since_migrating(&conn)?;
    let blocked: i64 = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))?;
    Ok(if blocked == 0 {
        Swept::Done
    } else {
        Swept::CheckpointBlocked
    })
}

/// Whoever is creating a project runs its first migration without a transaction,
/// so a project with no migration recorded yet is theirs to finish.
fn has_applied_a_migration(conn: &Connection) -> rusqlite::Result<bool> {
    let has_table: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE name = 'migrations')",
        [],
        |row| row.get(0),
    )?;
    if !has_table {
        return Ok(false);
    }
    conn.query_row("SELECT EXISTS (SELECT 1 FROM migrations)", [], |row| {
        row.get(0)
    })
}

/// The previous release writes IPs after 00014 when it is still draining, or when
/// it is rolled back to. The migration won't run twice, so run its SQL here, which
/// is safe to repeat. Does nothing once a later migration drops the column.
fn erase_useragent_ips_written_since_migrating(
    conn: &Connection,
) -> Result<(), Box<dyn std::error::Error>> {
    let has_column: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM pragma_table_info('useragents') WHERE name = 'ip')",
        [],
        |row| row.get(0),
    )?;
    if !has_column {
        return Ok(());
    }
    let has_ips: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM useragents WHERE ip IS NOT NULL)",
        [],
        |row| row.get(0),
    )?;
    if !has_ips {
        return Ok(());
    }
    // The file rebuilds both tables as 00014 left them, which would quietly undo
    // any later migration of them, so stop and say so instead.
    let last_migration: i64 =
        conn.query_row("SELECT MAX(migration_number) FROM migrations", [], |row| {
            row.get(0)
        })?;
    if last_migration != ERASE_USERAGENT_IPS_MIGRATION {
        return Err(format!(
            "useragents still has IPs, but erasing them is only safe at migration {ERASE_USERAGENT_IPS_MIGRATION}, not {last_migration}"
        )
        .into());
    }
    let transaction = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    transaction.execute_batch(ERASE_USERAGENT_IPS)?;
    transaction.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::TestDataDir;

    fn has_table(conn: &Connection, name: &str) -> bool {
        conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE name = ?1)",
            [name],
            |row| row.get(0),
        )
        .expect("query sqlite_master")
    }

    #[test]
    fn leaves_other_files_and_projects_still_being_created_alone() {
        let directory = TestDataDir::new("project-db-sweep");
        for name in ["notes.db", "-1.db", "+1.db", "007.db", "2.db"] {
            let conn = Connection::open(directory.path().join(name)).expect("open");
            conn.execute_batch("CREATE TABLE migrations (migration_number INTEGER PRIMARY KEY);")
                .expect("create table");
        }

        sweep_project_databases(directory.path(), || false);

        for name in ["notes.db", "-1.db", "+1.db", "007.db", "2.db"] {
            let conn = Connection::open(directory.path().join(name)).expect("open");
            assert!(!has_table(&conn, "useragents"), "{name} was migrated");
        }
    }

    #[tokio::test]
    async fn waiting_ends_when_the_process_exits() {
        let context = ServerContext::new_in_memory("sweep-wait-exit");
        let mut child = std::process::Command::new("true").spawn().expect("spawn");
        let pid = child.id();
        child.wait().expect("reap child");
        let started = Instant::now();
        wait_for_processes_to_exit(&context, &[pid], Duration::from_secs(600)).await;
        assert!(started.elapsed() < Duration::from_secs(60));
    }

    #[tokio::test]
    async fn waiting_for_a_live_process_ends_at_the_limit() {
        let context = ServerContext::new_in_memory("sweep-wait-limit");
        let started = Instant::now();
        let own_pid = std::process::id();
        wait_for_processes_to_exit(&context, &[own_pid], Duration::from_millis(1)).await;
        assert!(started.elapsed() < Duration::from_secs(60));
    }

    #[tokio::test]
    async fn waiting_for_a_live_process_ends_at_shutdown() {
        let context = ServerContext::new_in_memory("sweep-wait-shutdown");
        context.request_shutdown();
        let started = Instant::now();
        let own_pid = std::process::id();
        wait_for_processes_to_exit(&context, &[own_pid], Duration::from_secs(600)).await;
        assert!(started.elapsed() < Duration::from_secs(60));
    }

    #[test]
    fn recognizes_only_canonical_project_ids() {
        for (name, expected) in [
            ("1.db", true),
            ("1234.db", true),
            ("007.db", false),
            ("+1.db", false),
            ("-1.db", false),
            ("notes.db", false),
            ("1.db-wal", false),
        ] {
            assert_eq!(is_project_database(Path::new(name)), expected, "{name}");
        }
    }
}
