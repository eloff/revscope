use std::collections::VecDeque;

use parking_lot::Mutex;

/// A thread-safe least-recently-used cache for a handful of entries, with no
/// expiry. Lookups scan linearly, which is cheaper than hashing at the small
/// capacities it is meant for.
pub(crate) struct LruCache<K, V> {
    /// Most recently used first.
    entries: Mutex<VecDeque<(K, V)>>,
    capacity: usize,
}

impl<K: PartialEq, V: Clone> LruCache<K, V> {
    pub(crate) fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "an LRU cache needs room for one entry");
        Self {
            entries: Mutex::new(VecDeque::with_capacity(capacity)),
            capacity,
        }
    }

    /// Returns the value cached under `key`, marking it most recently used.
    pub(crate) fn get(&self, key: &K) -> Option<V> {
        let mut entries = self.entries.lock();
        let index = entries.iter().position(|(cached, _)| cached == key)?;
        let entry = entries.remove(index)?;
        let value = entry.1.clone();
        entries.push_front(entry);
        Some(value)
    }

    /// Caches `value` under `key` as the most recently used entry, evicting the
    /// least recently used one when full.
    pub(crate) fn put(&self, key: K, value: V) {
        let mut entries = self.entries.lock();
        if let Some(index) = entries.iter().position(|(cached, _)| *cached == key) {
            entries.remove(index);
        } else if entries.len() == self.capacity {
            entries.pop_back();
        }
        entries.push_front((key, value));
    }

    pub(crate) fn clear(&self) {
        self.entries.lock().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_the_least_recently_used_entry() {
        let cache = LruCache::new(2);
        cache.put('a', 1);
        cache.put('b', 2);
        // Reading A leaves B as the eviction victim when C arrives.
        assert_eq!(cache.get(&'a'), Some(1));
        cache.put('c', 3);
        assert_eq!(cache.get(&'a'), Some(1));
        assert_eq!(cache.get(&'c'), Some(3));
        assert_eq!(cache.get(&'b'), None);
    }

    #[test]
    fn replacing_a_key_refreshes_it_without_evicting() {
        let cache = LruCache::new(2);
        cache.put('a', 1);
        cache.put('b', 2);
        cache.put('a', 10);
        cache.put('c', 3);
        assert_eq!(cache.get(&'a'), Some(10));
        assert_eq!(cache.get(&'b'), None);
        assert_eq!(cache.get(&'c'), Some(3));
    }

    #[test]
    fn clear_drops_every_entry() {
        let cache = LruCache::new(2);
        cache.put('a', 1);
        cache.clear();
        assert_eq!(cache.get(&'a'), None);
    }
}
