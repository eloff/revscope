use std::future::Future;
use std::sync::Arc;

use tokio::task::JoinHandle;

use crate::server_context::ServerContext;

pub fn spawn<T>(
    server_context: Arc<ServerContext>,
    future: impl Future<Output = T> + Send + 'static,
) -> JoinHandle<T>
where
    T: Send + 'static,
{
    server_context.active_task_started();
    tokio::spawn(async move {
        let _guard = ActiveTaskGuard { server_context };
        future.await
    })
}

pub fn spawn_blocking<T>(
    server_context: Arc<ServerContext>,
    f: impl FnOnce() -> T + Send + 'static,
) -> JoinHandle<T>
where
    T: Send + 'static,
{
    server_context.active_task_started();
    tokio::task::spawn_blocking(move || {
        let _guard = ActiveTaskGuard { server_context };
        f()
    })
}

struct ActiveTaskGuard {
    server_context: Arc<ServerContext>,
}

impl Drop for ActiveTaskGuard {
    fn drop(&mut self) {
        self.server_context.active_task_finished();
    }
}
