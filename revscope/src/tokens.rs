//! Opaque bearer tokens: how they are minted, how long they live, and where
//! they are kept.
//!
//! Seven kinds exist, and they differ only in lifetime and storage:
//!
//! * download tokens ([`crate::download_db`]) live in memory, because a lost
//!   token only costs the dashboard one extra request to mint another;
//! * demo Ask challenge tokens ([`crate::chat`]) are not kept at all: they
//!   prove a browser interacted with the public demo before spending tokens,
//!   and are signed so that minting them, which anyone may do, costs no memory;
//! * MCP authorization codes ([`crate::mcp`]) live in memory too: they are
//!   single-use and only bridge the browser redirect to the token request;
//! * query tokens ([`crate::query_token`]) outlive the process, so only their
//!   hash is stored and the plaintext is shown to the caller once;
//! * MCP access tokens ([`crate::mcp::token`]) are stored the same way and
//!   differ only in how long they last and how they are obtained;
//! * sessions and password reset tokens ([`crate::auth`]) are stored the same
//!   way, but travel as a [`crate::uuid_token::UuidToken`] rather than a
//!   [`TokenSecret`].
//!
//! The lifetimes live here together so they can be compared at a glance, and
//! the Postgres-backed kinds share one cleanup, run by
//! [`crate::main_db_cleanup`].

use std::fmt;
use std::net::IpAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use dashmap::DashMap;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

use crate::auth::AuthError;
use crate::db::open_main_database_write;
use crate::server_context::ServerContext;
use crate::util::seconds_since_epoch;

/// How long a dashboard login lasts before the user must log in again.
pub(crate) const SESSION_MAX_AGE_SECONDS: i32 = 30 * 24 * 60 * 60;

/// A year. A CLI is configured once, often by an agent running unattended, so its
/// credential outlives a browser login. Logout and password resets revoke it.
pub(crate) const API_TOKEN_TTL_SECONDS: i32 = 365 * 24 * 60 * 60;

/// Emailed, and it lets the holder take over an account, so it is short.
pub(crate) const PASSWORD_RESET_TTL_SECONDS: i32 = 60 * 60;

/// An invite is a password reset link for a user who has no password yet. It
/// only works for that new user, so it can wait a few days to be accepted.
pub(crate) const INVITE_TTL_SECONDS: i32 = 7 * 24 * 60 * 60;

/// Long enough to start a download later in the working day, short enough that
/// a leaked URL stops working the same day.
pub(crate) const DOWNLOAD_TOKEN_TTL_SECONDS: u32 = 12 * 60 * 60;

/// Long enough for a visitor to ask several demo questions in one sitting,
/// short enough that a scraped token stops working before it can be widely
/// reused. Bound to the client IP at mint time.
pub(crate) const DEMO_CHAT_CHALLENGE_TTL_SECONDS: u32 = 15 * 60;

/// Matches the dashboard session lifetime: an agent configured once with a
/// query token keeps working for as long as a browser login would.
pub(crate) const QUERY_TOKEN_TTL_SECONDS: i32 = 30 * 24 * 60 * 60;

/// A year. Re-authorizing an MCP client costs the user a browser round trip
/// through the consent page, so the token outlives a session deliberately;
/// revoking them is a matter of deleting the `kind = 'mcp'` rows.
pub(crate) const MCP_TOKEN_TTL_SECONDS: i32 = 365 * 24 * 60 * 60;

/// Only has to survive the redirect from the consent page back to the client,
/// which then immediately exchanges it. Single use as well as short-lived.
pub(crate) const MCP_AUTHORIZATION_CODE_TTL_SECONDS: u32 = 5 * 60;

const SECRET_HEX_LEN: usize = 32;

/// An [`ExpiringTokens`] map is swept no sooner than this size, so small maps
/// are never swept at all and large ones only after they have doubled.
const MIN_SWEEP_AT: usize = 1024;

/// The expiry, as 8 hex characters, then a 128 bit truncated HMAC.
const CHALLENGE_EXPIRY_HEX_LEN: usize = 8;
const CHALLENGE_MAC_LEN: usize = 16;
const CHALLENGE_HEX_LEN: usize = CHALLENGE_EXPIRY_HEX_LEN + CHALLENGE_MAC_LEN * 2;

/// Postgres tables whose rows are tokens with an `expires_at` column.
const EXPIRING_TOKEN_TABLES: [&str; 3] = ["sessions", "password_reset_tokens", "api_tokens"];

/// A 128 bit random secret, written as 32 lowercase hex characters.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct TokenSecret(u128);

impl TokenSecret {
    pub(crate) fn generate() -> Result<Self, AuthError> {
        random_u128().map(Self)
    }

    /// Wraps an id that is already unguessable, such as a session id, so it
    /// can key an [`ExpiringTokens`] map.
    pub(crate) fn from_raw(value: u128) -> Self {
        Self(value)
    }

    /// Reads back the hex form. Anything else is not a token we minted.
    pub(crate) fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        if value.len() != SECRET_HEX_LEN || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        u128::from_str_radix(value, 16).ok().map(Self)
    }

    /// What is stored for tokens that outlive the process, so a leak of the
    /// database alone does not hand out working tokens.
    pub(crate) fn hash(&self) -> Vec<u8> {
        hash_secret(self.0)
    }
}

pub(crate) fn random_u128() -> Result<u128, AuthError> {
    let mut bytes = [0_u8; 16];
    getrandom::getrandom(&mut bytes)
        .map_err(|error| AuthError::Db(format!("failed to generate token: {error}")))?;
    Ok(u128::from_be_bytes(bytes))
}

pub(crate) fn hash_secret(secret: u128) -> Vec<u8> {
    Sha256::digest(secret.to_be_bytes()).to_vec()
}

impl fmt::Display for TokenSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:032x}", self.0)
    }
}

/// Keeps secrets out of logs and error reports.
impl fmt::Debug for TokenSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TokenSecret(redacted)")
    }
}

/// A freshly minted in-memory token and the second it stops working.
pub(crate) struct IssuedToken {
    pub(crate) secret: TokenSecret,
    /// Seconds since the unix epoch.
    pub(crate) expires_at: u32,
}

struct Granted<T> {
    value: T,
    expires_at: u32,
}

/// Tokens held in memory until they expire: a lookup is one hash map hit and
/// nothing survives a restart.
pub(crate) struct ExpiringTokens<T> {
    granted: DashMap<u128, Granted<T>>,
    /// The size at which the next insert sweeps out expired entries.
    sweep_at: AtomicUsize,
}

impl<T: Clone> ExpiringTokens<T> {
    pub(crate) fn new() -> Self {
        Self {
            granted: DashMap::new(),
            sweep_at: AtomicUsize::new(MIN_SWEEP_AT),
        }
    }

    pub(crate) fn issue(&self, value: T, ttl_seconds: u32) -> Result<IssuedToken, AuthError> {
        let secret = TokenSecret::generate()?;
        let expires_at = seconds_since_epoch().saturating_add(ttl_seconds);
        self.insert(secret, value, expires_at);
        Ok(IssuedToken { secret, expires_at })
    }

    /// Expired entries are dropped on the way through: nothing grows the map
    /// but an insert. A sweep walks the whole map, so it only runs once the map
    /// has doubled since the last one (and never below [`MIN_SWEEP_AT`]
    /// entries), which keeps inserts O(1) amortized. The map can therefore hold
    /// up to that many expired entries; [`Self::len`] counts them.
    pub(crate) fn insert(&self, secret: TokenSecret, value: T, expires_at: u32) {
        if self.granted.len() >= self.sweep_at.load(Ordering::Relaxed) {
            self.sweep_expired();
        }
        self.granted.insert(secret.0, Granted { value, expires_at });
    }

    /// Drops every expired entry now, rather than waiting for the next
    /// doubling.
    pub(crate) fn sweep_expired(&self) {
        let now = seconds_since_epoch();
        self.granted.retain(|_, granted| now < granted.expires_at);
        let next = (self.granted.len() * 2).max(MIN_SWEEP_AT);
        self.sweep_at.store(next, Ordering::Relaxed);
    }

    /// The value the token grants, or `None` once it has expired or was never
    /// issued. An expired entry is removed as it is read.
    pub(crate) fn get(&self, secret: TokenSecret) -> Option<T> {
        let granted = self.granted.get(&secret.0)?;
        if seconds_since_epoch() < granted.expires_at {
            return Some(granted.value.clone());
        }
        drop(granted);
        self.granted.remove(&secret.0);
        None
    }

    /// The value the token grants, removing it either way. Single-use grants
    /// such as OAuth authorization codes redeem themselves through this, so a
    /// replayed code finds nothing.
    pub(crate) fn take(&self, secret: TokenSecret) -> Option<T> {
        let (_, granted) = self.granted.remove(&secret.0)?;
        (seconds_since_epoch() < granted.expires_at).then_some(granted.value)
    }

    /// Drops every grant for which `keep` returns false. Used when the rows
    /// behind a cache have been deleted, so this process does not keep serving
    /// them for the rest of the TTL.
    pub(crate) fn retain(&self, mut keep: impl FnMut(&T) -> bool) {
        self.granted.retain(|_, granted| keep(&granted.value));
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.granted.is_empty()
    }

    /// Entries held, expired ones included until a sweep or read drops them.
    pub(crate) fn len(&self) -> usize {
        self.granted.len()
    }
}

/// A freshly signed challenge and the second it stops working.
pub(crate) struct IssuedChallenge {
    pub(crate) token: String,
    /// Seconds since the unix epoch.
    pub(crate) expires_at: u32,
}

/// Tokens that bind a client IP to an expiry by signing both, so checking one
/// needs nothing but the key. The key is random per process: a restart voids
/// every outstanding challenge, as an in-memory map would. A challenge can be
/// presented again until it expires, but only from the IP it was minted for.
pub(crate) struct SignedChallenges {
    key: [u8; 32],
}

impl SignedChallenges {
    pub(crate) fn new() -> Result<Self, AuthError> {
        let mut key = [0_u8; 32];
        getrandom::getrandom(&mut key)
            .map_err(|error| AuthError::Db(format!("failed to generate challenge key: {error}")))?;
        Ok(Self { key })
    }

    pub(crate) fn issue(&self, client_ip: IpAddr, ttl_seconds: u32) -> IssuedChallenge {
        let expires_at = seconds_since_epoch().saturating_add(ttl_seconds);
        let tag = self.mac(client_ip, expires_at).finalize().into_bytes();
        let mut truncated = [0_u8; CHALLENGE_MAC_LEN];
        truncated.copy_from_slice(&tag[..CHALLENGE_MAC_LEN]);
        let token = format!("{expires_at:08x}{:032x}", u128::from_be_bytes(truncated));
        IssuedChallenge { token, expires_at }
    }

    /// Whether `token` was signed by this process for `client_ip` and has not
    /// yet expired. The signature is compared in constant time.
    pub(crate) fn verify(&self, token: &str, client_ip: IpAddr) -> bool {
        let token = token.trim();
        if token.len() != CHALLENGE_HEX_LEN || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return false;
        }
        let (expiry_hex, mac_hex) = token.split_at(CHALLENGE_EXPIRY_HEX_LEN);
        let (Ok(expires_at), Ok(tag)) = (
            u32::from_str_radix(expiry_hex, 16),
            u128::from_str_radix(mac_hex, 16),
        ) else {
            return false;
        };
        if seconds_since_epoch() >= expires_at {
            return false;
        }
        self.mac(client_ip, expires_at)
            .verify_truncated_left(&tag.to_be_bytes())
            .is_ok()
    }

    fn mac(&self, client_ip: IpAddr, expires_at: u32) -> Hmac<Sha256> {
        let mut mac =
            Hmac::<Sha256>::new_from_slice(&self.key).expect("HMAC accepts any key length");
        // The leading version byte keeps a v4 address from colliding with a v6 one.
        match client_ip {
            IpAddr::V4(ip) => {
                mac.update(&[4]);
                mac.update(&ip.octets());
            }
            IpAddr::V6(ip) => {
                mac.update(&[6]);
                mac.update(&ip.octets());
            }
        }
        mac.update(&expires_at.to_be_bytes());
        mac
    }
}

/// Deletes rows from the Postgres token tables once they expire. Expiry is
/// enforced by every read, so this only keeps the tables from growing.
pub(crate) async fn delete_expired_tokens(
    server_context: Arc<ServerContext>,
) -> Result<(), AuthError> {
    let conn = open_main_database_write(server_context).await?;
    for table in EXPIRING_TOKEN_TABLES {
        conn.execute(
            &format!("DELETE FROM {table} WHERE expires_at < NOW()"),
            &[],
        )
        .await
        .map_err(AuthError::from)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn len_counts_expired_entries_until_a_sweep_drops_them() {
        let tokens = ExpiringTokens::new();
        for raw in 0..3 {
            tokens.insert(TokenSecret::from_raw(raw), (), 0);
        }
        let live = TokenSecret::from_raw(3);
        tokens.insert(live, (), seconds_since_epoch() + 60);
        // Below MIN_SWEEP_AT nothing sweeps on insert.
        assert_eq!(tokens.len(), 4);
        tokens.sweep_expired();
        assert_eq!(tokens.len(), 1);
        assert!(tokens.get(live).is_some());
    }

    #[test]
    fn a_secret_round_trips_through_its_hex_form() {
        let secret = TokenSecret::generate().expect("generate");
        let hex = secret.to_string();
        assert_eq!(hex.len(), SECRET_HEX_LEN);
        assert_eq!(TokenSecret::parse(&hex), Some(secret));
    }

    #[test]
    fn parsing_rejects_anything_that_is_not_the_hex_form() {
        assert_eq!(TokenSecret::parse(""), None);
        assert_eq!(TokenSecret::parse("abc"), None);
        assert_eq!(TokenSecret::parse(&"z".repeat(SECRET_HEX_LEN)), None);
        assert_eq!(TokenSecret::parse(&"0".repeat(SECRET_HEX_LEN + 1)), None);
    }

    #[test]
    fn the_secret_is_not_recoverable_from_its_hash_or_its_debug_output() {
        let secret = TokenSecret::generate().expect("generate");
        assert_eq!(secret.hash().len(), 32);
        assert_ne!(secret.hash(), secret.to_string().into_bytes());
        assert_eq!(format!("{secret:?}"), "TokenSecret(redacted)");
    }

    #[test]
    fn an_issued_token_grants_its_value_until_it_expires() {
        let tokens = ExpiringTokens::new();
        let issued = tokens.issue(7_i32, 60).expect("issue");
        assert!(issued.expires_at > seconds_since_epoch());
        assert_eq!(tokens.get(issued.secret), Some(7));

        tokens.insert(issued.secret, 7, seconds_since_epoch() - 1);
        assert_eq!(tokens.get(issued.secret), None);
        assert!(tokens.is_empty());
    }

    #[test]
    fn taking_a_token_redeems_it_exactly_once() {
        let tokens = ExpiringTokens::new();
        let issued = tokens.issue(9_i32, 60).expect("issue");
        assert_eq!(tokens.take(issued.secret), Some(9));
        assert_eq!(tokens.take(issued.secret), None);
        assert!(tokens.is_empty());

        let expired = TokenSecret::generate().expect("generate");
        tokens.insert(expired, 3_i32, seconds_since_epoch() - 1);
        assert_eq!(tokens.take(expired), None);
    }

    #[test]
    fn retain_drops_grants_that_do_not_match() {
        let tokens = ExpiringTokens::new();
        let keep = tokens.issue("keep".to_string(), 60).expect("issue");
        let drop = tokens.issue("drop".to_string(), 60).expect("issue");
        tokens.retain(|value| value == "keep");
        assert_eq!(tokens.get(keep.secret), Some("keep".to_string()));
        assert_eq!(tokens.get(drop.secret), None);
    }

    #[test]
    fn inserting_sweeps_expired_tokens_only_once_the_map_has_grown() {
        let tokens = ExpiringTokens::new();
        let expired_at = seconds_since_epoch() - 1;
        for _ in 0..MIN_SWEEP_AT - 1 {
            let stale = TokenSecret::generate().expect("generate");
            tokens.insert(stale, 1_i32, expired_at);
        }
        let live = tokens.issue(2_i32, 60).expect("issue");
        assert_eq!(tokens.len(), MIN_SWEEP_AT, "no sweep below the threshold");

        let next = tokens.issue(3_i32, 60).expect("issue");
        assert_eq!(tokens.len(), 2, "the sweep keeps only live tokens");
        assert_eq!(tokens.get(live.secret), Some(2));
        assert_eq!(tokens.get(next.secret), Some(3));
    }

    #[test]
    fn a_signed_challenge_verifies_only_for_its_ip_until_it_expires() {
        let challenges = SignedChallenges::new().expect("key");
        let ip: IpAddr = "203.0.113.10".parse().expect("ip");
        let other: IpAddr = "203.0.113.20".parse().expect("ip");
        let issued = challenges.issue(ip, 60);
        assert_eq!(issued.token.len(), CHALLENGE_HEX_LEN);
        assert!(issued.expires_at > seconds_since_epoch());
        assert!(challenges.verify(&issued.token, ip));
        assert!(!challenges.verify(&issued.token, other));

        let expired = challenges.issue(ip, 0);
        assert!(!challenges.verify(&expired.token, ip));
    }

    #[test]
    fn a_signed_challenge_rejects_tampering_and_other_keys() {
        let challenges = SignedChallenges::new().expect("key");
        let ip: IpAddr = "2001:db8::1".parse().expect("ip");
        let issued = challenges.issue(ip, 60);

        let later = format!("{:08x}{}", issued.expires_at + 3600, &issued.token[8..]);
        assert!(!challenges.verify(&later, ip));

        let last = issued.token.chars().last().expect("non-empty");
        let flipped = if last == '0' { '1' } else { '0' };
        let forged = format!("{}{flipped}", &issued.token[..issued.token.len() - 1]);
        assert!(!challenges.verify(&forged, ip));

        let elsewhere = SignedChallenges::new().expect("key");
        assert!(!elsewhere.verify(&issued.token, ip));

        assert!(!challenges.verify("", ip));
        assert!(!challenges.verify(&issued.token[1..], ip));
        assert!(!challenges.verify(&"z".repeat(CHALLENGE_HEX_LEN), ip));
    }
}
