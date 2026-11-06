use std::fmt;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use dashmap::{DashMap, mapref::entry::Entry};
use log::info;
use parking_lot::{Condvar, Mutex};
use rusqlite::config::DbConfig;
use rusqlite::limits::Limit;
use rusqlite::{OpenFlags, Params, ffi};

use crate::db::DbError;
use crate::server_context::{ProjectDbBackend, ServerContext};
use crate::util::seconds_since_epoch;

/// Idle connections outlive one ingest timer flush so periodic writes reuse them.
const PROJECT_DB_CACHE_TTL_SECS: u32 = crate::ingest::FLUSH_INTERVAL_SECS as u32 + 60;
const PROJECT_DB_CACHE_SWEEP_INTERVAL_SECS: u64 = 15;
const PROJECT_DB_CHECKOUT_TIMEOUT: Duration = Duration::from_secs(30);
const PREPARED_STATEMENT_CACHE_CAPACITY: usize = 64;
type RawConnection = rusqlite::Connection;

pub type ProjectCaches = Arc<DashMap<i32, Arc<ProjectCache>>>;

/// Per-project state shared across a request: identity plus the SQLite pool.
///
/// `name` and `account_id` are immutable and must be known when the entry is
/// created (see [`insert_project`] / [`crate::projects::fetch_project`]).
#[derive(Debug)]
pub struct ProjectCache {
    id: i32,
    name: String,
    /// Account UUID packed as `u128` (same encoding as `exclude_users`).
    account_id: u128,
    db: ProjectDbCache,
    /// Account-scoped settings loaded from the main DB (TTL-cached).
    account_settings: Mutex<Option<crate::exclude_users::AccountSettingsCache>>,
    /// Timezone the project's dashboards bucket dates in (TTL-cached).
    timezone: Mutex<Option<crate::timezone::ProjectTimezoneCache>>,
}

impl ProjectCache {
    #[cfg(test)]
    fn new_with_db(id: i32, name: String, account_id: u128, db: ProjectDbCache) -> Self {
        Self {
            id,
            name,
            account_id,
            db,
            account_settings: Mutex::new(None),
            timezone: Mutex::new(None),
        }
    }

    pub fn id(&self) -> i32 {
        self.id
    }

    pub fn id_string(&self) -> String {
        self.id.to_string()
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn account_id(&self) -> u128 {
        self.account_id
    }

    pub(crate) fn refresh_db_expiry(&self) {
        self.db.refresh_expiry();
    }

    pub(crate) fn account_settings_if_fresh(
        &self,
    ) -> Option<crate::exclude_users::AccountSettingsCache> {
        let guard = self.account_settings.lock();
        guard
            .as_ref()
            .filter(|settings| settings.is_fresh())
            .cloned()
    }

    pub(crate) fn store_account_settings(
        &self,
        settings: crate::exclude_users::AccountSettingsCache,
    ) {
        *self.account_settings.lock() = Some(settings);
    }

    pub(crate) fn timezone_if_fresh(&self) -> Option<crate::timezone::ProjectTimezone> {
        let guard = self.timezone.lock();
        guard
            .as_ref()
            .filter(|cached| cached.is_fresh())
            .map(|cached| cached.timezone().clone())
    }

    pub(crate) fn store_timezone(&self, timezone: crate::timezone::ProjectTimezoneCache) {
        *self.timezone.lock() = Some(timezone);
    }

    /// Forces the next read to reload the zone, after a settings save.
    pub(crate) fn clear_timezone(&self) {
        *self.timezone.lock() = None;
    }
}

/// Pack a UUID string (`xxxxxxxx-xxxx-...` or 32 hex digits) into `u128`.
pub(crate) fn parse_uuid_key(value: &str) -> Option<u128> {
    u128::from_str_radix(&value.replace('-', ""), 16).ok()
}

#[derive(Debug)]
struct InnerCache {
    idle: Option<Connection>,
    acquired_for_write: bool,
    migrations_up_to_date: bool,
}

#[derive(Debug)]
struct ProjectDbCache {
    expires_at: std::sync::atomic::AtomicU32,
    inner: Mutex<InnerCache>,
    write_available: Condvar,
}

impl ProjectDbCache {
    fn new_expiring() -> Self {
        Self {
            expires_at: std::sync::atomic::AtomicU32::new(
                seconds_since_epoch().saturating_add(PROJECT_DB_CACHE_TTL_SECS),
            ),
            inner: Mutex::new(InnerCache {
                idle: None,
                acquired_for_write: false,
                migrations_up_to_date: false,
            }),
            write_available: Condvar::new(),
        }
    }

    #[cfg(test)]
    fn new_non_expiring() -> Self {
        Self {
            expires_at: std::sync::atomic::AtomicU32::new(u32::MAX),
            inner: Mutex::new(InnerCache {
                idle: None,
                acquired_for_write: false,
                migrations_up_to_date: false,
            }),
            write_available: Condvar::new(),
        }
    }

    fn refresh_expiry(&self) {
        self.expires_at.store(
            seconds_since_epoch().saturating_add(PROJECT_DB_CACHE_TTL_SECS),
            Ordering::Relaxed,
        );
    }

    /// Drop a cached idle connection when its TTL has elapsed.
    fn expire_idle_if_due(&self, now: u32) {
        if now < self.expires_at.load(Ordering::Relaxed) {
            return;
        }
        let mut inner = self.inner.lock();
        if inner.acquired_for_write {
            return;
        }
        if let Some(mut idle) = inner.idle.take() {
            idle.close();
        }
    }
}

pub fn new_project_caches() -> ProjectCaches {
    Arc::new(DashMap::new())
}

/// Look up a cached project without creating one.
pub fn get_project(server_context: &ServerContext, project_id: i32) -> Option<Arc<ProjectCache>> {
    let project = server_context.project_cache.get(&project_id)?.clone();
    project.refresh_db_expiry();
    Some(project)
}

/// Insert a fully-identified project into [`ServerContext::project_cache`] and
/// [`ServerContext::project_name_cache`]. If an entry already exists for `project_id`,
/// returns that Arc (first insert wins for identity).
pub fn insert_project(
    server_context: &ServerContext,
    project_id: i32,
    name: String,
    account_id: u128,
) -> Arc<ProjectCache> {
    let project = insert_project_entry(server_context, project_id, name, account_id);
    // A lookup that read the project row before a delete committed can get
    // here after the delete dropped the cache entries; take them out again.
    // Checked after inserting so a delete landing in between is still seen.
    if server_context.deleted_project_ids.contains(&project_id) {
        evict_project(server_context, project_id);
    }
    project
}

/// Whether the project was deleted while this process ran.
pub(crate) fn is_project_deleted(server_context: &ServerContext, project_id: i32) -> bool {
    server_context.deleted_project_ids.contains(&project_id)
}

fn insert_project_entry(
    server_context: &ServerContext,
    project_id: i32,
    name: String,
    account_id: u128,
) -> Arc<ProjectCache> {
    match server_context.project_cache.entry(project_id) {
        Entry::Occupied(existing) => {
            let project = existing.get().clone();
            project.refresh_db_expiry();
            server_context
                .project_name_cache
                .insert(project.name().to_owned(), Arc::clone(&project));
            project
        }
        Entry::Vacant(vacant) => {
            let project = Arc::new(ProjectCache {
                id: project_id,
                name,
                account_id,
                db: ProjectDbCache::new_expiring(),
                account_settings: Mutex::new(None),
                timezone: Mutex::new(None),
            });
            project.refresh_db_expiry();
            vacant.insert(Arc::clone(&project));
            server_context
                .project_name_cache
                .insert(project.name().to_owned(), Arc::clone(&project));
            project
        }
    }
}

/// Resolve `project_id` from the cache, fetching identity from Postgres on miss.
pub fn require_project(
    server_context: Arc<ServerContext>,
    project_id: i32,
) -> Result<Arc<ProjectCache>, DbError> {
    if let Some(project) = get_project(server_context.as_ref(), project_id) {
        return Ok(project);
    }
    crate::projects::fetch_project_blocking(server_context, project_id)?
        .ok_or_else(|| DbError::ProjectDatabaseNotFound(project_id.to_string()))
}

#[derive(Clone, Debug)]
enum CacheIdentity {
    Project {
        project_key: i32,
        project_caches: ProjectCaches,
    },
}

#[derive(Clone, Debug)]
struct CacheRef {
    identity: CacheIdentity,
    project: Arc<ProjectCache>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum ConnectionKind {
    Read,
    Write,
}

impl fmt::Display for ConnectionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConnectionKind::Read => f.write_str("read"),
            ConnectionKind::Write => f.write_str("write"),
        }
    }
}

impl CacheRef {
    fn is_active(&self) -> bool {
        match self.identity {
            CacheIdentity::Project {
                project_key,
                ref project_caches,
            } => project_cache_entry_is_active(project_caches, project_key, &self.project),
        }
    }

    fn db(&self) -> &ProjectDbCache {
        &self.project.db
    }
}

pub struct Connection {
    connection: Option<RawConnection>,
    is_write: bool,
    cache_ref: CacheRef,
}

impl fmt::Debug for Connection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Connection")
            .field("is_write", &self.is_write)
            .finish_non_exhaustive()
    }
}

impl Deref for Connection {
    type Target = RawConnection;

    fn deref(&self) -> &Self::Target {
        self.connection
            .as_ref()
            .expect("connection lease must hold an open connection")
    }
}

impl Connection {
    pub fn execute<P: Params>(&self, sql: &str, params: P) -> Result<usize, rusqlite::Error> {
        self.prepare_cached(sql)?.execute(params)
    }

    pub fn query_row<T, P, F>(&self, sql: &str, params: P, f: F) -> Result<T, rusqlite::Error>
    where
        P: Params,
        F: FnOnce(&rusqlite::Row<'_>) -> Result<T, rusqlite::Error>,
    {
        self.prepare_cached(sql)?.query_row(params, f)
    }

    pub fn with_query<P, T>(
        &self,
        sql: &str,
        params: P,
        f: impl FnOnce(rusqlite::Rows<'_>) -> Result<T, rusqlite::Error>,
    ) -> Result<T, rusqlite::Error>
    where
        P: Params,
    {
        let mut stmt = self.prepare_cached(sql)?;
        f(stmt.query(params)?)
    }

    /// Temporarily allows writes on a read checkout so the caller can use TEMP
    /// tables (or other connection-local scratch state) without taking the
    /// exclusive write slot.
    ///
    /// Read checkouts run with `PRAGMA query_only = 1`, which blocks TEMP DDL/DML.
    /// This clears that pragma for the guard's lifetime and restores it on drop.
    /// Write checkouts are left unchanged (already writable).
    ///
    /// Prefer keeping the guard in scope for the whole TEMP lifetime so `DROP`
    /// of those tables still runs while writes are allowed.
    pub fn allow_temp_writes(&self) -> Result<AllowTempWrites<'_>, DbError> {
        if self.is_write {
            return Ok(AllowTempWrites {
                conn: self,
                restore_query_only: false,
            });
        }
        self.execute("PRAGMA query_only = 0", [])
            .map_err(DbError::SetQueryOnly)?;
        Ok(AllowTempWrites {
            conn: self,
            restore_query_only: true,
        })
    }

    /// Drop the underlying SQLite connection without returning it to the pool.
    ///
    /// Use when the connection may have request-scoped state (tighter
    /// `sqlite3_limit` values, progress handlers) that must not leak to later
    /// checkouts.
    pub fn close(&mut self) {
        let _ = self.connection.take();
    }
}

/// Restores `PRAGMA query_only = 1` when dropped (see [`Connection::allow_temp_writes`]).
pub struct AllowTempWrites<'a> {
    conn: &'a Connection,
    restore_query_only: bool,
}

impl Drop for AllowTempWrites<'_> {
    fn drop(&mut self) {
        if self.restore_query_only {
            let _ = self.conn.execute("PRAGMA query_only = 1", []);
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        let Some(connection) = self.connection.take() else {
            return;
        };
        return_to_cache(
            &self.cache_ref,
            Connection {
                connection: Some(connection),
                is_write: self.is_write,
                cache_ref: self.cache_ref.clone(),
            },
        );
    }
}

pub fn start_db_cache_expiry_task_for(server_context: Arc<ServerContext>) {
    if server_context
        .cache_expiry_task_started
        .swap(true, Ordering::SeqCst)
    {
        return;
    }

    crate::task::spawn_blocking(server_context.clone(), move || {
        while !server_context
            .wait_for_shutdown_or_timeout(Duration::from_secs(PROJECT_DB_CACHE_SWEEP_INTERVAL_SECS))
        {
            expire_idle_project_db_connections(&server_context);
        }
    });
}

/// Open a project SQLite database for reading.
///
/// The project must already be in [`ServerContext::project_cache`] (via
/// [`insert_project`], [`crate::projects::fetch_project`], or
/// [`crate::projects::resolve_project`]).
pub fn open_project_database_read(
    server_context: &ServerContext,
    project_id: &str,
) -> Result<Connection, DbError> {
    let project = cached_project(server_context, project_id)?;
    open_project_database_read_for(server_context, &project)
}

pub fn open_project_database_read_for(
    server_context: &ServerContext,
    project: &Arc<ProjectCache>,
) -> Result<Connection, DbError> {
    project.refresh_db_expiry();
    let cache_ref = cache_ref_for(server_context, project);
    let open_target = project_open_target(server_context, &project.id_string());
    checkout_read_connection(server_context, &cache_ref, &open_target)
}

/// Open a project SQLite database for writing. See [`open_project_database_read`].
pub fn open_project_database_write(
    server_context: &ServerContext,
    project_id: &str,
) -> Result<Connection, DbError> {
    let project = cached_project(server_context, project_id)?;
    open_project_database_write_for(server_context, &project)
}

pub fn open_project_database_write_for(
    server_context: &ServerContext,
    project: &Arc<ProjectCache>,
) -> Result<Connection, DbError> {
    project.refresh_db_expiry();
    let cache_ref = cache_ref_for(server_context, project);
    let open_target = project_open_target(server_context, &project.id_string());
    checkout_write_connection(
        server_context,
        &cache_ref,
        &open_target,
        PROJECT_DB_CHECKOUT_TIMEOUT,
    )
}

pub fn try_open_project_database_write(
    server_context: &ServerContext,
    project_id: &str,
) -> Result<Option<Connection>, DbError> {
    let project = cached_project(server_context, project_id)?;
    project.refresh_db_expiry();
    let cache_ref = cache_ref_for(server_context, &project);
    let open_target = project_open_target(server_context, &project.id_string());
    try_checkout_connection(
        server_context,
        &cache_ref,
        &open_target,
        ConnectionKind::Write,
    )
}

fn cached_project(
    server_context: &ServerContext,
    project_id: &str,
) -> Result<Arc<ProjectCache>, DbError> {
    let project_key = decode_project_cache_key(project_id)?;
    get_project(server_context, project_key).ok_or_else(|| {
        DbError::ProjectDatabaseNotFound(format!(
            "project {project_id} is not in project_cache; fetch or insert it first"
        ))
    })
}

/// Create a new project SQLite database, apply migrations, and register it as
/// openable. Errors if a database for `project_id` already exists.
pub fn create_project_database(
    server_context: &ServerContext,
    project_id: &str,
) -> Result<(), DbError> {
    let project_key = decode_project_cache_key(project_id)?;
    if is_project_deleted(server_context, project_key) {
        return Err(DbError::ProjectDatabaseNotFound(project_id.to_string()));
    }
    if server_context.created_project_dbs.contains(&project_key) {
        return Err(DbError::ProjectDatabaseAlreadyExists(
            project_id.to_string(),
        ));
    }
    let open_target = project_open_target(server_context, project_id);
    if let ProjectOpenTarget::Filesystem { db_path, .. } = &open_target
        && db_path.exists()
    {
        return Err(DbError::ProjectDatabaseAlreadyExists(
            project_id.to_string(),
        ));
    }

    let raw = create_database(&open_target)?;
    if let Err(error) = crate::db::migrations::apply_missing_project_migrations(&raw) {
        // Best-effort cleanup so a failed create does not leave a half-built DB.
        drop(raw);
        let _ = remove_project_database_files(server_context, project_id);
        return Err(DbError::ApplyMigrations(error));
    }

    let project = get_project(server_context, project_key).ok_or_else(|| {
        DbError::ProjectDatabaseNotFound(format!(
            "project {project_id} must be inserted into project_cache before create"
        ))
    })?;
    let cache_ref = cache_ref_for(server_context, &project);
    {
        let mut inner = project.db.inner.lock();
        inner.migrations_up_to_date = true;
        if let Some(mut previous) = inner.idle.take() {
            // Avoid Drop -> return_to_cache while this lock is held.
            previous.close();
        }
        inner.idle = Some(Connection {
            connection: Some(raw),
            is_write: false,
            cache_ref: cache_ref.clone(),
        });
    }
    server_context.created_project_dbs.insert(project_key);
    info!("Created project database for project {project_id}");
    Ok(())
}

/// Remove a project SQLite database (filesystem files and/or registration).
/// Used to roll back signup when a later step fails, and to delete a project.
pub fn remove_project_database(
    server_context: &ServerContext,
    project_id: &str,
) -> Result<(), DbError> {
    let project_key = decode_project_cache_key(project_id)?;
    evict_project(server_context, project_key);
    server_context
        .download_snapshots
        .remove_project(project_key);
    remove_project_database_files(server_context, project_id)
}

/// Removes a deleted project's database for good: unlike
/// [`remove_project_database`], nothing still holding the project can open it
/// into existence again or put it back in the caches.
pub(crate) fn delete_project_database(
    server_context: &ServerContext,
    project_id: &str,
) -> Result<(), DbError> {
    let project_key = decode_project_cache_key(project_id)?;
    server_context.deleted_project_ids.insert(project_key);
    remove_project_database(server_context, project_id)
}

/// Drops the project's identity and idle connection from the caches.
fn evict_project(server_context: &ServerContext, project_key: i32) {
    let removed = server_context.project_cache.remove(&project_key);
    server_context.created_project_dbs.remove(&project_key);
    if let Some((_, project)) = removed {
        server_context
            .project_name_cache
            .remove_if(project.name(), |_, cached| Arc::ptr_eq(cached, &project));
        let idle = {
            let mut inner = project.db.inner.lock();
            inner.acquired_for_write = false;
            inner.idle.take()
        };
        // Cache entry is gone, so Drop closes without re-inserting.
        drop(idle);
    }
}

fn remove_project_database_files(
    server_context: &ServerContext,
    project_id: &str,
) -> Result<(), DbError> {
    match server_context.project_db_backend() {
        ProjectDbBackend::Filesystem { data_dir } => {
            let db_path = named_database_path(data_dir, project_id);
            // The WAL can hold rows not yet checkpointed into the main file.
            let mut wal_path = db_path.clone().into_os_string();
            wal_path.push("-wal");
            let mut shm_path = db_path.clone().into_os_string();
            shm_path.push("-shm");
            for path in [db_path, wal_path.into(), shm_path.into()] {
                match std::fs::remove_file(&path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(DbError::RemoveProjectDatabase(error)),
                }
            }
            Ok(())
        }
        ProjectDbBackend::InMemory { .. } => Ok(()),
    }
}

fn cache_ref_for(server_context: &ServerContext, project: &Arc<ProjectCache>) -> CacheRef {
    CacheRef {
        identity: CacheIdentity::Project {
            project_key: project.id,
            project_caches: server_context.project_cache.clone(),
        },
        project: Arc::clone(project),
    }
}

fn project_cache_entry_is_active(
    project_caches: &DashMap<i32, Arc<ProjectCache>>,
    project_key: i32,
    project: &Arc<ProjectCache>,
) -> bool {
    project_caches
        .get(&project_key)
        .map(|entry| Arc::ptr_eq(entry.value(), project))
        .unwrap_or(false)
}

fn return_to_cache(cache_ref: &CacheRef, mut connection: Connection) {
    let is_write = connection.is_write;
    if !cache_ref.is_active() {
        connection.close();
        return;
    }
    let mut inner = cache_ref.db().inner.lock();
    if inner.idle.is_none() {
        connection.is_write = false;
        inner.idle = Some(connection);
    } else {
        connection.close();
    }
    if is_write {
        inner.acquired_for_write = false;
        drop(inner);
        cache_ref.db().write_available.notify_one();
    }
}

/// Check out a read-only connection to the project database at `open_target`.
///
/// Opens a new SQLite connection when the cache has no idle connection or a
/// write connection is currently checked out (reads are not blocked on write).
/// First-load reads wait while another checkout is applying migrations.
fn checkout_read_connection(
    server_context: &ServerContext,
    cache_ref: &CacheRef,
    open_target: &ProjectOpenTarget<'_>,
) -> Result<Connection, DbError> {
    let deadline = Instant::now() + PROJECT_DB_CHECKOUT_TIMEOUT;
    loop {
        match try_checkout_connection(server_context, cache_ref, open_target, ConnectionKind::Read)?
        {
            Some(connection) => return Ok(connection),
            None => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(DbError::ProjectDbCheckoutTimeout);
                }
                let mut inner = cache_ref.db().inner.lock();
                if !inner.migrations_up_to_date {
                    let _ = cache_ref
                        .db()
                        .write_available
                        .wait_for(&mut inner, remaining);
                }
            }
        }
    }
}

/// Check out an exclusive write connection to the project database at `open_target`.
///
/// Blocks on the cache condvar until the write slot is free or `timeout` elapses.
fn checkout_write_connection(
    server_context: &ServerContext,
    cache_ref: &CacheRef,
    open_target: &ProjectOpenTarget<'_>,
    timeout: Duration,
) -> Result<Connection, DbError> {
    let deadline = Instant::now() + timeout;
    loop {
        match try_checkout_connection(
            server_context,
            cache_ref,
            open_target,
            ConnectionKind::Write,
        )? {
            Some(connection) => return Ok(connection),
            None => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(DbError::ProjectDbCheckoutTimeout);
                }
                let mut inner = cache_ref.db().inner.lock();
                if inner.acquired_for_write {
                    let _ = cache_ref
                        .db()
                        .write_available
                        .wait_for(&mut inner, remaining);
                }
            }
        }
    }
}

/// Try to check out a connection without blocking.
///
/// Returns `Ok(None)` when the exclusive write slot is held and this checkout
/// needs it: a write, or a read that still has to apply migrations. After
/// migrations are current, reads reuse a cached idle connection or open a new
/// one even while a write is checked out.
fn try_checkout_connection(
    server_context: &ServerContext,
    cache_ref: &CacheRef,
    open_target: &ProjectOpenTarget<'_>,
    kind: ConnectionKind,
) -> Result<Option<Connection>, DbError> {
    let is_write = matches!(kind, ConnectionKind::Write);

    let (maybe_cached, migrations_up_to_date) = {
        let mut inner = cache_ref.db().inner.lock();
        let migrations_up_to_date = inner.migrations_up_to_date;
        if is_write || !migrations_up_to_date {
            if inner.acquired_for_write {
                return Ok(None);
            }
            inner.acquired_for_write = true;
        }
        (inner.idle.take(), migrations_up_to_date)
    };
    let held_exclusive = is_write || !migrations_up_to_date;

    let mut result: Result<Option<Connection>, DbError> = match maybe_cached {
        None => open_or_create_database(server_context, open_target, kind).map(|raw| {
            Some(Connection {
                connection: Some(raw),
                is_write,
                cache_ref: cache_ref.clone(),
            })
        }),
        Some(mut cached) => {
            cached.is_write = is_write;
            Ok(Some(cached))
        }
    };

    if let Ok(Some(connection)) = result {
        result =
            ensure_connection_ready(cache_ref.db(), &connection, is_write, migrations_up_to_date)
                .map(|()| Some(connection));
    }

    if result.is_err() && held_exclusive {
        reset_write_after_failed_open(cache_ref.db());
    }

    result
}

fn reset_write_after_failed_open(cache: &ProjectDbCache) {
    let mut inner = cache.inner.lock();
    inner.acquired_for_write = false;
    drop(inner);
    cache.write_available.notify_all();
}

/// Create a new SQLite database file / shared-memory DB. Callers must ensure it
/// does not already exist.
fn create_database(open_target: &ProjectOpenTarget<'_>) -> Result<RawConnection, DbError> {
    match open_target {
        ProjectOpenTarget::Filesystem { data_dir, db_path } => {
            std::fs::create_dir_all(data_dir).map_err(DbError::CreateDataDir)?;
            let conn = rusqlite::Connection::open(db_path).map_err(DbError::Open)?;
            prepare_new_connection(&conn)?;
            info!("Created project database at {}", db_path.display());
            Ok(conn)
        }
        ProjectOpenTarget::InMemory { uri, project_id } => {
            let conn = rusqlite::Connection::open(uri.as_str()).map_err(DbError::Open)?;
            prepare_new_connection(&conn)?;
            info!("Created in-memory project database for project {project_id}");
            Ok(conn)
        }
    }
}

/// Open an existing project database, or create an empty one when missing.
/// Migrations are applied by [`ensure_connection_ready`] on first checkout.
fn open_or_create_database(
    server_context: &ServerContext,
    open_target: &ProjectOpenTarget<'_>,
    kind: ConnectionKind,
) -> Result<RawConnection, DbError> {
    match open_database(server_context, open_target, kind) {
        Ok(conn) => Ok(conn),
        Err(DbError::ProjectDatabaseNotFound(missing)) => {
            let project_key = open_target_project_key(open_target)?;
            if is_project_deleted(server_context, project_key) {
                return Err(DbError::ProjectDatabaseNotFound(missing));
            }
            let conn = create_database(open_target)?;
            server_context.created_project_dbs.insert(project_key);
            Ok(conn)
        }
        Err(error) => Err(error),
    }
}

/// Open an existing project database. Does not create missing databases.
fn open_database(
    server_context: &ServerContext,
    open_target: &ProjectOpenTarget<'_>,
    kind: ConnectionKind,
) -> Result<RawConnection, DbError> {
    match open_target {
        ProjectOpenTarget::Filesystem { db_path, .. } => {
            // SQLITE_OPEN_CREATE is intentionally omitted so missing files error.
            // Always READ_WRITE at the SQLite level: pooled connections are reused
            // for later write checkouts even if this open was for a read.
            let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
            let conn = rusqlite::Connection::open_with_flags(db_path, flags).map_err(|error| {
                if !db_path.exists() {
                    DbError::ProjectDatabaseNotFound(db_path.display().to_string())
                } else {
                    DbError::Open(error)
                }
            })?;
            prepare_new_connection(&conn)?;
            info!(
                "Opened project database for {kind} at {}",
                db_path.display()
            );
            Ok(conn)
        }
        ProjectOpenTarget::InMemory { uri, project_id } => {
            let project_key = decode_project_cache_key(project_id)?;
            if !server_context.created_project_dbs.contains(&project_key) {
                return Err(DbError::ProjectDatabaseNotFound(project_id.to_string()));
            }
            let conn = rusqlite::Connection::open(uri.as_str()).map_err(DbError::Open)?;
            prepare_new_connection(&conn)?;
            info!("Opened in-memory project database for {kind} at {uri}");
            Ok(conn)
        }
    }
}

fn open_target_project_key(open_target: &ProjectOpenTarget<'_>) -> Result<i32, DbError> {
    let project_id = match open_target {
        ProjectOpenTarget::Filesystem { db_path, .. } => db_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or_else(|| DbError::InvalidProjectId(db_path.display().to_string()))?,
        ProjectOpenTarget::InMemory { project_id, .. } => project_id.as_str(),
    };
    decode_project_cache_key(project_id)
}

fn prepare_new_connection(conn: &RawConnection) -> Result<(), DbError> {
    conn.set_prepared_statement_cache_capacity(PREPARED_STATEMENT_CACHE_CAPACITY);
    harden_connection(conn)
}

/// Lock down a freshly opened project DB connection against ATTACH, extension
/// loading, and SQL that can corrupt the file via ordinary statements.
fn harden_connection(conn: &RawConnection) -> Result<(), DbError> {
    // main+temp already exist; SQLITE_LIMIT_ATTACHED=0 rejects further ATTACH.
    conn.set_limit(Limit::SQLITE_LIMIT_ATTACHED, 0)
        .map_err(DbError::HardenConnection)?;

    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)
        .map_err(DbError::HardenConnection)?;

    // SQLITE_DBCONFIG_ENABLE_LOAD_EXTENSION is not in rusqlite's DbConfig enum.
    let mut enabled = 1;
    let rc = unsafe {
        ffi::sqlite3_db_config(
            conn.handle(),
            ffi::SQLITE_DBCONFIG_ENABLE_LOAD_EXTENSION,
            0, // disable C API and SQL load_extension()
            &mut enabled,
        )
    };
    if rc != ffi::SQLITE_OK {
        return Err(DbError::HardenConnection(rusqlite::Error::SqliteFailure(
            ffi::Error::new(rc),
            Some("failed to disable load_extension".into()),
        )));
    }
    debug_assert_eq!(enabled, 0, "load_extension should be disabled");
    if enabled != 0 {
        return Err(DbError::HardenConnection(rusqlite::Error::SqliteFailure(
            ffi::Error::new(ffi::SQLITE_ERROR),
            Some("load_extension remained enabled after disable".into()),
        )));
    }

    Ok(())
}

enum ProjectOpenTarget<'a> {
    Filesystem {
        data_dir: &'a Path,
        db_path: PathBuf,
    },
    InMemory {
        uri: String,
        project_id: String,
    },
}

fn project_open_target<'a>(
    server_context: &'a ServerContext,
    project_id: &str,
) -> ProjectOpenTarget<'a> {
    match server_context.project_db_backend() {
        ProjectDbBackend::Filesystem { data_dir } => ProjectOpenTarget::Filesystem {
            data_dir,
            db_path: named_database_path(data_dir, project_id),
        },
        ProjectDbBackend::InMemory { namespace } => ProjectOpenTarget::InMemory {
            uri: in_memory_database_uri(namespace, project_id),
            project_id: project_id.to_string(),
        },
    }
}

fn in_memory_database_uri(namespace: &str, project_id: &str) -> String {
    format!("file:{namespace}-{project_id}?mode=memory&cache=shared")
}

fn ensure_connection_ready(
    cache: &ProjectDbCache,
    conn: &RawConnection,
    is_write: bool,
    migrations_up_to_date: bool,
) -> Result<(), DbError> {
    if !migrations_up_to_date {
        crate::db::migrations::apply_missing_project_migrations(conn)
            .map_err(DbError::ApplyMigrations)?;
        mark_migrations_up_to_date(cache, is_write);
    }

    apply_connection_pragmas(conn, is_write)?;

    Ok(())
}

fn mark_migrations_up_to_date(cache: &ProjectDbCache, is_write: bool) {
    let mut inner = cache.inner.lock();
    inner.migrations_up_to_date = true;
    if !is_write {
        inner.acquired_for_write = false;
    }
    drop(inner);
    cache.write_available.notify_all();
}

fn apply_connection_pragmas(conn: &RawConnection, is_write: bool) -> Result<(), DbError> {
    #[cfg(debug_assertions)]
    conn.execute("PRAGMA foreign_keys = ON", [])
        .map_err(DbError::EnableForeignKeys)?;
    let sql = if is_write {
        "PRAGMA query_only = 0"
    } else {
        "PRAGMA query_only = 1"
    };
    conn.execute(sql, []).map_err(DbError::SetQueryOnly)?;
    Ok(())
}

fn decode_project_cache_key(project_id: &str) -> Result<i32, DbError> {
    parse_project_id(project_id)
}

/// Parse a non-negative project id used by API paths and sqlite filenames.
pub fn parse_project_id(project_id: &str) -> Result<i32, DbError> {
    crate::projects::parse_project_id_str(project_id)
        .ok_or_else(|| DbError::InvalidProjectId(project_id.to_string()))
}

/// Close idle SQLite connections whose TTL has elapsed. Keeps [`ProjectCache`]
/// entries (and their name/account identity) in the map.
fn expire_idle_project_db_connections(server_context: &ServerContext) {
    let now = seconds_since_epoch();
    for entry in server_context.project_cache.iter() {
        entry.value().db.expire_idle_if_due(now);
    }
}

pub fn project_database_path(server_context: &ServerContext, project_id: &str) -> PathBuf {
    match server_context.project_db_backend() {
        ProjectDbBackend::Filesystem { data_dir } => named_database_path(data_dir, project_id),
        ProjectDbBackend::InMemory { namespace } => {
            PathBuf::from(in_memory_database_uri(namespace, project_id))
        }
    }
}

fn named_database_path(data_dir: &Path, name: &str) -> PathBuf {
    data_dir.join(format!("{name}.db"))
}

/// Standalone in-memory [`Connection`] for unit tests (not checked out from the pool).
#[cfg(test)]
pub(crate) fn in_memory_test_connection() -> Connection {
    let project_key = 0_i32;
    let project_caches = Arc::new(DashMap::new());
    let project = Arc::new(ProjectCache::new_with_db(
        project_key,
        "test".to_string(),
        1,
        ProjectDbCache::new_non_expiring(),
    ));
    project_caches.insert(project_key, project.clone());
    let cache_ref = CacheRef {
        identity: CacheIdentity::Project {
            project_key,
            project_caches,
        },
        project,
    };
    let connection = rusqlite::Connection::open_in_memory().expect("open in-memory connection");
    connection.set_prepared_statement_cache_capacity(PREPARED_STATEMENT_CACHE_CAPACITY);
    Connection {
        connection: Some(connection),
        is_write: true,
        cache_ref,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::TestDataDir;
    use std::path::Path;

    fn make_test_server_context(prefix: &str) -> (ServerContext, TestDataDir) {
        let test_dir = TestDataDir::new(prefix);
        let server_context = ServerContext::new(test_dir.path().to_path_buf());
        (server_context, test_dir)
    }

    fn make_test_connection() -> RawConnection {
        let conn = rusqlite::Connection::open_in_memory().expect("open in-memory connection");
        conn.set_prepared_statement_cache_capacity(PREPARED_STATEMENT_CACHE_CAPACITY);
        conn
    }

    fn make_idle_connection(cache_ref: &CacheRef) -> Connection {
        Connection {
            connection: Some(make_test_connection()),
            is_write: false,
            cache_ref: cache_ref.clone(),
        }
    }

    fn filesystem_open_target<'a>(
        server_context: &'a ServerContext,
        project_id: &str,
    ) -> ProjectOpenTarget<'a> {
        project_open_target(server_context, project_id)
    }

    #[test]
    fn builds_database_path_under_data_dir() {
        assert_eq!(
            named_database_path(Path::new("data"), "demo"),
            Path::new("data/demo.db")
        );
    }

    #[test]
    fn in_memory_uri_uses_shared_cache() {
        assert_eq!(
            in_memory_database_uri("test-ns", "1"),
            "file:test-ns-1?mode=memory&cache=shared"
        );
    }

    #[test]
    fn decodes_project_id_decimal_to_i32() {
        assert_eq!(decode_project_cache_key("0").unwrap(), 0);
        assert_eq!(decode_project_cache_key("15").unwrap(), 15);
        assert!(decode_project_cache_key("not-a-number").is_err());
        assert!(decode_project_cache_key("-1").is_err());
    }

    #[test]
    fn parse_project_id_accepts_non_negative_decimals() {
        assert_eq!(parse_project_id("0").unwrap(), 0);
        assert_eq!(parse_project_id("1").unwrap(), 1);
        assert!(parse_project_id("-3").is_err());
    }

    fn insert_test_project(
        project_caches: &DashMap<i32, Arc<ProjectCache>>,
        id: i32,
        db: ProjectDbCache,
    ) -> Arc<ProjectCache> {
        let project = Arc::new(ProjectCache::new_with_db(
            id,
            format!("project_{id}"),
            1,
            db,
        ));
        project_caches.insert(id, project.clone());
        project
    }

    fn seed_project(server_context: &ServerContext, project_id: &str) -> Arc<ProjectCache> {
        let project_key = decode_project_cache_key(project_id).unwrap();
        insert_project(
            server_context,
            project_key,
            format!("project_{project_key}"),
            1,
        )
    }

    #[test]
    fn try_open_write_returns_none_when_busy() {
        let (server_context, _test_dir) = make_test_server_context("write-busy");
        let project = Arc::new(ProjectCache::new_with_db(
            0,
            "project_0".to_string(),
            1,
            ProjectDbCache::new_non_expiring(),
        ));
        project.db.inner.lock().acquired_for_write = true;
        let cache_ref = CacheRef {
            identity: CacheIdentity::Project {
                project_key: 0,
                project_caches: server_context.project_cache.clone(),
            },
            project,
        };
        let second = try_checkout_connection(
            &server_context,
            &cache_ref,
            &filesystem_open_target(&server_context, "irrelevant"),
            ConnectionKind::Write,
        )
        .expect("try open should not error");
        assert!(second.is_none(), "try-open should return None while in use");
    }

    #[test]
    fn try_read_returns_none_while_migrations_pending() {
        let (server_context, _test_dir) = make_test_server_context("read-migrate-busy");
        let project = Arc::new(ProjectCache::new_with_db(
            0,
            "project_0".to_string(),
            1,
            ProjectDbCache::new_non_expiring(),
        ));
        project.db.inner.lock().acquired_for_write = true;
        let cache_ref = CacheRef {
            identity: CacheIdentity::Project {
                project_key: 0,
                project_caches: server_context.project_cache.clone(),
            },
            project,
        };
        let second = try_checkout_connection(
            &server_context,
            &cache_ref,
            &filesystem_open_target(&server_context, "irrelevant"),
            ConnectionKind::Read,
        )
        .expect("try read should not error");
        assert!(
            second.is_none(),
            "first-load read should wait while another checkout applies migrations"
        );
    }

    #[test]
    fn first_read_checkout_releases_write_slot_after_migrate() {
        let (server_context, _test_dir) = make_test_server_context("read-migrate-slot");
        let project = seed_project(&server_context, "1");
        let conn = open_project_database_read(&server_context, "1").expect("read");
        let inner = project.db.inner.lock();
        assert!(inner.migrations_up_to_date);
        assert!(
            !inner.acquired_for_write,
            "read must not keep the write slot after migrate"
        );
        drop(inner);
        drop(conn);
    }

    #[test]
    fn first_write_checkout_keeps_write_slot_after_migrate() {
        let (server_context, _test_dir) = make_test_server_context("write-migrate-slot");
        let project = seed_project(&server_context, "1");
        let conn = open_project_database_write(&server_context, "1").expect("write");
        let inner = project.db.inner.lock();
        assert!(inner.migrations_up_to_date);
        assert!(inner.acquired_for_write);
        drop(inner);
        drop(conn);
        assert!(!project.db.inner.lock().acquired_for_write);
    }

    #[test]
    fn read_checkout_proceeds_while_write_held_after_migrations() {
        let (server_context, _test_dir) = make_test_server_context("read-during-write");
        seed_project(&server_context, "1");
        let write = open_project_database_write(&server_context, "1").expect("write");
        let read = open_project_database_read(&server_context, "1").expect("read");
        read.query_row("SELECT 1", [], |row| row.get::<_, i32>(0))
            .expect("query");
        drop(read);
        drop(write);
    }

    #[test]
    fn concurrent_first_reads_serialize_migrations() {
        let (server_context, _test_dir) = make_test_server_context("concurrent-migrate");
        seed_project(&server_context, "1");
        let server_context = Arc::new(server_context);

        std::thread::scope(|scope| {
            let mut joins = Vec::new();
            for _ in 0..8 {
                let server_context = server_context.clone();
                joins.push(scope.spawn(move || {
                    let conn = open_project_database_read(&server_context, "1").expect("read");
                    conn.query_row("SELECT 1", [], |row| row.get::<_, i32>(0))
                        .expect("query");
                }));
            }
            for join in joins {
                join.join().expect("thread");
            }
        });

        let project = get_project(&server_context, 1).expect("project");
        let inner = project.db.inner.lock();
        assert!(inner.migrations_up_to_date);
        assert!(!inner.acquired_for_write);
    }

    #[test]
    fn returning_read_connections_keeps_at_most_one_idle() {
        let project_key = 42_i32;
        let project_caches = DashMap::new();
        let project = insert_test_project(
            &project_caches,
            project_key,
            ProjectDbCache::new_non_expiring(),
        );
        let cache_ref = CacheRef {
            identity: CacheIdentity::Project {
                project_key,
                project_caches: Arc::new(project_caches),
            },
            project: project.clone(),
        };
        return_to_cache(&cache_ref, make_idle_connection(&cache_ref));
        return_to_cache(&cache_ref, make_idle_connection(&cache_ref));
        assert!(project.db.inner.lock().idle.is_some());
    }

    #[test]
    fn returning_write_to_removed_project_cache_is_ignored() {
        let (server_context, _test_dir) = make_test_server_context("removed-write-cache");
        let project = seed_project(&server_context, "1");
        let project_key = project.id();
        let cache_ref = CacheRef {
            identity: CacheIdentity::Project {
                project_key,
                project_caches: server_context.project_cache.clone(),
            },
            project: project.clone(),
        };
        let _ = server_context.project_cache.remove(&project_key);
        return_to_cache(
            &cache_ref,
            Connection {
                connection: Some(make_test_connection()),
                is_write: true,
                cache_ref: cache_ref.clone(),
            },
        );
        let inner = project.db.inner.lock();
        assert!(
            !inner.acquired_for_write && inner.idle.is_none(),
            "inactive project cache should not receive returned write connection"
        );
    }

    #[test]
    fn cached_write_can_be_taken_without_opening_new_database() {
        let (server_context, _test_dir) = make_test_server_context("cached-write");
        let project = Arc::new(ProjectCache::new_with_db(
            0,
            "project_0".to_string(),
            1,
            ProjectDbCache::new_non_expiring(),
        ));
        let cache_ref = CacheRef {
            identity: CacheIdentity::Project {
                project_key: 0,
                project_caches: server_context.project_cache.clone(),
            },
            project: project.clone(),
        };
        project.db.inner.lock().idle = Some(make_idle_connection(&cache_ref));
        let write = try_checkout_connection(
            &server_context,
            &cache_ref,
            &filesystem_open_target(&server_context, "irrelevant"),
            ConnectionKind::Write,
        )
        .expect("try open should not error");
        assert!(write.is_some());
        assert!(project.db.inner.lock().acquired_for_write);
    }

    #[test]
    fn removing_project_entry_prevents_read_return_to_cache() {
        let (server_context, _test_dir) = make_test_server_context("removed-read-cache");
        let project = seed_project(&server_context, "2");
        let project_key = project.id();
        let cache_ref = CacheRef {
            identity: CacheIdentity::Project {
                project_key,
                project_caches: server_context.project_cache.clone(),
            },
            project: project.clone(),
        };
        let _ = server_context.project_cache.remove(&project_key);
        return_to_cache(&cache_ref, make_idle_connection(&cache_ref));
        assert!(project.db.inner.lock().idle.is_none());
    }

    #[test]
    fn prepare_cached_reuses_same_statement() {
        let project_key = 1_i32;
        let project_caches = DashMap::new();
        let project = insert_test_project(
            &project_caches,
            project_key,
            ProjectDbCache::new_non_expiring(),
        );
        let cache_ref = CacheRef {
            identity: CacheIdentity::Project {
                project_key,
                project_caches: Arc::new(project_caches),
            },
            project,
        };
        let conn = make_idle_connection(&cache_ref);
        assert_eq!(
            conn.prepare_cached("SELECT 1")
                .unwrap()
                .query_row([], |row| row.get::<_, i32>(0)),
            Ok(1)
        );
        assert_eq!(
            conn.prepare_cached("SELECT 1")
                .unwrap()
                .query_row([], |row| row.get::<_, i32>(0)),
            Ok(1)
        );
    }

    #[test]
    fn cached_execute_reuses_prepared_statements() {
        let project_key = 2_i32;
        let project_caches = DashMap::new();
        let project = insert_test_project(
            &project_caches,
            project_key,
            ProjectDbCache::new_non_expiring(),
        );
        let cache_ref = CacheRef {
            identity: CacheIdentity::Project {
                project_key,
                project_caches: Arc::new(project_caches),
            },
            project,
        };
        let conn = make_idle_connection(&cache_ref);
        conn.execute("CREATE TEMP TABLE probe (id INTEGER)", [])
            .unwrap();
        assert_eq!(
            conn.query_row("SELECT 1", [], |row| row.get::<_, i32>(0))
                .unwrap(),
            1
        );
        conn.with_query("SELECT 1", [], |_| Ok(())).unwrap();
        assert_eq!(
            conn.query_row("SELECT 2", [], |row| row.get::<_, i32>(0))
                .unwrap(),
            2
        );
    }

    #[test]
    fn write_out_resets_after_failed_open() {
        let (server_context, _test_dir) = make_test_server_context("failed-open");
        let project = Arc::new(ProjectCache::new_with_db(
            0,
            "project_0".to_string(),
            1,
            ProjectDbCache::new_non_expiring(),
        ));
        let cache_ref = CacheRef {
            identity: CacheIdentity::Project {
                project_key: 0,
                project_caches: server_context.project_cache.clone(),
            },
            project: project.clone(),
        };
        let data_dir = server_context.data_dir().expect("filesystem backend");
        let bad_db_path = data_dir.join("missing-parent").join("should-fail.db");
        let open_target = ProjectOpenTarget::Filesystem {
            data_dir,
            db_path: bad_db_path,
        };
        let first_error = checkout_write_connection(
            &server_context,
            &cache_ref,
            &open_target,
            PROJECT_DB_CHECKOUT_TIMEOUT,
        )
        .err();
        assert!(first_error.is_some());
        assert!(!project.db.inner.lock().acquired_for_write);
    }

    #[test]
    fn open_creates_missing_filesystem_database() {
        let (server_context, _test_dir) = make_test_server_context("missing-db");
        seed_project(&server_context, "1");
        let conn = open_project_database_read(&server_context, "1").expect("create on open");
        conn.query_row("SELECT 1", [], |row| row.get::<_, i32>(0))
            .expect("query newly created db");
        assert!(
            project_database_path(&server_context, "1").exists(),
            "sqlite file should exist after first open"
        );
    }

    #[test]
    fn open_errors_when_project_not_in_cache() {
        let (server_context, _test_dir) = make_test_server_context("not-cached");
        let err = open_project_database_read(&server_context, "1").unwrap_err();
        assert!(
            matches!(err, DbError::ProjectDatabaseNotFound(_)),
            "expected not found, got {err}"
        );
    }

    #[test]
    fn create_then_open_filesystem_database() {
        let (server_context, _test_dir) = make_test_server_context("create-then-open");
        seed_project(&server_context, "1");
        create_project_database(&server_context, "1").expect("create project db");
        let conn = open_project_database_read(&server_context, "1").expect("open created db");
        conn.query_row("SELECT 1", [], |row| row.get::<_, i32>(0))
            .expect("query created db");
    }

    #[test]
    fn allow_temp_writes_lets_read_checkout_use_temp_tables() {
        let (server_context, _test_dir) = make_test_server_context("temp-writes");
        seed_project(&server_context, "1");
        create_project_database(&server_context, "1").expect("create project db");
        let conn = open_project_database_read(&server_context, "1").expect("open read");

        assert!(
            conn.execute("CREATE TEMP TABLE blocked (id INTEGER)", [])
                .is_err(),
            "query_only should block TEMP DDL"
        );

        {
            let _allow = conn.allow_temp_writes().expect("allow temp writes");
            conn.execute("CREATE TEMP TABLE scratch (id INTEGER PRIMARY KEY)", [])
                .expect("TEMP DDL while allowed");
            conn.execute("INSERT INTO scratch (id) VALUES (1)", [])
                .expect("TEMP DML while allowed");
            assert_eq!(
                conn.query_row("SELECT id FROM scratch", [], |row| row.get::<_, i64>(0))
                    .expect("read TEMP"),
                1
            );
        }

        assert!(
            conn.execute("CREATE TEMP TABLE blocked_again (id INTEGER)", [])
                .is_err(),
            "query_only should be restored after guard drops"
        );
    }

    #[test]
    fn hardens_opened_project_connections() {
        let (server_context, _test_dir) = make_test_server_context("harden");
        seed_project(&server_context, "1");
        create_project_database(&server_context, "1").expect("create project db");
        let conn = open_project_database_write(&server_context, "1").expect("open write db");

        assert_eq!(
            conn.limit(Limit::SQLITE_LIMIT_ATTACHED)
                .expect("read attached limit"),
            0
        );
        assert!(
            conn.db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE)
                .expect("read defensive"),
            "defensive mode should be on"
        );
        assert!(
            conn.execute("ATTACH DATABASE ':memory:' AS other", [])
                .is_err(),
            "ATTACH should be denied"
        );

        let mut enabled = 1;
        let rc = unsafe {
            ffi::sqlite3_db_config(
                conn.handle(),
                ffi::SQLITE_DBCONFIG_ENABLE_LOAD_EXTENSION,
                -1, // leave unchanged; only report
                &mut enabled,
            )
        };
        assert_eq!(rc, ffi::SQLITE_OK);
        assert_eq!(enabled, 0, "load_extension should be disabled");
    }

    #[test]
    fn close_does_not_return_the_connection_to_the_pool() {
        let (server_context, _test_dir) = make_test_server_context("close-no-return");
        let project = seed_project(&server_context, "1");
        create_project_database(&server_context, "1").expect("create project db");
        assert!(
            project.db.inner.lock().idle.is_some(),
            "create leaves an idle connection"
        );

        let mut conn = open_project_database_read(&server_context, "1").expect("checkout");
        assert!(
            project.db.inner.lock().idle.is_none(),
            "checkout should take the idle connection"
        );
        conn.close();
        assert!(
            project.db.inner.lock().idle.is_none(),
            "close must not return the connection to the pool"
        );
    }

    #[test]
    fn create_errors_when_filesystem_database_exists() {
        let (server_context, _test_dir) = make_test_server_context("create-exists");
        seed_project(&server_context, "1");
        create_project_database(&server_context, "1").expect("create project db");
        let err = create_project_database(&server_context, "1").unwrap_err();
        assert!(
            matches!(err, DbError::ProjectDatabaseAlreadyExists(_)),
            "expected already exists, got {err}"
        );
    }

    #[test]
    fn a_deleted_project_cannot_be_recreated_by_a_holder_or_a_late_lookup() {
        let (server_context, test_dir) = make_test_server_context("delete-tombstone");
        let project = seed_project(&server_context, "1");
        create_project_database(&server_context, "1").expect("create project db");
        let db_path = named_database_path(test_dir.path(), "1");
        assert!(db_path.exists());

        delete_project_database(&server_context, "1").expect("delete project db");
        assert!(!db_path.exists());

        // An integration sync that loaded the project before the delete.
        let err = open_project_database_write_for(&server_context, &project).unwrap_err();
        assert!(matches!(err, DbError::ProjectDatabaseNotFound(_)), "{err}");
        assert!(open_project_database_read_for(&server_context, &project).is_err());
        assert!(!db_path.exists(), "opening must not recreate the file");

        // A lookup that read the row before the delete committed.
        insert_project(&server_context, 1, project.name().to_string(), 1);
        assert!(get_project(&server_context, 1).is_none());
        assert!(
            !server_context
                .project_name_cache
                .contains_key(project.name())
        );
        assert!(create_project_database(&server_context, "1").is_err());
        assert!(!db_path.exists());
    }
}
