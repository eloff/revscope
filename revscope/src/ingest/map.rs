//! A thread-safe, insertion-ordered map of in-flight ingest batches keyed by
//! `project_id`.
//!
//! This is a doubly linked list threaded through a hash
//! map, where the list nodes live in a `Vec` (an arena) and are linked by `u32`
//! indices instead of pointers. Slots hold reference-counted batch mutexes, and
//! freed slots are recycled through a free list so the arena only ever grows.
//!
//! # What it provides
//!
//! * `get` – O(1) get-or-insert, returning a locked guard to the batch.
//! * `least_recently_flushed` – O(1) access to the head (oldest) batch.
//! * `next_after` – O(1) access to the next batch after a slot index.
//! * `pop` – O(1) unlink of the guarded node (head or middle).
//! * `update_flushed_at` – O(1) move of any batch to the tail (most recent).
//!
//! # Concurrency model
//!
//! Two layers of locks are used:
//!
//! * The **structural** lock (`inner: RwLock<IngestMapInner>`) guards the map,
//!   the arena's links, the free list and the head/tail cursors.
//! * Each batch has its **own** `Mutex` so callers can mutate one batch while
//!   the structure (and other batches) are accessed concurrently.
//!
//! The links (`prev`/`next`) live in the arena slots, *not* inside the batch
//! mutex. That is deliberate: structural operations therefore only ever need to
//! hold the single batch mutex the caller already owns, never a neighbour's.
//!
//! ## Lock ordering / deadlock freedom
//!
//! The one rule that keeps this deadlock-free:
//!
//! > **No thread ever blocks on a batch mutex while holding the inner lock.**
//!
//! Look-ups read the inner lock only to find a slot, clone the batch `Arc`,
//! *release the inner lock*, and only then lock the batch. Structural
//! callers (`pop`/`update`) already hold a batch lock and then take the inner
//! lock (batch → inner). Because the inner lock is never held while waiting on a
//! batch lock, the inverse ordering can never close a cycle.
//!
//! ## Slot reuse safety
//!
//! Because a look-up locks a batch *after* dropping the inner lock, the slot it
//! captured could in principle be popped and reused in between. We detect this
//! after acquiring the batch lock by comparing the batch's own `project_id`
//! against the one we wanted (a popped/free slot is stamped with [`FREE_SLOT`]).
//! On a mismatch we simply retry. A batch can never be popped *while a guard is
//! held*, because popping requires locking that same batch first.
//!
//! ## Initialisation ordering
//!
//! A slot is only ever made reachable (inserted into the map and linked into the
//! list) *after* it carries its real `project_id`:
//!
//! * A **brand-new** slot's `Arc` is unreachable by any other thread while we
//!   hold the inner lock, so it is constructed already-stamped and published
//!   atomically under that single write lock.
//! * A **recycled** slot's `Arc` may still have a stale look-up waiting on it, so
//!   it is stamped off to the side (after releasing the inner lock) and only then
//!   re-published under the inner lock.
//!
//! Consequently a *published* slot is never observed as [`FREE_SLOT`]. The
//! sentinel only ever appears on slots that are not in the map or list (freshly
//! reserved, or popped and awaiting recycle), which is exactly what the slot
//! reuse check above relies on.

use fxhash::{FxBuildHasher, FxHashMap};
use parking_lot::{ArcMutexGuard, Mutex, RawMutex, RwLock};
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;
use tokio::time::Instant;

use super::batch::BatchData;

/// Sentinel `project_id` stamped on a slot that is not currently published in
/// the map and list: either freshly reserved before it is stamped and linked,
/// or popped and awaiting recycle. Real project ids are positive SERIAL values;
/// `0` is reserved.
const FREE_SLOT: i32 = 0;

pub(crate) type IngestBatchGuard = ArcMutexGuard<RawMutex, IngestBatch>;

pub(crate) struct IngestBatch {
    flushed_at: Instant,
    project_id: i32,
    /// Index of this batch's slot in the arena. Stable for the life of the slot
    /// (a recycled slot keeps the same index), so it identifies the batch to the
    /// structural operations.
    self_idx: u32,
    /// A threshold flush has been handed to a background task and has not
    /// started yet. Reset when the batch is popped, so a task queued for the
    /// popped batch finds it clear and leaves the recreated batch alone.
    flush_scheduled: bool,
    /// When the current run of flushes that found the database locked by
    /// another connection began, and when the latest of them ran. Cleared with
    /// the rows.
    locked: Option<LockedRun>,
    /// The queued, not-yet-flushed pageviews/events/engagements for this project
    /// plus the in-memory lookup maps that supplement the database for rows that
    /// have not been flushed yet.
    data: BatchData,
}

impl IngestBatch {
    pub fn new(project_id: i32, self_idx: u32) -> Self {
        Self {
            flushed_at: Instant::now(),
            project_id,
            self_idx,
            flush_scheduled: false,
            locked: None,
            data: BatchData::default(),
        }
    }

    pub fn flushed_at(&self) -> Instant {
        self.flushed_at
    }

    pub fn project_id(&self) -> i32 {
        self.project_id
    }

    pub(crate) fn self_idx(&self) -> u32 {
        self.self_idx
    }

    /// Mark a background threshold flush as queued. Returns false if one
    /// already is, in which case the caller must not queue another.
    pub(crate) fn schedule_flush(&mut self) -> bool {
        !std::mem::replace(&mut self.flush_scheduled, true)
    }

    #[cfg(test)]
    pub(crate) fn flush_scheduled(&self) -> bool {
        self.flush_scheduled
    }

    /// Claim the queued background flush. Returns false if none is queued for
    /// this batch: the slot was popped (and possibly recreated) since, or
    /// another task already claimed it.
    pub(crate) fn take_scheduled_flush(&mut self) -> bool {
        std::mem::replace(&mut self.flush_scheduled, false)
    }

    /// Note that a flush found the database locked. Returns how long the
    /// batch has been locked out, and whether this starts a new run. A lock
    /// found more than `max_gap` after the previous one starts a new run, so
    /// brief locks hours apart do not add up to one long one.
    pub(crate) fn mark_locked(&mut self, max_gap: Duration) -> (Duration, bool) {
        let now = Instant::now();
        match &mut self.locked {
            Some(run) if now.duration_since(run.latest) <= max_gap => {
                run.latest = now;
                (now.duration_since(run.since), false)
            }
            _ => {
                self.locked = Some(LockedRun {
                    since: now,
                    latest: now,
                });
                (Duration::ZERO, true)
            }
        }
    }

    /// End the current run of locked flushes, when a flush got past the lock
    /// or never reached the database.
    pub(crate) fn clear_locked(&mut self) {
        self.locked = None;
    }

    #[cfg(test)]
    pub(crate) fn set_locked_since(&mut self, since: Instant) {
        self.locked = Some(LockedRun {
            since,
            latest: Instant::now(),
        });
    }

    /// Empty the queued rows and lookup maps after a successful flush. Used when
    /// the batch stays in the map.
    pub fn clear(&mut self) {
        self.locked = None;
        BatchData::clear(self);
    }

    /// Fully reset the batch so the slot can be recycled for another project.
    pub fn reset(&mut self) {
        self.flush_scheduled = false;
        self.locked = None;
        BatchData::reset(self);
    }
}

struct LockedRun {
    since: Instant,
    latest: Instant,
}

impl Deref for IngestBatch {
    type Target = BatchData;

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl DerefMut for IngestBatch {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.data
    }
}

/// One arena slot: the shared batch mutex plus its list links.
struct Slot {
    batch: Arc<Mutex<IngestBatch>>,
    prev: Option<u32>,
    next: Option<u32>,
}

struct IngestMapInner {
    batch_idx_by_project_id: FxHashMap<i32, u32>,
    slots: Vec<Slot>,
    /// Indices in `slots` that are not currently in use. We pop from this before
    /// pushing a new slot to the end.
    free_batch_indices: Vec<u32>,
    /// head is the least recently flushed batch, or None if map is empty
    head_batch_idx: Option<u32>,
    /// tail is the most recently flushed or inserted batch, or None if map is empty
    tail_batch_idx: Option<u32>,
}

impl IngestMapInner {
    pub const fn new() -> Self {
        Self {
            batch_idx_by_project_id: FxHashMap::with_hasher(FxBuildHasher::new()),
            slots: Vec::new(),
            free_batch_indices: Vec::new(),
            head_batch_idx: None,
            tail_batch_idx: None,
        }
    }

    fn batch_arc(&self, idx: u32) -> Arc<Mutex<IngestBatch>> {
        Arc::clone(&self.slots[idx as usize].batch)
    }

    /// Push a brand-new slot, already stamped with `project_id`, and return its
    /// index. The slot's `Arc` is freshly allocated and not yet referenced by any
    /// other thread, so the caller may construct it fully (and even lock it) while
    /// still holding the inner lock without risking a deadlock.
    fn push_new_slot(&mut self, project_id: i32) -> u32 {
        let idx = self.slots.len() as u32;
        self.slots.push(Slot {
            batch: Arc::new(Mutex::new(IngestBatch::new(project_id, idx))),
            prev: None,
            next: None,
        });
        idx
    }

    fn link_at_tail(&mut self, idx: u32) {
        let i = idx as usize;
        self.slots[i].next = None;
        self.slots[i].prev = self.tail_batch_idx;
        match self.tail_batch_idx {
            Some(tail) => self.slots[tail as usize].next = Some(idx),
            None => self.head_batch_idx = Some(idx),
        }
        self.tail_batch_idx = Some(idx);
    }

    /// Detach slot `idx` from the list, patching its neighbours and the
    /// head/tail cursors. Leaves the slot's own links cleared.
    fn unlink(&mut self, idx: u32) {
        let i = idx as usize;
        let prev = self.slots[i].prev;
        let next = self.slots[i].next;
        match prev {
            Some(p) => self.slots[p as usize].next = next,
            None => self.head_batch_idx = next,
        }
        match next {
            Some(n) => self.slots[n as usize].prev = prev,
            None => self.tail_batch_idx = prev,
        }
        self.slots[i].prev = None;
        self.slots[i].next = None;
    }

    fn move_to_tail(&mut self, idx: u32) {
        if self.tail_batch_idx == Some(idx) {
            return;
        }
        self.unlink(idx);
        self.link_at_tail(idx);
    }
}

pub(crate) struct IngestMap {
    inner: RwLock<IngestMapInner>,
    /// Notified when a batch is inserted into a previously empty map, so the
    /// flush scheduler can wake from an idle wait and arm its timer.
    batch_added: Notify,
}

impl IngestMap {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(IngestMapInner::new()),
            batch_added: Notify::new(),
        }
    }

    /// Wait until a batch is inserted into the (otherwise empty) map. Used by
    /// the flush scheduler while no batch is pending.
    pub async fn wait_for_batch(&self) {
        self.batch_added.notified().await;
    }

    /// Lock `batch` and return its guard iff the batch still belongs
    /// to `project_id`. `None` means the slot was popped and recycled out from
    /// under us between cloning the `Arc` and locking it, so the caller should
    /// retry.
    fn lock_matching(
        &self,
        batch: &Arc<Mutex<IngestBatch>>,
        project_id: i32,
    ) -> Option<IngestBatchGuard> {
        let guard = batch.lock_arc();
        (guard.project_id == project_id).then_some(guard)
    }

    /// Lock the existing batch for `project_id`, if any. Does not create a batch.
    pub fn get_existing(&self, project_id: i32) -> Option<IngestBatchGuard> {
        debug_assert_ne!(project_id, FREE_SLOT, "project_id collides with sentinel");
        loop {
            let existing = {
                let inner = self.inner.read();
                inner
                    .batch_idx_by_project_id
                    .get(&project_id)
                    .map(|&idx| inner.batch_arc(idx))
            };
            let batch = existing?;
            if let Some(guard) = self.lock_matching(&batch, project_id) {
                return Some(guard);
            }
            // Slot was recycled between the read and the lock; retry.
        }
    }

    /// Get or insert an IngestBatch for `project_id` and return a locked guard to it.
    pub fn get(&self, project_id: i32) -> IngestBatchGuard {
        debug_assert_ne!(project_id, FREE_SLOT, "project_id collides with sentinel");

        loop {
            // Fast path: the project already has a batch. Clone its Arc under
            // the read lock, release the read lock, then lock it.
            let existing = {
                let inner = self.inner.read();
                inner
                    .batch_idx_by_project_id
                    .get(&project_id)
                    .map(|&idx| inner.batch_arc(idx))
            };
            if let Some(batch) = existing {
                if let Some(guard) = self.lock_matching(&batch, project_id) {
                    return guard;
                }
                // The slot was popped and recycled between dropping the read lock
                // and locking the batch. Retry from scratch.
                continue;
            }

            // Slow path: the read lock saw no entry. We never publish a slot
            // (into the map or the list) until it already carries its real
            // `project_id`, so no thread can ever observe one mid-init.
            let recycled = {
                let mut inner = self.inner.write();
                // Re-check: a concurrent writer may have published it since our
                // read miss.
                if let Some(&idx) = inner.batch_idx_by_project_id.get(&project_id) {
                    let batch = inner.batch_arc(idx);
                    drop(inner);
                    if let Some(guard) = self.lock_matching(&batch, project_id) {
                        return guard;
                    }
                    continue;
                }
                match inner.free_batch_indices.pop() {
                    None => {
                        // No free slot to recycle. A brand-new slot's Arc is not
                        // yet reachable by anyone else, so we can construct it
                        // already stamped and publish it atomically under this
                        // single write lock - no separate init step, no race.
                        let was_empty = inner.head_batch_idx.is_none();
                        let idx = inner.push_new_slot(project_id);
                        inner.link_at_tail(idx);
                        inner.batch_idx_by_project_id.insert(project_id, idx);
                        // Locking here can't block: no other thread can reach
                        // this fresh Arc until after the inner lock is released.
                        let guard = inner.batch_arc(idx).lock_arc();
                        if was_empty {
                            self.batch_added.notify_one();
                        }
                        return guard;
                    }
                    // A recycled slot's Arc may still have a stale look-up waiting
                    // on it, so we must NOT lock it while holding the inner lock.
                    // Initialise it off to the side instead.
                    Some(idx) => (inner.batch_arc(idx), idx),
                }
            };

            // Stamp the recycled slot off to the side. The inner lock is released
            // now, so this lock can't deadlock even if a stale look-up is still
            // draining the slot: that thread can take the inner lock to finish.
            let (batch, idx) = recycled;
            let mut guard = batch.lock_arc();
            guard.project_id = project_id;
            guard.flushed_at = Instant::now();
            guard.self_idx = idx;

            // Publish under the write lock, re-checking for a concurrent inserter
            // that created the same project meanwhile. Only after linking +
            // inserting does the slot become reachable - already stamped, so it
            // is never observed as FREE_SLOT through the map or the list.
            {
                let mut inner = self.inner.write();
                if let Some(&winner) = inner.batch_idx_by_project_id.get(&project_id) {
                    // Lost the race: return our slot to the free list (stamped
                    // free so any stale observer of its previous life retries),
                    // then fall back to the winner.
                    guard.project_id = FREE_SLOT;
                    drop(guard);
                    let winner_batch = inner.batch_arc(winner);
                    inner.free_batch_indices.push(idx);
                    drop(inner);
                    if let Some(guard) = self.lock_matching(&winner_batch, project_id) {
                        return guard;
                    }
                    continue;
                }
                let was_empty = inner.head_batch_idx.is_none();
                inner.link_at_tail(idx);
                inner.batch_idx_by_project_id.insert(project_id, idx);
                if was_empty {
                    self.batch_added.notify_one();
                }
            }
            return guard;
        }
    }

    /// Get the least recently flushed IngestBatch and return a locked guard to it, or None if map is empty.
    pub fn least_recently_flushed(&self) -> Option<IngestBatchGuard> {
        loop {
            let head = {
                let inner = self.inner.read();
                inner.head_batch_idx.map(|h| (h, inner.batch_arc(h)))
            };
            let (head_idx, batch) = head?;

            let guard = batch.lock_arc();

            // The head could have changed between us reading it and locking the
            // batch. Once we hold this batch's lock it cannot itself be moved
            // (that requires its lock), so a stable head means we have the
            // genuine least-recently-flushed batch.
            //
            // This also subsumes the FREE_SLOT case: a freed slot is never in the
            // list (it is unlinked before being stamped FREE_SLOT, and only
            // linked back after being re-stamped), so if we ever lock a FREE_SLOT
            // slot here the head must have moved on and the check below retries.
            let still_head = self.inner.read().head_batch_idx == Some(head_idx);
            if still_head {
                return Some(guard);
            }
            drop(guard);
        }
    }

    /// Lock the batch after `idx` in LRU order (older to newer), if any.
    ///
    /// `idx` is a slot index from [`IngestBatch::self_idx`]. The caller must
    /// drop any guard for `idx` before calling this so we do not wait on a
    /// neighbour while holding a batch lock.
    pub fn next_after(&self, idx: u32) -> Option<IngestBatchGuard> {
        loop {
            let next = {
                let inner = self.inner.read();
                let slot = inner.slots.get(idx as usize)?;
                slot.next.map(|n| (n, inner.batch_arc(n)))
            };
            let (next_idx, batch) = next?;
            let guard = batch.lock_arc();
            if guard.project_id == FREE_SLOT {
                drop(guard);
                continue;
            }
            let still_next = {
                let inner = self.inner.read();
                inner.slots.get(idx as usize).and_then(|slot| slot.next) == Some(next_idx)
            };
            if still_next {
                return Some(guard);
            }
            drop(guard);
        }
    }

    /// Unlink the guarded batch from the map and list.
    ///
    /// Safe for the head or a middle node: links live on the arena slot, so
    /// neighbours' batch mutexes are not required. No-op if this guard's slot
    /// is already free (double-pop).
    pub fn pop(&self, mut batch: IngestBatchGuard) {
        let idx = batch.self_idx;
        let project_id = batch.project_id;
        if project_id == FREE_SLOT {
            return;
        }
        {
            let mut inner = self.inner.write();
            match inner.batch_idx_by_project_id.remove(&project_id) {
                Some(mapped_idx) if mapped_idx == idx => {
                    inner.unlink(idx);
                    inner.free_batch_indices.push(idx);
                }
                Some(other_idx) => {
                    inner.batch_idx_by_project_id.insert(project_id, other_idx);
                    return;
                }
                None => return,
            }
        }
        // Stamp the slot as free so any thread that captured this slot before it
        // was popped detects the change after it acquires the lock. We still
        // hold the lock here, so no recycling inserter can observe it until we
        // drop `batch`.
        batch.reset();
        batch.project_id = FREE_SLOT;
    }

    /// Update flushed_at on the batch.
    /// which must have been returned by a prior call to [`IngestMap::get()`].
    pub fn update_flushed_at(&self, mut batch: IngestBatchGuard) {
        batch.clear();
        batch.flushed_at = Instant::now();
        let idx = batch.self_idx;
        let mut inner = self.inner.write();
        inner.move_to_tail(idx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::time::Duration;

    #[test]
    fn locks_far_apart_do_not_add_up_to_one_long_lockout() {
        let gap = Duration::from_secs(10);
        let mut batch = IngestBatch::new(1, 1);
        assert_eq!(batch.mark_locked(gap), (Duration::ZERO, true));

        let long_ago = Instant::now() - Duration::from_secs(60);
        batch.locked = Some(LockedRun {
            since: long_ago,
            latest: Instant::now(),
        });
        let (locked_for, new_run) = batch.mark_locked(gap);
        assert!(!new_run && locked_for >= Duration::from_secs(60));

        batch.locked = Some(LockedRun {
            since: long_ago,
            latest: long_ago,
        });
        assert_eq!(batch.mark_locked(gap), (Duration::ZERO, true));

        batch.clear_locked();
        assert_eq!(batch.mark_locked(gap), (Duration::ZERO, true));
    }

    impl IngestMap {
        /// Drain the list head-to-tail (oldest first), returning project ids in
        /// order. Test-only; assumes no other threads are active.
        fn drain_order(&self) -> Vec<i32> {
            let mut out = Vec::new();
            while let Some(guard) = self.least_recently_flushed() {
                out.push(guard.project_id);
                self.pop(guard);
            }
            out
        }

        fn slot_count(&self) -> usize {
            self.inner.read().slots.len()
        }

        fn len(&self) -> usize {
            self.inner.read().batch_idx_by_project_id.len()
        }

        /// Verify the data-structure invariants. Test-only; assumes the map is
        /// quiescent (no concurrent operations).
        fn check_consistency(&self) {
            let inner = self.inner.read();
            let mut seen = 0usize;
            let mut cursor = inner.head_batch_idx;
            let mut expected_prev = None;
            while let Some(c) = cursor {
                let slot = &inner.slots[c as usize];
                assert_eq!(slot.prev, expected_prev, "prev link mismatch at {c}");
                let pid = slot.batch.lock().project_id;
                assert_ne!(pid, FREE_SLOT, "live slot {c} stamped free");
                assert_eq!(
                    inner.batch_idx_by_project_id.get(&pid),
                    Some(&c),
                    "map/list disagree for project {pid}"
                );
                expected_prev = Some(c);
                cursor = slot.next;
                seen += 1;
                assert!(seen <= inner.slots.len(), "cycle detected in list");
            }
            assert_eq!(inner.tail_batch_idx, expected_prev, "tail link mismatch");
            assert_eq!(
                seen,
                inner.batch_idx_by_project_id.len(),
                "list length != map length"
            );
            assert_eq!(
                seen + inner.free_batch_indices.len(),
                inner.slots.len(),
                "live + free != arena size"
            );
        }
    }

    #[test]
    fn get_inserts_and_returns_same_batch() {
        let map = IngestMap::new();

        let first = {
            let g = map.get(7);
            assert_eq!(g.project_id(), 7);
            g.self_idx
        };
        // Second get for the same project returns the same slot.
        let g = map.get(7);
        assert_eq!(g.self_idx, first);
        assert_eq!(g.project_id(), 7);
        drop(g);

        assert_eq!(map.len(), 1);
        map.check_consistency();
    }

    #[test]
    fn least_recently_flushed_is_none_when_empty() {
        let map = IngestMap::new();
        assert!(map.least_recently_flushed().is_none());
    }

    #[test]
    fn insertion_order_is_preserved() {
        let map = IngestMap::new();
        for p in [10i32, 20, 30, 40] {
            drop(map.get(p));
            map.check_consistency();
        }
        assert_eq!(map.drain_order(), vec![10, 20, 30, 40]);
        assert!(map.least_recently_flushed().is_none());
        map.check_consistency();
    }

    #[test]
    fn pop_middle_unlinks_neighbours() {
        let map = IngestMap::new();
        for p in [1i32, 2, 3] {
            drop(map.get(p));
        }

        let mid = map.get(2);
        map.pop(mid);
        map.check_consistency();

        assert_eq!(map.least_recently_flushed().unwrap().project_id(), 1);
        assert_eq!(map.len(), 2);
        assert_eq!(map.drain_order(), vec![1, 3]);
    }

    #[test]
    fn pop_removes_from_head() {
        let map = IngestMap::new();
        for p in [1i32, 2, 3] {
            drop(map.get(p));
        }

        let head = map.least_recently_flushed().unwrap();
        assert_eq!(head.project_id(), 1);
        map.pop(head);
        map.check_consistency();

        assert_eq!(map.least_recently_flushed().unwrap().project_id(), 2);
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn next_after_follows_list_order() {
        let map = IngestMap::new();
        for p in [1i32, 2, 3] {
            drop(map.get(p));
        }

        let head = map.least_recently_flushed().unwrap();
        assert_eq!(head.project_id(), 1);
        let idx = head.self_idx();
        drop(head);

        let second = map.next_after(idx).unwrap();
        assert_eq!(second.project_id(), 2);
        let second_idx = second.self_idx();
        drop(second);

        let third = map.next_after(second_idx).unwrap();
        assert_eq!(third.project_id(), 3);
        let third_idx = third.self_idx();
        drop(third);

        assert!(map.next_after(third_idx).is_none());
    }

    #[test]
    fn update_flushed_at_moves_to_tail() {
        let map = IngestMap::new();
        for p in [1i32, 2, 3] {
            drop(map.get(p));
        }

        // Touch the head (1) -> it should move to the back.
        let before = map.least_recently_flushed().unwrap().flushed_at();
        std::thread::sleep(Duration::from_millis(2));
        let one = map.get(1);
        map.update_flushed_at(one);
        map.check_consistency();

        let new_head_pid = map.least_recently_flushed().unwrap().project_id();
        assert_eq!(new_head_pid, 2);
        // 1 is now newest; confirm its timestamp advanced.
        let touched_after = map.get(1).flushed_at();
        assert!(touched_after > before);

        assert_eq!(map.drain_order(), vec![2, 3, 1]);
    }

    #[test]
    fn update_on_tail_is_a_noop_for_order() {
        let map = IngestMap::new();
        for p in [1i32, 2, 3] {
            drop(map.get(p));
        }
        let tail = map.get(3);
        map.update_flushed_at(tail);
        map.check_consistency();
        assert_eq!(map.drain_order(), vec![1, 2, 3]);
    }

    #[test]
    fn update_middle_element() {
        let map = IngestMap::new();
        for p in [1i32, 2, 3, 4, 5] {
            drop(map.get(p));
        }
        let mid = map.get(3);
        map.update_flushed_at(mid);
        map.check_consistency();
        assert_eq!(map.drain_order(), vec![1, 2, 4, 5, 3]);
    }

    #[test]
    fn freed_slots_are_recycled() {
        let map = IngestMap::new();
        drop(map.get(1));
        drop(map.get(2));
        assert_eq!(map.slot_count(), 2);

        // Drain everything back onto the free list.
        let _ = map.drain_order();
        assert_eq!(map.len(), 0);

        // New inserts should reuse the two free slots rather than grow.
        drop(map.get(3));
        drop(map.get(4));
        assert_eq!(map.slot_count(), 2, "expected slot reuse, not growth");
        assert_eq!(map.drain_order(), vec![3, 4]);
        map.check_consistency();
    }

    #[test]
    fn single_element_pop_empties_map() {
        let map = IngestMap::new();
        drop(map.get(99));
        let g = map.least_recently_flushed().unwrap();
        map.pop(g);
        assert!(map.least_recently_flushed().is_none());
        assert_eq!(map.len(), 0);
        map.check_consistency();
    }

    #[test]
    fn concurrent_get_same_project_returns_one_slot() {
        let map = Arc::new(IngestMap::new());
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let map = Arc::clone(&map);
                std::thread::spawn(move || {
                    for _ in 0..2_000 {
                        let g = map.get(42);
                        assert_eq!(g.project_id(), 42);
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(map.len(), 1);
        assert_eq!(
            map.slot_count(),
            1,
            "concurrent inserts created extra slots"
        );
        map.check_consistency();
    }

    /// Hammer the map from many threads doing get/update while a flusher pops and
    /// re-orders. The point is to surface deadlocks, panics, or invariant
    /// violations under contention and heavy slot recycling.
    #[test]
    fn concurrent_stress() {
        const WORKERS: usize = 8;
        const PROJECTS: i32 = 16;
        const ITERS: usize = 20_000;

        let map = Arc::new(IngestMap::new());
        let stop = Arc::new(AtomicBool::new(false));
        let counter = Arc::new(AtomicU64::new(0));

        let workers: Vec<_> = (0..WORKERS)
            .map(|w| {
                let map = Arc::clone(&map);
                let counter = Arc::clone(&counter);
                std::thread::spawn(move || {
                    for i in 0..ITERS {
                        let project = ((i as i32).wrapping_mul(1_103_515_245) + w as i32)
                            .rem_euclid(PROJECTS)
                            + 1;
                        let g = map.get(project);
                        assert_eq!(g.project_id(), project);
                        // Every so often, bump the batch to the tail.
                        if i % 5 == 0 {
                            map.update_flushed_at(g);
                        } else {
                            drop(g);
                        }
                        counter.fetch_add(1, Ordering::Relaxed);
                    }
                })
            })
            .collect();

        // Flusher: continuously pops or re-orders the oldest batch.
        let flusher = {
            let map = Arc::clone(&map);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                let mut toggle = false;
                while !stop.load(Ordering::Relaxed) {
                    if let Some(g) = map.least_recently_flushed() {
                        if toggle {
                            map.pop(g);
                        } else {
                            map.update_flushed_at(g);
                        }
                        toggle = !toggle;
                    } else {
                        std::thread::yield_now();
                    }
                }
            })
        };

        for t in workers {
            t.join().unwrap();
        }
        stop.store(true, Ordering::Relaxed);
        flusher.join().unwrap();

        assert_eq!(counter.load(Ordering::Relaxed) as usize, WORKERS * ITERS);
        // Whatever remains must still be a consistent structure.
        map.check_consistency();
        assert!(map.len() <= PROJECTS as usize);
    }
}
