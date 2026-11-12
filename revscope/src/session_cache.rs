//! Sessions Postgres has already confirmed, so a burst of dashboard requests
//! resolves its session once rather than once per request.
//!
//! Only `auth::load_session_user` fills it, for [`crate::auth::session_user`],
//! and only with a row Postgres returned, so a made-up cookie never takes up
//! space here. Entries live for [`SESSION_CACHE_TTL_SECONDS`], or until the
//! session itself expires if that comes first. Postgres remains the source of truth:
//! anything that deletes a session or changes what it resolves to (logout,
//! password reset, plan change, a user's role or projects) invalidates here
//! after its write commits.
//!
//! Each process has its own cache, so an invalidation on one server reaches the
//! others only when their entries expire, at most the TTL later.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::auth::SessionUser;
use crate::tokens::{ExpiringTokens, TokenSecret};
use crate::util::seconds_since_epoch;

/// How long a confirmed session is trusted before Postgres is asked again.
pub(crate) const SESSION_CACHE_TTL_SECONDS: u32 = 60;

/// Past this many entries the cache is emptied rather than grown. Every entry
/// is a session someone actually logged in to within the last TTL, so this is
/// far above normal traffic; it is a backstop, not an eviction policy.
const MAX_SESSION_CACHE_ENTRIES: usize = 100_000;

pub(crate) struct SessionCache {
    users: ExpiringTokens<SessionUser>,
    /// Bumped by every invalidation. A lookup that started before one may
    /// have read the row the invalidation just changed, so it must not cache
    /// what it read; see [`Self::insert`].
    generation: AtomicU64,
}

impl SessionCache {
    pub(crate) fn new() -> Self {
        Self {
            users: ExpiringTokens::new(),
            generation: AtomicU64::new(0),
        }
    }

    pub(crate) fn get(&self, session_key: u128) -> Option<SessionUser> {
        self.users.get(TokenSecret::from_raw(session_key))
    }

    /// Read before querying Postgres and pass to [`Self::insert`] after.
    pub(crate) fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Caches `user` unless an invalidation ran since `generation` was read.
    /// `session_expires_at` is in seconds since the unix epoch.
    ///
    /// Invalidations bump the generation and then sweep. If the bump lands
    /// before the re-check below, the entry is taken back out; if it lands
    /// after, the entry is already in the map when the sweep runs. Either way
    /// a row read before the invalidating write committed does not survive it.
    pub(crate) fn insert(
        &self,
        session_key: u128,
        user: SessionUser,
        session_expires_at: u32,
        generation: u64,
    ) {
        if self.generation() != generation {
            return;
        }
        if self.users.len() >= MAX_SESSION_CACHE_ENTRIES {
            // The count includes expired entries, which may be all it takes.
            self.users.sweep_expired();
            if self.users.len() >= MAX_SESSION_CACHE_ENTRIES {
                self.users.retain(|_| false);
            }
        }
        let secret = TokenSecret::from_raw(session_key);
        let expires_at = seconds_since_epoch()
            .saturating_add(SESSION_CACHE_TTL_SECONDS)
            .min(session_expires_at);
        self.users.insert(secret, user, expires_at);
        if self.generation() != generation {
            self.users.take(secret);
        }
    }

    /// One session ended, as on logout.
    pub(crate) fn remove(&self, session_key: u128) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.users.take(TokenSecret::from_raw(session_key));
    }

    /// Drops one entry after Postgres rejected its session. Unlike
    /// [`Self::remove`] this leaves the generation alone: nothing was written,
    /// so there is no stale read to block, and bumping here would let any
    /// caller with a made-up cookie stop every concurrent lookup from caching.
    pub(crate) fn evict(&self, session_key: u128) {
        self.users.take(TokenSecret::from_raw(session_key));
    }

    /// Every session of one user, as after a password reset.
    pub(crate) fn invalidate_user(&self, user_id: &str) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.users.retain(|user| user.user_id != user_id);
    }

    /// Every session on one account, as after a plan change, so /me shows the
    /// new plan at once (nothing gates on the cached plan: the AI quota reads
    /// it from Postgres), or after someone on it confirms their email.
    pub(crate) fn invalidate_account(&self, account_id: &str) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.users.retain(|user| user.account_id != account_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAR_FUTURE: u32 = u32::MAX;

    fn user(user_id: &str, account_id: &str) -> SessionUser {
        SessionUser {
            user_id: user_id.to_string(),
            account_id: account_id.to_string(),
            name: "Test".to_string(),
            email: "test@example.com".to_string(),
            plan: "starter".to_string(),
            retention_months: Some(12),
            is_admin: true,
            projects: crate::user_access::ProjectScope::All,
            account_confirmed: true,
        }
    }

    #[test]
    fn an_entry_lasts_no_longer_than_its_session() {
        let cache = SessionCache::new();
        let expired = seconds_since_epoch() - 1;
        cache.insert(1, user("u1", "a1"), expired, cache.generation());
        assert!(cache.get(1).is_none());
    }

    #[test]
    fn returns_what_was_inserted() {
        let cache = SessionCache::new();
        cache.insert(1, user("u1", "a1"), FAR_FUTURE, cache.generation());
        assert_eq!(
            cache.get(1).map(|user| user.user_id),
            Some("u1".to_string())
        );
        assert!(cache.get(2).is_none());
    }

    #[test]
    fn remove_drops_one_session() {
        let cache = SessionCache::new();
        cache.insert(1, user("u1", "a1"), FAR_FUTURE, cache.generation());
        cache.insert(2, user("u1", "a1"), FAR_FUTURE, cache.generation());
        cache.remove(1);
        assert!(cache.get(1).is_none());
        assert!(cache.get(2).is_some());
    }

    #[test]
    fn evicting_a_rejected_session_does_not_block_concurrent_lookups() {
        let cache = SessionCache::new();
        let generation = cache.generation();
        cache.evict(99);
        cache.insert(1, user("u1", "a1"), FAR_FUTURE, generation);
        assert!(cache.get(1).is_some());
    }

    #[test]
    fn invalidate_user_drops_only_that_users_sessions() {
        let cache = SessionCache::new();
        cache.insert(1, user("u1", "a1"), FAR_FUTURE, cache.generation());
        cache.insert(2, user("u1", "a1"), FAR_FUTURE, cache.generation());
        cache.insert(3, user("u2", "a1"), FAR_FUTURE, cache.generation());
        cache.invalidate_user("u1");
        assert!(cache.get(1).is_none());
        assert!(cache.get(2).is_none());
        assert!(cache.get(3).is_some());
    }

    #[test]
    fn invalidate_account_drops_every_user_on_it() {
        let cache = SessionCache::new();
        cache.insert(1, user("u1", "a1"), FAR_FUTURE, cache.generation());
        cache.insert(2, user("u2", "a1"), FAR_FUTURE, cache.generation());
        cache.insert(3, user("u3", "a2"), FAR_FUTURE, cache.generation());
        cache.invalidate_account("a1");
        assert!(cache.get(1).is_none());
        assert!(cache.get(2).is_none());
        assert!(cache.get(3).is_some());
    }

    #[test]
    fn lookup_that_raced_an_invalidation_is_not_cached() {
        let cache = SessionCache::new();
        let generation = cache.generation();
        // The row was read, then a password reset committed and invalidated.
        cache.invalidate_user("u1");
        cache.insert(1, user("u1", "a1"), FAR_FUTURE, generation);
        assert!(cache.get(1).is_none());
    }

    #[test]
    fn full_cache_starts_over_instead_of_growing() {
        let cache = SessionCache::new();
        for key in 0..MAX_SESSION_CACHE_ENTRIES as u128 {
            cache.insert(key, user("u", "a"), FAR_FUTURE, cache.generation());
        }
        cache.insert(u128::MAX, user("u", "a"), FAR_FUTURE, cache.generation());
        assert!(cache.get(0).is_none());
        assert!(cache.get(u128::MAX).is_some());
    }
}
