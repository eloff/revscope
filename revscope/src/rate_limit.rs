//! Token-bucket rate limits, mostly for unauthenticated routes. The auth routes (login,
//! signup, forgot password) would otherwise let anyone guess passwords, spend
//! server CPU on password hashing, or send reset emails without bound; tracker
//! error reports and MCP client registration would let anyone write to the main
//! database without bound.
//!
//! Buckets live in memory, so the limits are per process and reset on restart.
//! That is enough to make online guessing impractical and to keep one client
//! from filling a table; it is not an audit log.

use std::collections::hash_map::RandomState;
use std::net::IpAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use sha2::{Digest, Sha256};

/// A bucket map is swept no sooner than this size; after that, only once it
/// has doubled since the last sweep, so inserts stay O(1) amortized.
const MIN_SWEEP_AT: usize = 1024;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Rate {
    /// Requests allowed back to back before the limit applies.
    burst: u32,
    /// How long it takes to earn back one request.
    every: Duration,
}

impl Rate {
    const fn new(burst: u32, every_seconds: u64) -> Self {
        Self {
            burst,
            every: Duration::from_secs(every_seconds),
        }
    }

    fn full_after(self) -> Duration {
        self.every * self.burst
    }
}

/// A shared office or NAT may put several people behind one address, so the
/// per-IP limits leave room for that.
///
/// The per-email limits are keyed by email and address together, because a
/// limit on the email alone would let anyone lock its owner out of login and
/// password reset by spending it first. The cost is that guesses at one
/// account, or reset emails to one inbox, are capped per address rather than
/// overall, so a distributed attacker is held only by the per-IP limits.
const LOGIN_PER_IP: Rate = Rate::new(20, 15);
const LOGIN_PER_EMAIL: Rate = Rate::new(10, 60);
const SIGNUP_PER_IP: Rate = Rate::new(5, 5 * 60);
const FORGOT_PASSWORD_PER_IP: Rate = Rate::new(5, 2 * 60);
const FORGOT_PASSWORD_PER_EMAIL: Rate = Rate::new(3, 10 * 60);
/// The tracker only reports errors thrown by its own script, so a real visitor
/// rarely gets near this.
const ERROR_REPORT_PER_IP: Rate = Rate::new(30, 10);
/// Hosted MCP clients register once per user who connects them, all from the
/// platform's own addresses, so this is generous. Registrations that never get
/// a token are deleted after a day.
const MCP_REGISTER_PER_IP: Rate = Rate::new(60, 10);
/// A generous allowance for an account setting up its projects' domains, while
/// keeping any one account far below Let's Encrypt's 300 new orders per 3 hours.
pub(crate) const CERTIFICATE_ORDERS_PER_ACCOUNT: Rate = Rate::new(10, 60 * 60);
/// DNS checks cost no certificate orders, so they need their own limit: a slow
/// nameserver makes each one hold a lookup place for many seconds.
pub(crate) const CERTIFICATE_DNS_CHECKS_PER_ACCOUNT: Rate = Rate::new(30, 60);
/// Accounts are cheap to create, so the per-account limit alone can't protect
/// Let's Encrypt's limit. This allows at most 160 orders in any 3 hours, which
/// leaves the rest of the 300 for renewals and the platform certificate.
pub(crate) const CERTIFICATE_ORDERS_PER_DEPLOYMENT: Rate = Rate::new(100, 3 * 60);

/// Every invite can email a stranger text the inviter wrote, from our domain.
/// The burst covers bringing a whole team on at once.
const INVITES_PER_ACCOUNT: Rate = Rate::new(20, 30 * 60);

#[derive(Clone, Copy)]
struct Bucket {
    /// When the bucket will be full again. A bucket is just this instant: the
    /// tokens it holds at `now` are `burst - (full_at - now) / every`.
    full_at: Instant,
}

/// One rate applied separately to each key.
pub(crate) struct RateLimiter {
    rate: Rate,
    /// Keyed by [`bucket_key`], so a client-chosen key of any length costs the
    /// same few bytes. Seeded rather than FxHash: the digest is public, so
    /// keys could be ground to collide into one shard.
    buckets: DashMap<u128, Bucket, RandomState>,
    sweep_at: AtomicUsize,
}

impl RateLimiter {
    pub(crate) fn new(rate: Rate) -> Self {
        Self {
            rate,
            buckets: DashMap::with_hasher(RandomState::new()),
            sweep_at: AtomicUsize::new(MIN_SWEEP_AT),
        }
    }

    /// Takes one request from `key`'s bucket, or returns how long until one is
    /// available.
    pub(crate) fn check(&self, key: &str) -> Result<(), Duration> {
        self.check_at(key, Instant::now())
    }

    fn check_at(&self, key: &str, now: Instant) -> Result<(), Duration> {
        if self.buckets.len() >= self.sweep_at.load(Ordering::Relaxed) {
            self.sweep(now);
        }
        let full_after = self.rate.full_after();
        let mut bucket = self
            .buckets
            .entry(bucket_key(key))
            .or_insert(Bucket { full_at: now });
        // Spending a token pushes the refill time out by one interval; the
        // bucket is empty once that would put it more than `full_after` away.
        let full_at = bucket.full_at.max(now) + self.rate.every;
        let ahead = full_at - now;
        if ahead > full_after {
            return Err(ahead - full_after);
        }
        bucket.full_at = full_at;
        Ok(())
    }

    /// Gives back a request taken by [`Self::check`], for one that turned out
    /// to cost nothing worth limiting.
    pub(crate) fn refund(&self, key: &str) {
        if let Some(mut bucket) = self.buckets.get_mut(&bucket_key(key)) {
            bucket.full_at = bucket
                .full_at
                .checked_sub(self.rate.every)
                .unwrap_or(bucket.full_at);
        }
    }

    /// Drops buckets that have refilled, since a fresh bucket behaves the same.
    fn sweep(&self, now: Instant) {
        self.buckets.retain(|_, bucket| bucket.full_at > now);
        let next = (self.buckets.len() * 2).max(MIN_SWEEP_AT);
        self.sweep_at.store(next, Ordering::Relaxed);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.buckets.len()
    }
}

pub(crate) struct AuthRateLimits {
    pub(crate) login_ip: RateLimiter,
    pub(crate) login_email: RateLimiter,
    pub(crate) signup_ip: RateLimiter,
    pub(crate) forgot_password_ip: RateLimiter,
    pub(crate) forgot_password_email: RateLimiter,
    pub(crate) invite_account: RateLimiter,
}

impl AuthRateLimits {
    pub(crate) fn new() -> Self {
        Self {
            login_ip: RateLimiter::new(LOGIN_PER_IP),
            login_email: RateLimiter::new(LOGIN_PER_EMAIL),
            signup_ip: RateLimiter::new(SIGNUP_PER_IP),
            forgot_password_ip: RateLimiter::new(FORGOT_PASSWORD_PER_IP),
            forgot_password_email: RateLimiter::new(FORGOT_PASSWORD_PER_EMAIL),
            invite_account: RateLimiter::new(INVITES_PER_ACCOUNT),
        }
    }
}

pub(crate) struct PublicWriteRateLimits {
    pub(crate) error_report_ip: RateLimiter,
    pub(crate) mcp_register_ip: RateLimiter,
}

impl PublicWriteRateLimits {
    pub(crate) fn new() -> Self {
        Self {
            error_report_ip: RateLimiter::new(ERROR_REPORT_PER_IP),
            mcp_register_ip: RateLimiter::new(MCP_REGISTER_PER_IP),
        }
    }
}

fn bucket_key(key: &str) -> u128 {
    let digest = Sha256::digest(key.as_bytes());
    let mut prefix = [0_u8; 16];
    prefix.copy_from_slice(&digest[..16]);
    u128::from_be_bytes(prefix)
}

/// The key an address is limited under. IPv6 users are routinely handed a
/// whole /64, so limiting single addresses there would limit nothing. A
/// dual-stack listener sees IPv4 clients as IPv4-mapped IPv6, which would
/// otherwise put them all in one /64.
pub(crate) fn ip_key(ip: IpAddr) -> String {
    match crate::ip_ranges::canonical_ip(ip) {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => {
            let segments = v6.segments();
            format!(
                "{:x}:{:x}:{:x}:{:x}::/64",
                segments[0], segments[1], segments[2], segments[3]
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_a_burst_then_refills_one_request_per_interval() {
        let limiter = RateLimiter::new(Rate::new(3, 10));
        let start = Instant::now();
        for _ in 0..3 {
            assert!(limiter.check_at("a", start).is_ok());
        }
        assert_eq!(limiter.check_at("a", start), Err(Duration::from_secs(10)));
        assert_eq!(
            limiter.check_at("a", start + Duration::from_secs(4)),
            Err(Duration::from_secs(6))
        );
        let later = start + Duration::from_secs(10);
        assert!(limiter.check_at("a", later).is_ok());
        assert!(limiter.check_at("a", later).is_err());
    }

    #[test]
    fn rejected_requests_do_not_push_the_refill_out() {
        let limiter = RateLimiter::new(Rate::new(1, 10));
        let start = Instant::now();
        assert!(limiter.check_at("a", start).is_ok());
        for _ in 0..100 {
            assert!(limiter.check_at("a", start).is_err());
        }
        assert!(
            limiter
                .check_at("a", start + Duration::from_secs(10))
                .is_ok()
        );
    }

    #[test]
    fn a_refund_returns_one_request() {
        let limiter = RateLimiter::new(Rate::new(1, 60));
        let now = Instant::now();
        assert!(limiter.check_at("a", now).is_ok());
        limiter.refund("a");
        assert!(limiter.check_at("a", now).is_ok());
        assert!(limiter.check_at("a", now).is_err());
    }

    #[test]
    fn keys_have_separate_buckets() {
        let limiter = RateLimiter::new(Rate::new(1, 60));
        let now = Instant::now();
        assert!(limiter.check_at("a", now).is_ok());
        assert!(limiter.check_at("a", now).is_err());
        assert!(limiter.check_at("b", now).is_ok());
    }

    #[test]
    fn a_long_idle_bucket_holds_no_more_than_its_burst() {
        let limiter = RateLimiter::new(Rate::new(2, 10));
        let start = Instant::now();
        assert!(limiter.check_at("a", start).is_ok());
        let much_later = start + Duration::from_secs(3600);
        assert!(limiter.check_at("a", much_later).is_ok());
        assert!(limiter.check_at("a", much_later).is_ok());
        assert!(limiter.check_at("a", much_later).is_err());
    }

    #[test]
    fn sweep_drops_only_refilled_buckets() {
        let limiter = RateLimiter::new(Rate::new(1, 10));
        let start = Instant::now();
        for index in 0..MIN_SWEEP_AT {
            assert!(limiter.check_at(&index.to_string(), start).is_ok());
        }
        let refilled = start + Duration::from_secs(10);
        assert!(limiter.check_at("fresh", refilled).is_ok());
        assert_eq!(limiter.len(), 1);
        assert!(limiter.check_at("fresh", refilled).is_err());
    }

    #[test]
    fn ipv6_addresses_share_their_64_prefix() {
        let first: IpAddr = "2001:db8:1:2::1".parse().unwrap();
        let second: IpAddr = "2001:db8:1:2:ffff::9".parse().unwrap();
        let other: IpAddr = "2001:db8:1:3::1".parse().unwrap();
        assert_eq!(ip_key(first), ip_key(second));
        assert_ne!(ip_key(first), ip_key(other));
        assert_eq!(ip_key("198.51.100.7".parse().unwrap()), "198.51.100.7");
        assert_eq!(
            ip_key("::ffff:198.51.100.7".parse().unwrap()),
            "198.51.100.7"
        );
    }
}
