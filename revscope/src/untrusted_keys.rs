//! Maps and sets keyed by values an outside party chooses, such as paths,
//! referrers and UTM values sent to the public tracker endpoint. FxHash is
//! unkeyed, so such a party could craft keys that all collide and make each
//! insert linear; std's randomly seeded SipHash prevents that.

#![allow(
    clippy::disallowed_types,
    reason = "the one place the std maps are allowed, for HashDoS resistance"
)]

pub(crate) type UntrustedKeyMap<K, V> = std::collections::HashMap<K, V>;
pub(crate) type UntrustedKeySet<T> = std::collections::HashSet<T>;
