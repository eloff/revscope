//! Concurrency limits for API and MCP work.
//!
//! A global semaphore caps how much blocking work runs at once. Under it, each
//! session may hold at most [`PER_SESSION_SLOTS`], so one dashboard opening its
//! burst of reads cannot take every global slot and starve other users. A
//! request takes its session slot first, then a global one, so a session's
//! excess requests queue on their own semaphore without holding global slots.
//!
//! Requests without a session cookie or bearer session (demo, public
//! dashboards, query and MCP tokens) are gated only globally.
//!
//! A session id is not checked against Postgres here, so the map is keyed by
//! client-chosen values. Three things keep that safe: an entry lives only while
//! its session has requests in flight or queued, the hasher is seeded, and a
//! session past [`PER_SESSION_MAX_QUEUED`] is refused without waiting. Rotating
//! made-up ids sidesteps the per-session cap, but not the global one.

use std::collections::hash_map::RandomState;
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// How long a request waits for its slots, in total, before 503.
pub(crate) const REQUEST_SLOT_WAIT: Duration = Duration::from_secs(5);

/// Concurrent API requests one session may have in flight. Browsers open about
/// six HTTP/1.1 connections per host, so a single tab rarely notices the cap.
pub(crate) const PER_SESSION_SLOTS: usize = 6;

/// Requests one session may have in flight or queued. Past this a request is
/// refused at once rather than parked for the full wait: a browser cannot get
/// near it, so whoever does is a script that should back off.
const PER_SESSION_MAX_QUEUED: usize = 64;

/// Keyed by a session id the client chose, so seeded SipHash rather than
/// FxHash: an unseeded hash would let crafted ids collide into one bucket.
type SessionSemaphores = DashMap<u128, SessionEntry, RandomState>;

struct SessionEntry {
    semaphore: Arc<Semaphore>,
    /// Requests holding or waiting for this semaphore. Changed only under the
    /// map's shard lock, so zero means nobody holds a permit or is about to.
    users: usize,
}

pub(crate) struct RequestSlots {
    global: Arc<Semaphore>,
    /// Only sessions with a request in flight or waiting have an entry.
    per_session: Arc<SessionSemaphores>,
    per_session_limit: usize,
    per_session_max_queued: usize,
    wait: Duration,
}

/// Held for the duration of one request; dropping it frees both slots.
pub(crate) struct RequestSlot {
    _global: OwnedSemaphorePermit,
    _session: Option<SessionSlot>,
}

/// One session slot, held or still being waited for.
struct SessionSlot {
    /// Declared first so the permit is released before `_user` drops the
    /// entry; a permit outliving its entry would let the next caller's fresh
    /// semaphore admit an extra request.
    permit: Option<OwnedSemaphorePermit>,
    semaphore: Arc<Semaphore>,
    _user: SessionUser,
}

/// One registered user of a session's entry. Created before any await, so
/// its `Drop` runs exactly once however the request ends, timeouts included.
struct SessionUser {
    session_key: u128,
    per_session: Arc<SessionSemaphores>,
}

impl Drop for SessionUser {
    fn drop(&mut self) {
        self.per_session
            .remove_if_mut(&self.session_key, |_, entry| {
                entry.users -= 1;
                entry.users == 0
            });
    }
}

impl RequestSlots {
    pub(crate) fn new(global_slots: usize, per_session_slots: usize, wait: Duration) -> Self {
        Self {
            global: Arc::new(Semaphore::new(global_slots)),
            per_session: Arc::new(DashMap::with_hasher(RandomState::new())),
            per_session_limit: per_session_slots,
            per_session_max_queued: PER_SESSION_MAX_QUEUED.max(per_session_slots),
            wait,
        }
    }

    /// Wait up to the configured wait for a session slot (when `session_key`
    /// is given) and then a global slot. `None` means answer 503.
    pub(crate) async fn acquire(&self, session_key: Option<u128>) -> Option<RequestSlot> {
        // Registered outside the timeout so a session over its queue cap is
        // refused immediately rather than after the wait.
        let mut session = match session_key {
            Some(session_key) => Some(self.register_session_user(session_key)?),
            None => None,
        };
        tokio::time::timeout(self.wait, async {
            if let Some(session) = session.as_mut() {
                session.permit = Some(session.semaphore.clone().acquire_owned().await.ok()?);
            }
            let global = self.global.clone().acquire_owned().await.ok()?;
            Some(RequestSlot {
                _global: global,
                _session: session.take(),
            })
        })
        .await
        .ok()
        .flatten()
    }

    fn register_session_user(&self, session_key: u128) -> Option<SessionSlot> {
        let mut entry = self
            .per_session
            .entry(session_key)
            .or_insert_with(|| SessionEntry {
                semaphore: Arc::new(Semaphore::new(self.per_session_limit)),
                users: 0,
            });
        if entry.users >= self.per_session_max_queued {
            return None;
        }
        entry.users += 1;
        let semaphore = entry.semaphore.clone();
        drop(entry);
        Some(SessionSlot {
            permit: None,
            semaphore,
            _user: SessionUser {
                session_key,
                per_session: self.per_session.clone(),
            },
        })
    }

    #[cfg(test)]
    fn tracked_sessions(&self) -> usize {
        self.per_session.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slots(global: usize, per_session: usize) -> RequestSlots {
        RequestSlots::new(global, per_session, Duration::from_millis(50))
    }

    #[tokio::test]
    async fn global_slots_time_out_when_held() {
        let slots = slots(1, 6);
        let _held = slots.acquire(None).await.expect("the only slot");
        assert!(slots.acquire(None).await.is_none());
    }

    #[tokio::test]
    async fn a_released_slot_can_be_acquired_again() {
        let slots = slots(1, 6);
        drop(slots.acquire(None).await.expect("the only slot"));
        assert!(slots.acquire(None).await.is_some());
    }

    #[tokio::test]
    async fn one_session_is_capped_while_others_proceed() {
        let slots = slots(10, 2);
        let _a1 = slots.acquire(Some(1)).await.expect("first");
        let _a2 = slots.acquire(Some(1)).await.expect("second");
        assert!(
            slots.acquire(Some(1)).await.is_none(),
            "a third request from the same session must wait"
        );
        assert!(slots.acquire(Some(2)).await.is_some());
        assert!(slots.acquire(None).await.is_some());
    }

    #[tokio::test]
    async fn a_waiting_session_request_holds_no_global_slot() {
        let slots = slots(2, 1);
        let _held = slots.acquire(Some(1)).await.expect("session 1");
        let waiter = slots.acquire(Some(1));
        // The waiter is queued on session 1's semaphore, so the one remaining
        // global slot is still free for another user.
        let (waited, other) = tokio::join!(waiter, slots.acquire(Some(2)));
        assert!(waited.is_none());
        assert!(other.is_some());
    }

    #[tokio::test]
    async fn queued_session_request_runs_when_a_slot_frees() {
        let slots = Arc::new(slots(10, 1));
        let held = slots.acquire(Some(1)).await.expect("first");
        let waiter = tokio::spawn({
            let slots = slots.clone();
            async move { slots.acquire(Some(1)).await.is_some() }
        });
        tokio::task::yield_now().await;
        drop(held);
        assert!(waiter.await.expect("join"));
    }

    #[tokio::test]
    async fn idle_sessions_are_forgotten() {
        let slots = slots(10, 2);
        let a = slots.acquire(Some(1)).await.expect("a");
        let b = slots.acquire(Some(1)).await.expect("b");
        assert!(slots.acquire(Some(1)).await.is_none(), "timed out waiter");
        assert_eq!(slots.tracked_sessions(), 1);
        drop(a);
        assert_eq!(slots.tracked_sessions(), 1, "b is still in flight");
        drop(b);
        assert_eq!(slots.tracked_sessions(), 0);
    }

    #[tokio::test]
    async fn global_timeout_releases_the_session_slot() {
        let slots = slots(1, 6);
        let _global = slots.acquire(None).await.expect("global");
        assert!(slots.acquire(Some(1)).await.is_none());
        assert_eq!(slots.tracked_sessions(), 0);
    }

    #[tokio::test]
    async fn a_session_over_its_queue_cap_is_refused_without_waiting() {
        let slots = Arc::new(RequestSlots::new(100, 1, Duration::from_secs(30)));
        let _held = slots.acquire(Some(1)).await.expect("in flight");
        let mut queued = Vec::new();
        for _ in 1..PER_SESSION_MAX_QUEUED {
            let slots = slots.clone();
            queued.push(tokio::spawn(async move {
                slots.acquire(Some(1)).await.is_some()
            }));
        }
        while slots.per_session.get(&1).map(|entry| entry.users) != Some(PER_SESSION_MAX_QUEUED) {
            tokio::task::yield_now().await;
        }
        let refused = tokio::time::timeout(Duration::from_secs(1), slots.acquire(Some(1)))
            .await
            .expect("an over-cap request must not wait");
        assert!(refused.is_none());
        assert!(
            slots.acquire(Some(2)).await.is_some(),
            "other sessions unaffected"
        );
        for task in queued {
            task.abort();
        }
    }

    #[tokio::test]
    async fn cancelled_waiters_leave_nothing_behind() {
        let slots = Arc::new(slots(10, 1));
        let held = slots.acquire(Some(1)).await.expect("held");
        let waiter = tokio::spawn({
            let slots = slots.clone();
            async move { slots.acquire(Some(1)).await }
        });
        while slots.per_session.get(&1).map(|entry| entry.users) != Some(2) {
            tokio::task::yield_now().await;
        }
        // Aborting drops the waiter mid-acquire, the path a closed client
        // connection takes.
        waiter.abort();
        let _ = waiter.await;
        assert_eq!(slots.per_session.get(&1).map(|entry| entry.users), Some(1));
        drop(held);
        assert_eq!(slots.tracked_sessions(), 0);
    }

    #[tokio::test]
    async fn a_session_never_exceeds_its_slots_under_churn() {
        let slots = Arc::new(slots(100, 3));
        let in_flight = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let peak = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut tasks = Vec::new();
        for _ in 0..40 {
            let (slots, in_flight, peak) = (slots.clone(), in_flight.clone(), peak.clone());
            tasks.push(tokio::spawn(async move {
                for _ in 0..20 {
                    let Some(_slot) = slots.acquire(Some(7)).await else {
                        continue;
                    };
                    let now = in_flight.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                    peak.fetch_max(now, std::sync::atomic::Ordering::SeqCst);
                    tokio::task::yield_now().await;
                    in_flight.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                }
            }));
        }
        for task in tasks {
            task.await.expect("join");
        }
        assert!(peak.load(std::sync::atomic::Ordering::SeqCst) <= 3);
        assert_eq!(slots.tracked_sessions(), 0);
    }
}
