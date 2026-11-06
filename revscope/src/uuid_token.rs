//! Bearer tokens that travel as random (version 4) UUIDs: session cookies and
//! password reset links. Like [`crate::tokens::TokenSecret`], only their hash
//! is stored, so a leak of the database alone does not hand out working tokens.

use std::fmt;

use crate::auth::AuthError;
use crate::tokens::{hash_secret, random_u128};
use crate::uuid_text::{format_uuid, parse_hyphenated_uuid};

const VERSION_MASK: u128 = 0xF << 76;
const VERSION_4: u128 = 0x4 << 76;
const VARIANT_MASK: u128 = 0x3 << 62;
const RFC_4122_VARIANT: u128 = 0x2 << 62;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct UuidToken(u128);

impl UuidToken {
    /// 122 random bits; the other six are the version and variant.
    pub(crate) fn generate() -> Result<Self, AuthError> {
        random_u128().map(Self::from_random)
    }

    fn from_random(random: u128) -> Self {
        Self((random & !(VERSION_MASK | VARIANT_MASK)) | VERSION_4 | RFC_4122_VARIANT)
    }

    /// Reads back the hyphenated form we hand out. Anything else, including a
    /// UUID of another version, is not a token we minted.
    pub(crate) fn parse(value: &str) -> Option<Self> {
        let raw = parse_hyphenated_uuid(value.trim())?;
        (raw & VERSION_MASK == VERSION_4 && raw & VARIANT_MASK == RFC_4122_VARIANT)
            .then_some(Self(raw))
    }

    /// What is stored in Postgres in place of the token.
    pub(crate) fn hash(&self) -> Vec<u8> {
        hash_secret(self.0)
    }

    /// Keys in-memory maps such as the session cache. Never stored.
    pub(crate) fn key(&self) -> u128 {
        self.0
    }
}

impl fmt::Display for UuidToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&format_uuid(self.0))
    }
}

/// Keeps tokens out of logs and error reports.
impl fmt::Debug for UuidToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("UuidToken(redacted)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_random_value_becomes_a_v4_uuid() {
        for random in [0, u128::MAX, 0x0198f0a1_2b3c_7d4e_cf90_1a2b3c4d5e6f] {
            let token = UuidToken::from_random(random);
            let text = token.to_string();
            assert_eq!(&text[14..15], "4", "{text}");
            assert!(matches!(&text[19..20], "8" | "9" | "a" | "b"), "{text}");
            assert_eq!(UuidToken::parse(&text), Some(token));
        }
    }

    #[test]
    fn a_generated_token_round_trips_in_either_case() {
        let token = UuidToken::generate().expect("generate");
        let text = token.to_string();
        assert_eq!(UuidToken::parse(&text), Some(token));
        assert_eq!(UuidToken::parse(&text.to_ascii_uppercase()), Some(token));
        assert_eq!(UuidToken::parse(&format!(" {text}\n")), Some(token));
    }

    #[test]
    fn two_tokens_differ() {
        let first = UuidToken::generate().expect("generate");
        let second = UuidToken::generate().expect("generate");
        assert_ne!(first, second);
        assert_ne!(first.hash(), second.hash());
    }

    #[test]
    fn parsing_rejects_anything_but_a_hyphenated_v4_uuid() {
        let token = UuidToken::generate().expect("generate").to_string();
        assert_eq!(UuidToken::parse(""), None);
        assert_eq!(UuidToken::parse(&token.replace('-', "")), None);
        // The UUIDv7 sessions issued before tokens were hashed.
        assert_eq!(
            UuidToken::parse("0198f0a1-2b3c-7d4e-8f90-1a2b3c4d5e6f"),
            None
        );
        // Version 4 but not the RFC 4122 variant.
        assert_eq!(
            UuidToken::parse("0198f0a1-2b3c-4d4e-cf90-1a2b3c4d5e6f"),
            None
        );
    }

    #[test]
    fn the_token_is_not_recoverable_from_its_hash_or_its_debug_output() {
        let token = UuidToken::generate().expect("generate");
        assert_eq!(token.hash().len(), 32);
        assert_ne!(token.hash(), token.key().to_be_bytes().to_vec());
        assert_eq!(format!("{token:?}"), "UuidToken(redacted)");
    }
}
