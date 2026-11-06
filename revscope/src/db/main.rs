use std::ops::Deref;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use log::{error, info};
use parking_lot::Mutex;
use tokio::sync::Notify;
use tokio_postgres::{Client, NoTls};

use crate::db::DbError;
use crate::db::PostgresError;
use crate::db::migrations::apply_missing_main_postgres_migrations;
use crate::server_context::ServerContext;

pub const MAX_POSTGRES_CONNECTIONS: usize = 100;
pub const MAX_IDLE_POSTGRES_CONNECTIONS: usize = 8;

/// Drop pooled connections that have sat unused longer than this.
const MAX_IDLE: Duration = Duration::from_secs(60 * 60);
/// Drop connections older than this even if they were recently used.
const MAX_LIFETIME: Duration = Duration::from_secs(8 * 60 * 60);

#[derive(Debug)]
struct PooledConnection {
    connection: Client,
    created_at: Instant,
    /// Set when the connection is placed in the idle pool.
    idle_since: Instant,
}

impl PooledConnection {
    fn new(connection: Client) -> Self {
        let now = Instant::now();
        Self {
            connection,
            created_at: now,
            idle_since: now,
        }
    }

    fn is_reusable(&self) -> bool {
        !self.connection.is_closed()
            && self.created_at.elapsed() < MAX_LIFETIME
            && self.idle_since.elapsed() < MAX_IDLE
    }

    fn should_return_to_pool(&self) -> bool {
        !self.connection.is_closed() && self.created_at.elapsed() < MAX_LIFETIME
    }
}

#[derive(Debug)]
pub struct MainDbCache {
    connections: Mutex<Vec<PooledConnection>>,
    total_connections: AtomicUsize,
    connection_available: Notify,
}

impl Default for MainDbCache {
    fn default() -> Self {
        Self::new()
    }
}

impl MainDbCache {
    pub fn new() -> Self {
        Self {
            connections: Mutex::new(Vec::new()),
            total_connections: AtomicUsize::new(0),
            connection_available: Notify::new(),
        }
    }

    pub(crate) fn close_idle_connections(&self) {
        let mut idle_connections = self.connections.lock();
        let closed = idle_connections.len();
        idle_connections.clear();
        drop(idle_connections);

        if closed > 0 {
            self.total_connections.fetch_sub(closed, Ordering::AcqRel);
            self.connection_available.notify_waiters();
        }
    }
}

fn discard_connection_slot(cache: &MainDbCache) {
    cache.total_connections.fetch_sub(1, Ordering::AcqRel);
    cache.connection_available.notify_one();
}

pub struct Connection {
    inner: Option<PooledConnection>,
    server_context: Arc<ServerContext>,
    /// Set while [`Connection::in_transaction`] may have a transaction open.
    /// Still set at drop (a panic, or a ROLLBACK that failed) means the server
    /// side may be mid-transaction, so the connection is closed, not pooled.
    transaction_open: AtomicBool,
}

impl Deref for Connection {
    type Target = Client;

    fn deref(&self) -> &Self::Target {
        &self
            .inner
            .as_ref()
            .expect("main db connection lease must hold an open connection")
            .connection
    }
}

impl Connection {
    /// Runs `body` inside a transaction on this connection, committing on `Ok`
    /// and rolling back on `Err`. A connection must not go back to the pool
    /// with a transaction open, so use this rather than a bare `BEGIN`.
    pub(crate) async fn in_transaction<T, E>(
        &self,
        body: impl AsyncFnOnce(&Connection) -> Result<T, E>,
    ) -> Result<T, E>
    where
        E: From<tokio_postgres::Error>,
    {
        self.transaction_open.store(true, Ordering::Release);
        self.batch_execute("BEGIN").await?;
        let result = match body(self).await {
            Ok(value) => self
                .batch_execute("COMMIT")
                .await
                .map(|()| value)
                .map_err(E::from),
            Err(error) => match self.batch_execute("ROLLBACK").await {
                Ok(()) => Err(error),
                Err(rollback_error) => {
                    error!("main postgres rollback failed: {rollback_error}");
                    return Err(error);
                }
            },
        };
        // A failed COMMIT ends the transaction on the server, so the
        // connection is clean either way once COMMIT or ROLLBACK answered.
        self.transaction_open.store(false, Ordering::Release);
        result
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        let Some(mut connection) = self.inner.take() else {
            return;
        };
        let cache = &self.server_context.main_db_cache;
        if self.server_context.shutdown_requested()
            || !connection.should_return_to_pool()
            || self.transaction_open.load(Ordering::Acquire)
        {
            discard_connection_slot(cache);
            return;
        }
        let mut idle_connections = cache.connections.lock();
        if idle_connections.len() < MAX_IDLE_POSTGRES_CONNECTIONS {
            connection.idle_since = Instant::now();
            idle_connections.push(connection);
            drop(idle_connections);
            cache.connection_available.notify_one();
            return;
        }
        drop(idle_connections);
        discard_connection_slot(cache);
    }
}

pub async fn open_main_database_read(
    server_context: Arc<ServerContext>,
) -> Result<Connection, DbError> {
    checkout_connection(server_context).await
}

pub async fn open_main_database_write(
    server_context: Arc<ServerContext>,
) -> Result<Connection, DbError> {
    checkout_connection(server_context).await
}

pub async fn try_open_main_database_write(
    server_context: Arc<ServerContext>,
) -> Result<Option<Connection>, DbError> {
    try_checkout_connection(server_context).await
}

pub async fn apply_main_migrations(server_context: Arc<ServerContext>) -> Result<(), DbError> {
    info!("Running main database migrations");
    let client = open_postgres_connection(server_context).await?;
    apply_missing_main_postgres_migrations(&client).await?;
    info!("Main database migrations completed successfully");
    Ok(())
}

async fn checkout_connection(server_context: Arc<ServerContext>) -> Result<Connection, DbError> {
    let cache = server_context.main_db_cache.clone();
    loop {
        let notified = cache.connection_available.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();

        match try_checkout_connection(server_context.clone()).await? {
            Some(connection) => return Ok(connection),
            None => notified.await,
        }
    }
}

async fn try_checkout_connection(
    server_context: Arc<ServerContext>,
) -> Result<Option<Connection>, DbError> {
    loop {
        let cached = server_context.main_db_cache.connections.lock().pop();
        let Some(cached) = cached else {
            break;
        };
        if cached.is_reusable() {
            return Ok(Some(Connection {
                inner: Some(cached),
                server_context: server_context.clone(),
                transaction_open: AtomicBool::new(false),
            }));
        }
        discard_connection_slot(&server_context.main_db_cache);
    }

    if !try_reserve_connection_slot(&server_context.main_db_cache) {
        return Ok(None);
    }

    let connection = match open_postgres_connection(server_context.clone()).await {
        Ok(connection) => connection,
        Err(error) => {
            discard_connection_slot(&server_context.main_db_cache);
            return Err(error);
        }
    };

    Ok(Some(Connection {
        inner: Some(PooledConnection::new(connection)),
        server_context,
        transaction_open: AtomicBool::new(false),
    }))
}

fn try_reserve_connection_slot(cache: &MainDbCache) -> bool {
    let mut observed = cache.total_connections.load(Ordering::Acquire);
    loop {
        if observed >= MAX_POSTGRES_CONNECTIONS {
            return false;
        }
        match cache.total_connections.compare_exchange_weak(
            observed,
            observed + 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return true,
            Err(actual) => observed = actual,
        }
    }
}

async fn open_postgres_connection(server_context: Arc<ServerContext>) -> Result<Client, DbError> {
    let connection_url = std::env::var("DATABASE_URL")
        .map_err(|_| DbError::MissingMainPostgresUrlEnv("DATABASE_URL"))?;
    let (client, connection) = tokio_postgres::connect(&connection_url, NoTls)
        .await
        .map_err(DbError::main_postgres_connect)?;

    crate::task::spawn(server_context, async move {
        if let Err(error) = connection.await {
            error!(
                "main postgres connection task failed: {}",
                PostgresError::from(error)
            );
        }
    });
    info!("Opened main postgres connection");
    Ok(client)
}
