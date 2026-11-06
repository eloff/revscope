//! Symmetric encryption for integration credentials at rest.
//!
//! Integration API keys and secrets are stored encrypted in the main Postgres
//! `integrations.encrypted_api_key` / `encrypted_secret` columns. There is no
//! authentication tag: these values are never accepted from or exposed to an
//! attacker who could tamper with the ciphertext, so a confidentiality-only
//! stream cipher (ChaCha20, RustCrypto) is sufficient. Each ciphertext is
//! stored as `{key-id}_{nonce || ciphertext}`. Unprefixed (legacy) values are
//! decrypted with key id `key1`.
//!
//! Key material is not compiled in. `INTEGRATION_ENCRYPTION_KEY` is the 32-byte
//! ChaCha20 key for key id `key1`, encoded as 64 hex characters.

use std::sync::OnceLock;

use chacha20::ChaCha20;
use chacha20::cipher::{KeyIvInit, StreamCipher};

fn cipher(key: &[u8; 32], nonce: &[u8; NONCE_LEN]) -> ChaCha20 {
    ChaCha20::new_from_slices(key, nonce)
        .expect("ChaCha20 key and nonce lengths are constant and valid")
}

/// Identifies the key used by [`encrypt`]. Stored ciphertext is prefixed with
/// `{ENCRYPTION_KEY_ID}_` so additional keys can be added later without
/// invalidating existing credentials.
const ENCRYPTION_KEY_ID: &str = "key1";

/// 64 hex characters: the 32-byte key for [`ENCRYPTION_KEY_ID`].
const ENCRYPTION_KEY_ENV: &str = "INTEGRATION_ENCRYPTION_KEY";

static ENCRYPTION_KEY: OnceLock<[u8; 32]> = OnceLock::new();

/// Round-trips in `cargo test` when [`ENCRYPTION_KEY_ENV`] is unset. Release
/// builds have no compiled-in key.
#[cfg(test)]
const TEST_ENCRYPTION_KEY_HEX: &str =
    "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

/// Loads [`ENCRYPTION_KEY_ENV`] and caches it. Startup calls this so a
/// deployment without the key exits before it listens.
pub fn require_integration_encryption_key() -> Result<(), String> {
    if ENCRYPTION_KEY.get().is_some() {
        return Ok(());
    }
    let key = load_encryption_key()?;
    // A concurrent caller may have initialized the cache first.
    let _ = ENCRYPTION_KEY.set(key);
    Ok(())
}

fn encryption_key() -> &'static [u8; 32] {
    ENCRYPTION_KEY.get_or_init(|| load_encryption_key().unwrap_or_else(|error| panic!("{error}")))
}

fn load_encryption_key() -> Result<[u8; 32], String> {
    parse_encryption_key(&configured_encryption_key_hex()?)
}

fn configured_encryption_key_hex() -> Result<String, String> {
    let value = std::env::var(ENCRYPTION_KEY_ENV).unwrap_or_default();
    let value = value.trim();
    if value.is_empty() {
        #[cfg(test)]
        {
            return Ok(TEST_ENCRYPTION_KEY_HEX.to_string());
        }
        #[cfg(not(test))]
        {
            return Err(format!(
                "{ENCRYPTION_KEY_ENV} must be set to a 32-byte key (64 hex characters)"
            ));
        }
    }
    Ok(value.to_string())
}

fn parse_encryption_key(hex_key: &str) -> Result<[u8; 32], String> {
    let bytes = hex_key.as_bytes();
    if bytes.len() != 64 {
        return Err(format!(
            "{ENCRYPTION_KEY_ENV} must be 64 hex characters (32 bytes)"
        ));
    }
    let mut key = [0u8; 32];
    for index in 0..32 {
        let Some(high) = crate::util::hex_nibble(bytes[index * 2]) else {
            return Err(format!("{ENCRYPTION_KEY_ENV} must be hexadecimal"));
        };
        let Some(low) = crate::util::hex_nibble(bytes[index * 2 + 1]) else {
            return Err(format!("{ENCRYPTION_KEY_ENV} must be hexadecimal"));
        };
        key[index] = (high << 4) | low;
    }
    Ok(key)
}

fn key_for_id(id: &str) -> Option<&'static [u8; 32]> {
    match id {
        "key1" => Some(encryption_key()),
        _ => None,
    }
}

/// Splits `{key-id}_{secret}`. A key-id is `key` followed by one or more digits.
/// Legacy blobs with no such prefix default to `key1` (copied, not
/// [`ENCRYPTION_KEY_ID`]) so existing secrets still decrypt after the current
/// key id is rotated.
fn split_key_id(stored: &[u8]) -> (&str, &[u8]) {
    let Some(after_key) = stored.strip_prefix(b"key") else {
        return ("key1", stored);
    };
    let digit_len = after_key.iter().take_while(|b| b.is_ascii_digit()).count();
    if digit_len == 0 || after_key.get(digit_len) != Some(&b'_') {
        return ("key1", stored);
    }
    let id_len = 3 + digit_len;
    let Ok(id) = std::str::from_utf8(&stored[..id_len]) else {
        return ("key1", stored);
    };
    (id, &stored[id_len + 1..])
}

const NONCE_LEN: usize = 12;

/// Byte equality whose running time depends only on the lengths, so a caller
/// comparing a secret does not leak how much of a guess was right.
pub(crate) fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut diff = left.len() ^ right.len();
    for index in 0..left.len().max(right.len()) {
        let left_byte = left.get(index).copied().unwrap_or(0);
        let right_byte = right.get(index).copied().unwrap_or(0);
        diff |= (left_byte ^ right_byte) as usize;
    }
    diff == 0
}

/// Encrypts `plaintext`, returning `{key-id}_{nonce || ciphertext}`. The 12-byte
/// random nonce makes repeated encryptions of the same value distinct.
pub(crate) fn encrypt(plaintext: &[u8]) -> Vec<u8> {
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::getrandom(&mut nonce)
        .expect("system RNG must be available for credential encryption");

    let key = encryption_key();
    let prefix = ENCRYPTION_KEY_ID.as_bytes();
    let mut output = Vec::with_capacity(prefix.len() + 1 + NONCE_LEN + plaintext.len());
    output.extend_from_slice(prefix);
    output.push(b'_');
    output.extend_from_slice(&nonce);
    output.extend_from_slice(plaintext);

    let body_start = prefix.len() + 1 + NONCE_LEN;
    cipher(key, &nonce).apply_keystream(&mut output[body_start..]);
    output
}

/// Reverses [`encrypt`]. Returns `None` when `stored` is too short to contain a
/// nonce, or the key-id prefix is unknown (e.g. corrupted data).
pub(crate) fn decrypt(stored: &[u8]) -> Option<Vec<u8>> {
    let (key_id, body) = split_key_id(stored);
    let key = key_for_id(key_id)?;
    if body.len() < NONCE_LEN {
        return None;
    }
    let (nonce, ciphertext) = body.split_at(NONCE_LEN);
    let mut plaintext = ciphertext.to_vec();
    let nonce: [u8; NONCE_LEN] = nonce.try_into().ok()?;
    cipher(key, &nonce).apply_keystream(&mut plaintext);
    Some(plaintext)
}

/// Decrypts a stored credential and masks it for display in GitHub style:
/// a recognizable prefix, a fixed bullet run, and a short suffix. Shorter
/// secrets and multi-line values (e.g. PEM keys) are fully masked.
pub(crate) fn mask_stored_secret(stored: &[u8]) -> Option<String> {
    let plaintext = decrypt(stored)?;
    let value = String::from_utf8_lossy(&plaintext);
    Some(mask_value(value.as_ref()))
}

const MASK_MIDDLE: &str = "••••••••";
const MIN_PREFIX_LEN: usize = 4;
const SUFFIX_LEN: usize = 4;
/// Look for a trailing `_` in this window so prefixes like `sk_live_` / `polar_oat_` stay intact.
const PREFIX_SCAN_LEN: usize = 12;
/// Require at least this many hidden characters between prefix and suffix.
const MIN_HIDDEN_LEN: usize = 4;

fn mask_value(value: &str) -> String {
    // PEM / multi-line secrets have no useful short prefix to show.
    if value.contains('\n') || value.contains("PRIVATE KEY") {
        return MASK_MIDDLE.to_string();
    }

    let chars: Vec<char> = value.chars().collect();
    if chars.len() < MIN_PREFIX_LEN + SUFFIX_LEN + MIN_HIDDEN_LEN {
        return MASK_MIDDLE.to_string();
    }

    let scan_end = (chars.len() - SUFFIX_LEN).min(PREFIX_SCAN_LEN);
    let prefix_len = chars[..scan_end]
        .iter()
        .rposition(|&c| c == '_')
        .map(|i| i + 1)
        .filter(|&len| len >= MIN_PREFIX_LEN)
        .unwrap_or(MIN_PREFIX_LEN)
        .min(chars.len() - SUFFIX_LEN);

    if chars.len() - prefix_len - SUFFIX_LEN < MIN_HIDDEN_LEN {
        return MASK_MIDDLE.to_string();
    }

    let prefix: String = chars[..prefix_len].iter().collect();
    let suffix: String = chars[chars.len() - SUFFIX_LEN..].iter().collect();
    format!("{prefix}{MASK_MIDDLE}{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_id_prefix() -> Vec<u8> {
        let mut prefix = ENCRYPTION_KEY_ID.as_bytes().to_vec();
        prefix.push(b'_');
        prefix
    }

    /// Pre-key-id format: `nonce || ciphertext` with no `{key-id}_` prefix.
    fn encrypt_legacy(plaintext: &[u8]) -> Vec<u8> {
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::getrandom(&mut nonce).expect("system RNG must be available");
        let mut output = Vec::with_capacity(NONCE_LEN + plaintext.len());
        output.extend_from_slice(&nonce);
        output.extend_from_slice(plaintext);
        cipher(encryption_key(), &nonce).apply_keystream(&mut output[NONCE_LEN..]);
        output
    }

    #[test]
    fn encrypt_then_decrypt_round_trips() {
        let secret = b"sk_live_super_secret_value_1234";
        let stored = encrypt(secret);
        let prefix = key_id_prefix();
        assert!(stored.starts_with(&prefix));
        let body = &stored[prefix.len()..];
        assert_ne!(&body[NONCE_LEN..], secret.as_slice());
        assert_eq!(decrypt(&stored).as_deref(), Some(secret.as_slice()));
    }

    #[test]
    fn encrypt_prefixes_current_key_id() {
        let stored = encrypt(b"secret");
        assert!(stored.starts_with(&key_id_prefix()));
    }

    #[test]
    fn decrypt_legacy_unprefixed_ciphertext() {
        let secret = b"sk_live_legacy_unprefixed";
        let stored = encrypt_legacy(secret);
        assert_eq!(decrypt(&stored).as_deref(), Some(secret.as_slice()));
    }

    #[test]
    fn decrypt_rejects_unknown_key_id() {
        let mut stored = b"key99_".to_vec();
        stored.extend_from_slice(&encrypt_legacy(b"secret"));
        assert_eq!(decrypt(&stored), None);
    }

    #[test]
    fn encrypt_uses_fresh_nonce_each_call() {
        let secret = b"repeated value";
        assert_ne!(encrypt(secret), encrypt(secret));
    }

    #[test]
    fn mask_keeps_key_prefix_and_suffix() {
        let stored = encrypt(b"sk_live_abcdef123456");
        assert_eq!(
            mask_stored_secret(&stored).as_deref(),
            Some("sk_live_••••••••3456")
        );
    }

    #[test]
    fn mask_falls_back_to_fixed_prefix_without_underscore() {
        let stored = encrypt(b"abcdefghijklmnop");
        assert_eq!(
            mask_stored_secret(&stored).as_deref(),
            Some("abcd••••••••mnop")
        );
    }

    #[test]
    fn mask_keeps_longer_underscore_prefix() {
        assert_eq!(
            mask_value("polar_oat_abcdefghij").as_str(),
            "polar_oat_••••••••ghij"
        );
    }

    #[test]
    fn mask_short_secret_is_fully_hidden() {
        let stored = encrypt(b"short");
        assert_eq!(mask_stored_secret(&stored).as_deref(), Some("••••••••"));
    }

    #[test]
    fn mask_pem_secret_is_fully_hidden() {
        let pem = "-----BEGIN PRIVATE KEY-----\nABC123\n-----END PRIVATE KEY-----";
        assert_eq!(mask_value(pem).as_str(), "••••••••");
    }

    #[test]
    fn decrypt_rejects_truncated_input() {
        assert_eq!(decrypt(&[0u8; 4]), None);
    }

    #[test]
    fn parses_a_32_byte_hex_key() {
        let key = parse_encryption_key(TEST_ENCRYPTION_KEY_HEX).unwrap();
        assert_eq!(key[0], 0x00);
        assert_eq!(key[31], 0xff);
        assert!(parse_encryption_key(&TEST_ENCRYPTION_KEY_HEX.to_ascii_uppercase()).is_ok());
    }

    #[test]
    fn rejects_malformed_encryption_keys() {
        assert!(parse_encryption_key("abcd").is_err());
        assert!(parse_encryption_key(&"ab".repeat(31)).is_err());
        assert!(parse_encryption_key(&"zz".repeat(32)).is_err());
    }
}
