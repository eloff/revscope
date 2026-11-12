//! Caps how many ad-hoc SQL statements run at once, server-wide and per owner.
//!
//! One statement can hold a core and spill temp files for the whole query
//! timeout. Query tokens are gated only by the global request cap and MCP tool
//! calls by none, so without this a single caller could fill every slot. A statement
//! over the cap is refused at once rather than queued, because waiting would
//! hold a blocking thread.
//!
//! The public demo's readers get one slot each, and together at most half the
//! server's. Guests and token callers are keyed by client network, which
//! costs nothing to vary, so without that bound a stranger could refuse every
//! private project's queries.

use fxhash::FxHashMap;
use parking_lot::Mutex;

/// Statements one owner may run at once. A person or agent working through a
/// question runs them one after another, so two leaves room for overlap.
pub(crate) const PER_OWNER_QUERY_SLOTS: usize = 2;

/// Statements one demo reader may run at once. One, so that even the smallest
/// demo allowance, two slots, can't be filled by a single reader.
const PER_DEMO_READER_QUERY_SLOTS: usize = 1;

/// Who a statement's slot is charged to.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum SlotOwner {
    /// A private project's readers all belong to one account, so they share it.
    Project(String),
    /// Anyone may read the public demo, so sharing its slots would let one
    /// reader lock out everyone else. Keyed by the reader instead.
    DemoReader(String),
}

impl SlotOwner {
    fn is_demo(&self) -> bool {
        matches!(self, Self::DemoReader(_))
    }
}

pub(crate) struct QuerySlots {
    counts: Mutex<Counts>,
    global_limit: usize,
    demo_limit: usize,
    per_owner_limit: usize,
}

#[derive(Default)]
struct Counts {
    total: usize,
    demo: usize,
    /// Only owners with a statement running have an entry.
    per_owner: FxHashMap<SlotOwner, usize>,
}

/// Held while one statement runs; dropping it frees the slot.
pub(crate) struct QuerySlot<'a> {
    slots: &'a QuerySlots,
    owner: SlotOwner,
}

impl QuerySlots {
    /// The demo gets half of `global_limit`, rounded down.
    pub(crate) fn new(global_limit: usize, per_owner_limit: usize) -> Self {
        Self {
            counts: Mutex::new(Counts::default()),
            global_limit,
            demo_limit: global_limit / 2,
            per_owner_limit,
        }
    }

    /// One slot per core, since each statement is CPU-bound and more would only
    /// slow every statement down, but at least two owners' worth so that one
    /// owner can never hold them all.
    pub(crate) fn with_default_limits() -> Self {
        let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
        Self::new(cores.max(2 * PER_OWNER_QUERY_SLOTS), PER_OWNER_QUERY_SLOTS)
    }

    /// A slot for `owner`, or `None` when it, the demo or the server is at its cap.
    pub(crate) fn try_acquire(&self, owner: SlotOwner) -> Option<QuerySlot<'_>> {
        let mut counts = self.counts.lock();
        let is_demo = owner.is_demo();
        if counts.total >= self.global_limit || (is_demo && counts.demo >= self.demo_limit) {
            return None;
        }
        let limit = if is_demo {
            PER_DEMO_READER_QUERY_SLOTS
        } else {
            self.per_owner_limit
        };
        let running = counts.per_owner.entry(owner.clone()).or_insert(0);
        if *running >= limit {
            return None;
        }
        *running += 1;
        counts.total += 1;
        counts.demo += usize::from(is_demo);
        Some(QuerySlot { slots: self, owner })
    }
}

impl Drop for QuerySlot<'_> {
    fn drop(&mut self) {
        let mut counts = self.slots.counts.lock();
        counts.total -= 1;
        counts.demo -= usize::from(self.owner.is_demo());
        if let Some(running) = counts.per_owner.get_mut(&self.owner) {
            *running -= 1;
            if *running == 0 {
                counts.per_owner.remove(&self.owner);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(id: &str) -> SlotOwner {
        SlotOwner::Project(id.to_string())
    }

    #[test]
    fn caps_each_owner_and_frees_slots_on_drop() {
        let slots = QuerySlots::new(10, 2);
        let first = slots.try_acquire(project("1")).expect("first slot");
        let _second = slots.try_acquire(project("1")).expect("second slot");
        assert!(
            slots.try_acquire(project("1")).is_none(),
            "owner is at its cap"
        );
        assert!(
            slots.try_acquire(project("2")).is_some(),
            "other owners are unaffected"
        );
        drop(first);
        assert!(
            slots.try_acquire(project("1")).is_some(),
            "dropping frees the slot"
        );
    }

    #[test]
    fn demo_readers_do_not_share_slots() {
        let slots = QuerySlots::new(10, 1);
        let _a = slots
            .try_acquire(SlotOwner::DemoReader("203.0.113.1".to_string()))
            .expect("slot for one reader");
        assert!(
            slots
                .try_acquire(SlotOwner::DemoReader("203.0.113.2".to_string()))
                .is_some(),
            "a second reader is not locked out by the first"
        );
    }

    #[test]
    fn caps_the_demo_at_half_the_server() {
        let slots = QuerySlots::new(4, 2);
        let reader = |n: u8| SlotOwner::DemoReader(format!("203.0.113.{n}"));
        let _a = slots.try_acquire(reader(1)).expect("first demo slot");
        let _b = slots.try_acquire(reader(2)).expect("second demo slot");
        assert!(
            slots.try_acquire(reader(3)).is_none(),
            "a new network still finds the demo full"
        );
        assert!(
            slots.try_acquire(project("2")).is_some(),
            "private projects keep the other half"
        );
    }

    #[test]
    fn one_reader_cannot_fill_the_demo_or_the_server() {
        let slots = QuerySlots::new(4, PER_OWNER_QUERY_SLOTS);
        let reader = |n: u8| SlotOwner::DemoReader(format!("203.0.113.{n}"));
        let _first = slots.try_acquire(reader(1)).expect("the reader's slot");
        assert!(
            slots.try_acquire(reader(1)).is_none(),
            "one slot per reader"
        );
        let _second = slots
            .try_acquire(reader(2))
            .expect("the smallest demo allowance still admits a second reader");
        let _private: Vec<_> = (0..PER_OWNER_QUERY_SLOTS)
            .map(|_| slots.try_acquire(project("2")).expect("private slot"))
            .collect();
        assert!(
            slots.try_acquire(project("3")).is_none(),
            "the server is now full, though no single owner filled it"
        );
    }

    #[test]
    fn the_default_cap_holds_more_than_one_owner() {
        let slots = QuerySlots::with_default_limits();
        assert!(slots.global_limit >= 2 * PER_OWNER_QUERY_SLOTS);
        assert!(slots.demo_limit > PER_DEMO_READER_QUERY_SLOTS);
    }

    #[test]
    fn caps_the_server_across_owners() {
        let slots = QuerySlots::new(2, 2);
        let _a = slots.try_acquire(project("1")).expect("slot for project 1");
        let _b = slots.try_acquire(project("2")).expect("slot for project 2");
        assert!(
            slots.try_acquire(project("3")).is_none(),
            "server is at its cap"
        );
    }

    #[test]
    fn a_refused_owner_leaves_no_entry_behind() {
        let slots = QuerySlots::new(1, 1);
        let held = slots.try_acquire(project("1")).expect("slot");
        assert!(slots.try_acquire(project("2")).is_none());
        drop(held);
        assert!(slots.counts.lock().per_owner.is_empty());
    }
}
