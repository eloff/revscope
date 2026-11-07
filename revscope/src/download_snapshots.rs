//! Point-in-time copies of project databases, which downloads stream from.
//!
//! The live `.db` file cannot be streamed as it is: writers keep checkpointing
//! WAL pages into it, so a copy read over several minutes mixes pages from
//! different moments and is corrupt. `VACUUM INTO` writes a consistent copy
//! from inside a single read transaction, with the WAL already folded in.
//! Nothing writes to a snapshot afterwards, so a resumed download continues
//! with exactly the bytes it started with.
//!
//! Each snapshot costs a full copy of the database on disk, and any signed-in
//! account may download the demo, so the copies are bounded: a project never
//! has more than [`MAX_SNAPSHOTS_PER_PROJECT`] alive, counting ones a response
//! is still streaming. A snapshot younger than [`REUSE_SECONDS`] serves new
//! downloads instead of a fresh copy being made, and so does the newest one
//! when the project is at the cap. Making room only ever evicts a snapshot
//! that is not being streamed and has been idle for [`IDLE_SECONDS`], and one
//! idle for [`DOWNLOAD_TOKEN_TTL_SECONDS`] is deleted regardless of room.

use std::any::Any;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dashmap::DashMap;
use log::warn;
use parking_lot::Mutex;
use rusqlite::OpenFlags;

use crate::db::project_database_path;
use crate::server_context::ServerContext;
use crate::tokens::DOWNLOAD_TOKEN_TTL_SECONDS;
use crate::util::seconds_since_epoch;

/// Under the data directory, beside the project databases. Each process
/// writes to a subdirectory named for its pid: during a zero-downtime upgrade
/// the old process keeps serving its snapshots after the new one starts, so
/// files are only deleted along with the process that made them. The server
/// context is a static that is never dropped, so nothing deletes them on exit;
/// [`remove_orphaned_snapshot_dirs`] does once the process is gone.
const SNAPSHOT_DIR: &str = "download-snapshots";

const MAX_SNAPSHOTS_PER_PROJECT: usize = 3;

/// How long a dropped download has to resume, counted from when its last
/// response ended, before its snapshot may be evicted to make room for a fresh
/// copy.
const IDLE_SECONDS: u32 = 15 * 60;

/// Repeated requests within this window share one copy, so asking for a
/// download over and over cannot keep the disk busy copying the database.
const REUSE_SECONDS: u32 = 60;

const PRUNE_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// A consistent copy of one project's database. The file is deleted when the
/// last reference is dropped, so a response still streaming it keeps it.
#[derive(Debug)]
pub(crate) struct Snapshot {
    pub(crate) path: PathBuf,
    pub(crate) length: u64,
    /// Unique across processes, so a validator from before a restart never
    /// matches a different copy.
    pub(crate) etag: String,
    created_at: u32,
    last_served: AtomicU32,
}

impl Snapshot {
    fn touch(&self) {
        self.last_served
            .fetch_max(seconds_since_epoch(), Ordering::Relaxed);
    }

    fn idle_for(&self, seconds: u32, now: u32) -> bool {
        self.last_served
            .load(Ordering::Relaxed)
            .saturating_add(seconds)
            <= now
    }
}

/// Held by a response for as long as it streams a snapshot. Counting idle
/// time from when the response ends, rather than when it started, gives a
/// long download that drops the full [`IDLE_SECONDS`] to resume.
struct Lease(Arc<Snapshot>);

impl Drop for Lease {
    fn drop(&mut self) {
        self.0.touch();
    }
}

/// What a response holds to keep `snapshot` alive while streaming it.
pub(crate) fn lease(snapshot: Arc<Snapshot>) -> Arc<dyn Any + Send + Sync> {
    Arc::new(Lease(snapshot))
}

/// Whether a response holds this snapshot besides the project's own list.
/// Only meaningful under the list's lock, which every clone is taken under.
fn streaming(snapshot: &Arc<Snapshot>) -> bool {
    Arc::strong_count(snapshot) > 1
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        remove_path(&self.path);
    }
}

#[derive(Default)]
struct ProjectSnapshots {
    /// Oldest first.
    snapshots: Mutex<Vec<Arc<Snapshot>>>,
    /// Held while copying, separately from `snapshots`, so concurrent requests
    /// for a fresh copy wait and then share it, while resumes keep being
    /// served from the existing ones.
    creating: Mutex<()>,
}

pub(crate) struct DownloadSnapshots {
    projects: DashMap<i32, Arc<ProjectSnapshots>>,
    next_id: AtomicU64,
}

impl DownloadSnapshots {
    pub(crate) fn new() -> Self {
        Self {
            projects: DashMap::new(),
            next_id: AtomicU64::new(0),
        }
    }

    /// The snapshot a resuming client's `If-Range` validator names, if the
    /// project still has it.
    pub(crate) fn find(&self, project_key: i32, etag: &str) -> Option<Arc<Snapshot>> {
        let project = self.projects.get(&project_key)?.clone();
        let snapshots = project.snapshots.lock();
        let found = snapshots.iter().find(|snapshot| snapshot.etag == etag)?;
        found.touch();
        Some(found.clone())
    }

    /// A snapshot no older than [`REUSE_SECONDS`], made now if there is none
    /// and there is room; otherwise the newest there is. Blocks for as long as
    /// copying the database takes.
    pub(crate) fn current(
        &self,
        server_context: &ServerContext,
        project_id: &str,
        project_key: i32,
    ) -> Result<Arc<Snapshot>, String> {
        let project = self.projects.entry(project_key).or_default().clone();
        let _creating = project.creating.lock();
        if let Some(reusable) = project.reusable() {
            return Ok(reusable);
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let created = create_snapshot(server_context, project_id, project_key, id);
        let snapshot = match created {
            Ok(snapshot) => Arc::new(snapshot),
            Err(error) => {
                // Don't keep the entry this call added for a project that
                // turns out to have no database, such as one just deleted.
                // Only while nobody else holds it: a request waiting on
                // `creating` would otherwise fill a list no lookup can reach.
                // Clones are taken under the map's shard lock, which
                // `remove_if` holds, so the count cannot change under it.
                self.projects.remove_if(&project_key, |_, listed| {
                    Arc::ptr_eq(listed, &project)
                        && Arc::strong_count(listed) == 2
                        && listed.snapshots.lock().is_empty()
                });
                return Err(error);
            }
        };
        project.push(snapshot.clone());
        // A delete that ran while the copy was made could have removed this
        // project's list before the copy joined it. Deleting marks the project
        // before removing the list, so checking the mark after joining catches
        // every interleaving.
        if server_context.deleted_project_ids.contains(&project_key) {
            drop(project.clear());
            self.remove_project(project_key);
            return Err(format!("project {project_id} was deleted"));
        }
        Ok(snapshot)
    }

    /// Deletes every snapshot of a project whose database is going away.
    pub(crate) fn remove_project(&self, project_key: i32) {
        if let Some((_, project)) = self.projects.remove(&project_key) {
            drop(project.clear());
        }
    }

    /// Deletes snapshots nobody has downloaded from for a token's lifetime.
    pub(crate) fn prune_expired(&self) {
        let now = seconds_since_epoch();
        let mut expired = Vec::new();
        for project in self.projects.iter() {
            expired.extend(
                project.evict(|snapshot| snapshot.idle_for(DOWNLOAD_TOKEN_TTL_SECONDS, now)),
            );
        }
        // Dropped outside the locks: dropping deletes a file.
        drop(expired);
    }

    #[cfg(test)]
    pub(crate) fn has_project(&self, project_key: i32) -> bool {
        self.projects.contains_key(&project_key)
    }

    #[cfg(test)]
    fn insert(&self, project_key: i32, snapshot: Snapshot) {
        let project = self.projects.entry(project_key).or_default().clone();
        project.push(Arc::new(snapshot));
    }
}

impl ProjectSnapshots {
    /// The newest snapshot if it is fresh enough to share, or if there is no
    /// room for another even after evicting the oldest idle one. Idle
    /// snapshots are only evicted when a copy needs their room.
    fn reusable(&self) -> Option<Arc<Snapshot>> {
        let now = seconds_since_epoch();
        let fresh = self.newest_if(|newest| now < newest.created_at.saturating_add(REUSE_SECONDS));
        if fresh.is_some() {
            return fresh;
        }
        if self.snapshots.lock().len() >= MAX_SNAPSHOTS_PER_PROJECT {
            drop(self.evict_oldest_idle(now));
        }
        if self.snapshots.lock().len() < MAX_SNAPSHOTS_PER_PROJECT {
            return None;
        }
        self.newest_if(|_| true)
    }

    fn newest_if(&self, pick: impl Fn(&Snapshot) -> bool) -> Option<Arc<Snapshot>> {
        let snapshots = self.snapshots.lock();
        let newest = snapshots.last().filter(|newest| pick(newest))?;
        newest.touch();
        Some(newest.clone())
    }

    /// Takes every snapshot, streaming or not, for the caller to drop outside
    /// the lock: dropping deletes a file.
    fn clear(&self) -> Vec<Arc<Snapshot>> {
        std::mem::take(&mut *self.snapshots.lock())
    }

    fn push(&self, snapshot: Arc<Snapshot>) {
        self.snapshots.lock().push(snapshot);
    }

    fn evict_oldest_idle(&self, now: u32) -> Option<Arc<Snapshot>> {
        let mut snapshots = self.snapshots.lock();
        let oldest = snapshots
            .iter()
            .position(|snapshot| !streaming(snapshot) && snapshot.idle_for(IDLE_SECONDS, now))?;
        Some(snapshots.remove(oldest))
    }

    /// Removes and returns the snapshots `evict` picks, except ones a response
    /// is streaming: those stay listed, so they keep counting toward the cap.
    fn evict(&self, evict: impl Fn(&Snapshot) -> bool) -> Vec<Arc<Snapshot>> {
        let mut snapshots = self.snapshots.lock();
        let (evicted, kept) = std::mem::take(&mut *snapshots)
            .into_iter()
            .partition(|snapshot| !streaming(snapshot) && evict(snapshot));
        *snapshots = kept;
        evicted
    }
}

fn create_snapshot(
    server_context: &ServerContext,
    project_id: &str,
    project_key: i32,
    id: u64,
) -> Result<Snapshot, String> {
    let dir = snapshot_dir(server_context)
        .ok_or_else(|| "project databases are not stored on disk".to_string())?;
    std::fs::create_dir_all(&dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since_epoch| since_epoch.as_nanos());
    let path = dir.join(format!("{project_key}-{nanos:x}-{id:x}.db"));
    let target = path
        .to_str()
        .ok_or_else(|| format!("{} is not valid UTF-8", path.display()))?;

    // A connection of its own rather than a pooled one: `VACUUM INTO` attaches
    // its target, and pooled connections forbid ATTACH because they run user
    // SQL. This one runs nothing else and is closed straight after.
    let source = project_database_path(server_context, project_id);
    let copied = rusqlite::Connection::open_with_flags(
        &source,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .and_then(|conn| conn.execute("VACUUM INTO ?1", [target]));
    if let Err(error) = copied {
        remove_path(&path);
        return Err(format!("snapshot of project {project_id}: {error}"));
    }

    let length = match std::fs::metadata(&path) {
        Ok(metadata) => metadata.len(),
        Err(error) => {
            remove_path(&path);
            return Err(format!("{}: {error}", path.display()));
        }
    };
    let now = seconds_since_epoch();
    Ok(Snapshot {
        path,
        length,
        etag: format!("\"{nanos:x}-{id:x}\""),
        created_at: now,
        last_served: AtomicU32::new(now),
    })
}

fn snapshot_root(server_context: &ServerContext) -> Option<PathBuf> {
    server_context
        .data_dir()
        .map(|data_dir| data_dir.join(SNAPSHOT_DIR))
}

fn snapshot_dir(server_context: &ServerContext) -> Option<PathBuf> {
    snapshot_root(server_context).map(|root| root.join(std::process::id().to_string()))
}

/// Deletes snapshots that exited processes left behind, at startup and then
/// periodically, and prunes this process's expired ones until shutdown.
pub fn start_download_snapshot_cleanup_task(server_context: Arc<ServerContext>) {
    let (Some(root), Some(own)) = (
        snapshot_root(&server_context),
        snapshot_dir(&server_context),
    ) else {
        return;
    };
    // Whatever is here already was left by an earlier process with our pid.
    remove_path(&own);
    remove_orphaned_snapshot_dirs(&root);
    crate::task::spawn(server_context.clone(), async move {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(PRUNE_INTERVAL) => {
                    server_context.download_snapshots.prune_expired();
                    remove_orphaned_snapshot_dirs(&root);
                }
                _ = server_context.wait_for_shutdown() => break,
            }
        }
    });
}

/// Removes every entry under `root` except the directories of live processes,
/// this one included.
fn remove_orphaned_snapshot_dirs(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let owner = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok());
        if !owner.is_some_and(process_alive) {
            remove_path(&entry.path());
        }
    }
}

pub(crate) fn process_alive(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // Signal 0 only checks that the process exists. EPERM means it does, but
    // belongs to another user.
    let signalled = unsafe { libc::kill(pid, 0) } == 0;
    signalled || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

fn remove_path(path: &Path) {
    let removed = if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    match removed {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => warn!("failed to delete {}: {error}", path.display()),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::test_utils::TestDataDir;

    /// A snapshot record for a file the test controls.
    pub(crate) fn snapshot_at(path: PathBuf, length: u64, etag: &str) -> Snapshot {
        let now = seconds_since_epoch();
        Snapshot {
            path,
            length,
            etag: etag.to_string(),
            created_at: now,
            last_served: AtomicU32::new(now),
        }
    }

    /// A snapshot file made `age` seconds ago and last served `idle` ago.
    fn aged_snapshot(dir: &Path, name: &str, age: u32, idle: u32) -> Snapshot {
        let path = dir.join(name);
        std::fs::write(&path, b"copy").expect("write snapshot file");
        let mut snapshot = snapshot_at(path, 4, &format!("\"{name}\""));
        let now = seconds_since_epoch();
        snapshot.created_at = now - age;
        snapshot.last_served = AtomicU32::new(now - idle);
        snapshot
    }

    fn project_with(snapshots: Vec<Snapshot>) -> ProjectSnapshots {
        let project = ProjectSnapshots::default();
        for snapshot in snapshots {
            project.push(Arc::new(snapshot));
        }
        project
    }

    fn listed(project: &ProjectSnapshots) -> Vec<String> {
        let snapshots = project.snapshots.lock();
        snapshots
            .iter()
            .map(|snapshot| snapshot.etag.clone())
            .collect()
    }

    #[test]
    fn a_fresh_snapshot_is_shared_and_a_stale_one_makes_way_for_a_copy() {
        let dir = TestDataDir::new("download-snapshots-fresh");
        let fresh = project_with(vec![aged_snapshot(dir.path(), "fresh", 0, 0)]);
        assert_eq!(fresh.reusable().expect("shared").etag, "\"fresh\"");

        let stale = project_with(vec![aged_snapshot(dir.path(), "stale", REUSE_SECONDS, 0)]);
        assert!(stale.reusable().is_none());
        assert!(project_with(Vec::new()).reusable().is_none());
    }

    #[test]
    fn at_the_cap_the_newest_is_shared_rather_than_another_copy_made() {
        let dir = TestDataDir::new("download-snapshots-cap");
        let project = project_with(
            ["a", "b", "c"]
                .into_iter()
                .map(|name| aged_snapshot(dir.path(), name, REUSE_SECONDS, 0))
                .collect(),
        );
        assert_eq!(project.reusable().expect("shared").etag, "\"c\"");
        assert_eq!(listed(&project).len(), MAX_SNAPSHOTS_PER_PROJECT);
    }

    #[test]
    fn idle_snapshots_stay_while_nothing_needs_their_room() {
        let dir = TestDataDir::new("download-snapshots-no-room-needed");
        let ttl = DOWNLOAD_TOKEN_TTL_SECONDS;
        let project = project_with(vec![
            aged_snapshot(dir.path(), "idle", ttl - 60, ttl - 60),
            aged_snapshot(dir.path(), "fresh", 0, 0),
        ]);
        assert_eq!(project.reusable().expect("shared").etag, "\"fresh\"");
        assert!(project.reusable().is_some());
        let stale = project_with(vec![aged_snapshot(dir.path(), "old", ttl - 60, ttl - 60)]);
        assert!(stale.reusable().is_none());
        assert_eq!(listed(&project), ["\"idle\"", "\"fresh\""]);
        assert_eq!(listed(&stale), ["\"old\""]);
    }

    #[test]
    fn idle_time_counts_from_when_the_last_response_ends() {
        let dir = TestDataDir::new("download-snapshots-lease");
        let project = project_with(vec![aged_snapshot(
            dir.path(),
            "long",
            IDLE_SECONDS * 2,
            IDLE_SECONDS * 2,
        )]);
        let leased = lease(project.snapshots.lock()[0].clone());
        drop(leased);
        let now = seconds_since_epoch();
        assert!(!project.snapshots.lock()[0].idle_for(IDLE_SECONDS, now));
    }

    #[test]
    fn making_room_evicts_only_idle_snapshots_that_are_not_streaming() {
        let dir = TestDataDir::new("download-snapshots-room");
        let project = project_with(vec![
            aged_snapshot(dir.path(), "idle", IDLE_SECONDS, IDLE_SECONDS),
            aged_snapshot(dir.path(), "streaming", IDLE_SECONDS, IDLE_SECONDS),
            aged_snapshot(dir.path(), "resumable", REUSE_SECONDS, IDLE_SECONDS - 60),
        ]);
        let idle_path = dir.path().join("idle");
        let streaming = project.snapshots.lock()[1].clone();

        assert!(project.reusable().is_none(), "room was made for a copy");
        assert!(!idle_path.exists());
        assert_eq!(listed(&project), ["\"streaming\"", "\"resumable\""]);

        drop(streaming);
        let evicted = project.evict(|_| true);
        let paths: Vec<PathBuf> = evicted
            .iter()
            .map(|snapshot| snapshot.path.clone())
            .collect();
        drop(evicted);
        assert!(paths.iter().all(|path| !path.exists()));
    }

    #[test]
    fn making_room_evicts_only_the_oldest_idle_snapshot() {
        let dir = TestDataDir::new("download-snapshots-oldest");
        let project = project_with(vec![
            aged_snapshot(dir.path(), "oldest", IDLE_SECONDS * 3, IDLE_SECONDS * 3),
            aged_snapshot(dir.path(), "older", IDLE_SECONDS * 2, IDLE_SECONDS * 2),
            aged_snapshot(dir.path(), "newest", IDLE_SECONDS, IDLE_SECONDS),
        ]);
        assert!(project.reusable().is_none(), "room was made for a copy");
        assert_eq!(listed(&project), ["\"older\"", "\"newest\""]);
    }

    #[test]
    fn pruning_deletes_only_snapshots_unserved_for_a_token_lifetime() {
        let dir = TestDataDir::new("download-snapshots-prune");
        let snapshots = DownloadSnapshots::new();
        let ttl = DOWNLOAD_TOKEN_TTL_SECONDS;
        snapshots.insert(1, aged_snapshot(dir.path(), "expired", ttl, ttl));
        snapshots.insert(2, aged_snapshot(dir.path(), "recent", ttl, ttl - 60));

        snapshots.prune_expired();
        assert!(!dir.path().join("expired").exists());
        assert!(snapshots.find(1, "\"expired\"").is_none());
        assert!(snapshots.find(2, "\"recent\"").is_some());
        assert!(snapshots.find(2, "\"expired\"").is_none());
    }

    #[test]
    fn only_the_directories_of_exited_processes_are_removed() {
        let root = TestDataDir::new("download-snapshots-orphans");
        let own = root.path().join(std::process::id().to_string());
        // Pid 1 always exists; the pid limit on Linux is below 2^22.
        let live = root.path().join("1");
        let exited = root.path().join(((1_u32 << 22) + 1).to_string());
        let stray = root.path().join("stray.db");
        for dir in [&own, &live, &exited] {
            std::fs::create_dir_all(dir).expect("create dir");
            std::fs::write(dir.join("1-0-0.db"), b"copy").expect("write");
        }
        std::fs::write(&stray, b"copy").expect("write");

        remove_orphaned_snapshot_dirs(root.path());
        assert!(own.exists());
        assert!(live.exists());
        assert!(!exited.exists());
        assert!(!stray.exists());
    }
}
