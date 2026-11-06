//! Caps how many client connections the server holds open at once.
//!
//! Each accepted connection holds a [`ConnectionSlot`] until its task ends, so
//! idle keep-alive connections count against the cap as much as busy ones. A
//! connection accepted over the cap is closed straight away, before any TLS
//! handshake or read, so a flood of idle sockets can't exhaust file descriptors
//! or memory. The plain HTTP and TLS listeners share one count.
//!
//! The cap only protects anything if it is reached before the process runs out
//! of file descriptors, so startup raises the soft descriptor limit to the hard
//! one and lowers the cap when even that can't fit it.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

/// Default for [`MAX_CONNECTIONS_ENV`].
pub(crate) const DEFAULT_MAX_CONNECTIONS: usize = 100_000;

/// Overrides [`DEFAULT_MAX_CONNECTIONS`] with a positive integer.
pub(crate) const MAX_CONNECTIONS_ENV: &str = "REVSCOPE_MAX_CONNECTIONS";

pub(crate) struct ConnectionLimit {
    counter: Arc<Counter>,
    rejected: AtomicU64,
}

struct Counter {
    open: AtomicUsize,
    max: usize,
    /// Set by the first rejection at the cap and cleared once open connections
    /// fall below 90% of it, so each saturation is logged once even though
    /// connections keep closing and being replaced while it lasts.
    saturated: AtomicBool,
}

/// Held for the life of one connection; dropping it frees the slot.
pub(crate) struct ConnectionSlot {
    counter: Arc<Counter>,
}

impl Drop for ConnectionSlot {
    fn drop(&mut self) {
        let counter = &self.counter;
        let open = counter.open.fetch_sub(1, Ordering::AcqRel) - 1;
        if open < counter.max - counter.max / 10 && counter.saturated.load(Ordering::Relaxed) {
            counter.saturated.store(false, Ordering::Relaxed);
        }
    }
}

impl ConnectionLimit {
    pub(crate) fn new(max: usize) -> Self {
        Self {
            counter: Arc::new(Counter {
                open: AtomicUsize::new(0),
                max,
                saturated: AtomicBool::new(false),
            }),
            rejected: AtomicU64::new(0),
        }
    }

    /// Reads [`MAX_CONNECTIONS_ENV`], falling back to the default when it is
    /// unset or not a positive integer, then lowers it to fit the descriptor
    /// limit.
    pub(crate) fn from_env() -> Self {
        let requested = max_from(std::env::var(MAX_CONNECTIONS_ENV).ok().as_deref());
        let soft_limit = open_file_soft_limit();
        let max = fit_to_descriptors(requested, soft_limit);
        if max < requested {
            log::warn!(
                "lowering the connection cap from {requested} to {max} to fit the open file limit of {}; raise the limit (ulimit -n, LimitNOFILE) to allow more",
                soft_limit.unwrap_or_default()
            );
        }
        Self::new(max)
    }

    /// Take a slot, or `None` when the cap is reached and the connection
    /// should be closed.
    pub(crate) fn try_acquire(&self) -> Option<ConnectionSlot> {
        let counter = &self.counter;
        let taken = counter
            .open
            .try_update(Ordering::AcqRel, Ordering::Acquire, |open| {
                (open < counter.max).then_some(open + 1)
            });
        if taken.is_ok() {
            return Some(ConnectionSlot {
                counter: counter.clone(),
            });
        }
        let rejected = self.rejected.fetch_add(1, Ordering::Relaxed) + 1;
        let newly_saturated = !counter.saturated.swap(true, Ordering::Relaxed);
        // A long saturation also logs every 10,000th rejection, so it stays
        // visible without flooding the log.
        if newly_saturated || rejected.is_multiple_of(10_000) {
            log::warn!(
                "connection cap of {} reached; closing new connections ({rejected} rejected since startup)",
                counter.max
            );
        }
        None
    }

    #[cfg(test)]
    pub(crate) fn open_count(&self) -> usize {
        self.counter.open.load(Ordering::Acquire)
    }

    #[cfg(test)]
    fn saturated(&self) -> bool {
        self.counter.saturated.load(Ordering::Relaxed)
    }
}

/// Raise the soft open-file limit to the hard limit, since the default soft
/// limit (often 1024) is far below the connection cap. Call at startup, before
/// the server context is built, so the cap is fitted to the raised limit.
pub fn raise_open_file_limit() {
    #[cfg(unix)]
    {
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: `limit` is a valid, writable rlimit.
        if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } != 0 {
            log::warn!(
                "could not read the open file limit: {}",
                std::io::Error::last_os_error()
            );
            return;
        }
        if limit.rlim_cur >= limit.rlim_max {
            return;
        }
        let previous = limit.rlim_cur;
        // macOS reports an unlimited hard limit but rejects a soft limit above
        // OPEN_MAX (10,240), so fall back to that rather than keep the default
        // of 256.
        for target in [limit.rlim_max, limit.rlim_max.min(MACOS_OPEN_MAX)] {
            if target <= previous {
                break;
            }
            let raised = libc::rlimit {
                rlim_cur: target,
                rlim_max: limit.rlim_max,
            };
            // SAFETY: `raised` is a valid rlimit; the call only reads it.
            if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raised) } == 0 {
                log::info!("raised the open file limit from {previous} to {target}");
                return;
            }
        }
        log::warn!(
            "could not raise the open file limit from {previous}: {}",
            std::io::Error::last_os_error()
        );
    }
}

#[cfg(unix)]
const MACOS_OPEN_MAX: libc::rlim_t = 10_240;

/// The soft open-file limit, or `None` when it is unlimited or unknown.
fn open_file_soft_limit() -> Option<u64> {
    #[cfg(unix)]
    {
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: `limit` is a valid, writable rlimit.
        if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } != 0
            || limit.rlim_cur == libc::RLIM_INFINITY
        {
            return None;
        }
        Some(limit.rlim_cur)
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// Lower `max` so the connections plus descriptors kept back for everything
/// else (listeners, SQLite files, the Postgres pool, outbound HTTP) fit under
/// `soft_limit`. The reserve is an estimate: project databases aren't counted,
/// so with enough of them open `accept` can still run out first, which the
/// accept loop survives by backing off.
fn fit_to_descriptors(max: usize, soft_limit: Option<u64>) -> usize {
    let Some(soft_limit) = soft_limit else {
        return max;
    };
    let reserved = (soft_limit / 4).clamp(64, 16_384);
    let available = usize::try_from(soft_limit.saturating_sub(reserved)).unwrap_or(usize::MAX);
    max.min(available).max(1)
}

fn max_from(value: Option<&str>) -> usize {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return DEFAULT_MAX_CONNECTIONS;
    };
    match value.parse::<usize>() {
        Ok(max) if max > 0 => max,
        _ => {
            log::warn!(
                "ignoring {MAX_CONNECTIONS_ENV}={value:?}: expected a positive integer; using {DEFAULT_MAX_CONNECTIONS}"
            );
            DEFAULT_MAX_CONNECTIONS
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_over_the_cap_and_frees_on_drop() {
        let limit = ConnectionLimit::new(2);
        let a = limit.try_acquire().expect("first");
        let _b = limit.try_acquire().expect("second");
        assert!(limit.try_acquire().is_none());
        assert_eq!(limit.open_count(), 2, "a rejection takes no slot");
        drop(a);
        assert_eq!(limit.open_count(), 1);
        assert!(limit.try_acquire().is_some());
    }

    #[test]
    fn saturation_rearms_only_after_falling_well_below_the_cap() {
        let limit = ConnectionLimit::new(10);
        let mut held: Vec<_> = (0..10)
            .map(|_| limit.try_acquire().expect("slot"))
            .collect();
        assert!(!limit.saturated());
        assert!(limit.try_acquire().is_none());
        assert!(
            limit.saturated(),
            "the first rejection marks the cap reached"
        );

        // Churn at the cap keeps it marked, so it isn't logged again.
        held.pop();
        held.push(limit.try_acquire().expect("freed slot"));
        assert!(limit.try_acquire().is_none());
        assert!(limit.saturated());

        // Below 90% of the cap, the next rejection counts as a new saturation.
        held.truncate(8);
        assert!(!limit.saturated());
    }

    #[test]
    fn the_cap_leaves_descriptors_for_everything_else() {
        assert_eq!(fit_to_descriptors(100_000, None), 100_000);
        assert_eq!(fit_to_descriptors(100_000, Some(1_048_576)), 100_000);
        assert_eq!(fit_to_descriptors(100_000, Some(100_000)), 83_616);
        assert_eq!(fit_to_descriptors(100_000, Some(1024)), 768);
        assert_eq!(fit_to_descriptors(500, Some(1024)), 500);
        // macOS's default soft limit, if it can't be raised.
        assert_eq!(fit_to_descriptors(100_000, Some(256)), 192);
        assert_eq!(fit_to_descriptors(100_000, Some(10_240)), 7_680);
        assert_eq!(fit_to_descriptors(100_000, Some(50)), 1);
    }

    #[test]
    fn env_value_parsing() {
        assert_eq!(max_from(None), DEFAULT_MAX_CONNECTIONS);
        assert_eq!(max_from(Some("")), DEFAULT_MAX_CONNECTIONS);
        assert_eq!(max_from(Some(" 250 ")), 250);
        assert_eq!(max_from(Some("0")), DEFAULT_MAX_CONNECTIONS);
        assert_eq!(max_from(Some("-5")), DEFAULT_MAX_CONNECTIONS);
        assert_eq!(max_from(Some("lots")), DEFAULT_MAX_CONNECTIONS);
    }
}
