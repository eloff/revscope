mod attribution;
mod cloudflare;
mod dodo;
pub(crate) mod google_auth;
mod gsc;
mod lemon_squeezy;
mod polar;
mod schedule;
mod shopify;
mod square;
mod store;
mod stripe;
mod sync;

pub use sync::start_integration_sync_task;
pub(crate) use sync::{ensure_project_cached, sleep_or_shutdown};

/// Per-request timeout for the report queries GSC and Cloudflare answer with
/// one large page, which can outlast the shared client's default.
pub(crate) const REPORT_QUERY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
