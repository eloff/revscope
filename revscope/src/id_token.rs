//! Sign in with Google or Apple: the browser gets an ID token from the provider and sends it to
//! us, and we check its signature against the provider's published keys. We never exchange codes
//! with them, so the server needs no client secret, only the client id tokens must be issued to.

use std::time::{Duration, Instant};

use fxhash::FxHashMap;
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Deserializer};
use tokio::sync::Mutex;

/// Who issues ID tokens and how to check them.
pub(crate) struct Provider {
    /// The provider's name in `user_identities` and the API.
    pub(crate) id: &'static str,
    certs_url: &'static str,
    issuers: &'static [&'static str],
    /// The token's fault: forged, expired, or for another app.
    pub(crate) refused: &'static str,
    /// Ours or the network's: the provider's keys couldn't be fetched.
    pub(crate) unreachable: &'static str,
    /// The token carries no email, so there is no address to sign up with.
    pub(crate) no_email: &'static str,
    pub(crate) unverified_email: &'static str,
    pub(crate) no_account: &'static str,
    pub(crate) already_linked: &'static str,
    /// The address matches a user, but the provider doesn't host it, so it can't vouch for it.
    pub(crate) email_not_hosted: &'static str,
    /// Whether the provider hosts this address, given the token's `hd` claim.
    /// A provider marks an address verified once, when it is added. Only for an
    /// address it hosts does that still say who reads it, so only those link to
    /// an existing user: a former employee's personal account on a work address
    /// would otherwise join the company's user.
    hosts_email: fn(email: &str, hosted_domain: Option<&str>) -> bool,
}

fn email_domain_is(email: &str, domains: &[&str]) -> bool {
    email
        .rsplit_once('@')
        .is_some_and(|(_, domain)| domains.iter().any(|d| domain.eq_ignore_ascii_case(d)))
}

/// Google's docs: authoritative for Gmail, and for Workspace accounts, which carry `hd`.
fn google_hosts_email(email: &str, hosted_domain: Option<&str>) -> bool {
    hosted_domain.is_some_and(|hd| !hd.trim().is_empty())
        || email_domain_is(email, &["gmail.com", "googlemail.com"])
}

/// Apple IDs can use any address; Apple hosts only its own and its relay's.
fn apple_hosts_email(email: &str, _hosted_domain: Option<&str>) -> bool {
    email_domain_is(
        email,
        &[
            "icloud.com",
            "me.com",
            "mac.com",
            "privaterelay.appleid.com",
        ],
    )
}

pub(crate) const GOOGLE: Provider = Provider {
    id: "google",
    certs_url: "https://www.googleapis.com/oauth2/v3/certs",
    issuers: &["accounts.google.com", "https://accounts.google.com"],
    refused: "Google sign-in didn't work. Try again.",
    unreachable: "Couldn't reach Google just now. Try again shortly.",
    no_email: "Google didn't share your email address.",
    unverified_email: "Google hasn't verified your email address yet.",
    no_account: "No RevScope account uses this Google account yet. Sign up first.",
    already_linked: "This Google account already has a RevScope account. Log in instead.",
    email_not_hosted: "An account with this email already exists. Google can only vouch for Gmail \
         and Google Workspace addresses, so log in the way you signed up, or use Forgot password.",
    hosts_email: google_hosts_email,
};

pub(crate) const APPLE: Provider = Provider {
    id: "apple",
    certs_url: "https://appleid.apple.com/auth/keys",
    issuers: &["https://appleid.apple.com"],
    refused: "Apple sign-in didn't work. Try again.",
    unreachable: "Couldn't reach Apple just now. Try again shortly.",
    no_email: "Apple didn't share your email address.",
    unverified_email: "Apple hasn't verified your email address yet.",
    no_account: "No RevScope account uses this Apple ID yet. Sign up first.",
    already_linked: "This Apple ID already has a RevScope account. Log in instead.",
    email_not_hosted: "An account with this email already exists. Apple can only vouch for iCloud \
         addresses, so log in the way you signed up, or use Forgot password.",
    hosts_email: apple_hosts_email,
};

/// Google rotates keys every few days and says how long to cache them; this is the fallback. Apple
/// says `no-store`, which we override with this too: a token naming a key we lack refetches anyway.
const DEFAULT_KEYS_TTL: Duration = Duration::from_secs(3600);
/// A token naming a key we don't have refetches the keys at most this often, so made-up key ids
/// can't make us hammer the provider.
const MIN_REFETCH: Duration = Duration::from_secs(60);
/// After a failed fetch, sign-ins wait this long before trying again, so an
/// outage doesn't have every one of them queue on the lock for a fetch of its own.
const FETCH_FAILURE_BACKOFF: Duration = Duration::from_secs(10);
/// Shorter than the shared client's timeout: sign-ins wait on it holding the lock.
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
/// Google and Apple ID tokens are about a kilobyte.
pub(crate) const MAX_TOKEN_LEN: usize = 8 * 1024;

/// Why an ID token wasn't accepted; `Refused` is the token's fault, `Failed` ours or the
/// network's. The detail is for logs only.
#[derive(Debug)]
pub(crate) enum SignInError {
    Refused(String),
    Failed(String),
}

/// What the provider vouches for.
#[derive(Debug)]
pub(crate) struct Identity {
    /// The provider's stable id for the person; unlike the email, it never changes.
    pub(crate) subject: String,
    pub(crate) email: Option<String>,
    pub(crate) email_verified: bool,
    /// Verified, and at an address the provider hosts, so it still vouches for who reads it.
    pub(crate) provider_hosts_email: bool,
    /// Google puts it in the token; Apple only hands it to the browser, on the first sign-in.
    pub(crate) name: Option<String>,
}

/// Checks one provider's ID tokens.
pub(crate) struct Verifier {
    pub(crate) provider: &'static Provider,
    /// The OAuth client id our web app signs in with (`aud` of its tokens). The browser learns it
    /// from `/v2/public/auth/providers`.
    pub(crate) client_id: String,
    keys: Mutex<KeyCache>,
}

#[derive(Default)]
struct KeyCache {
    keys: Option<Keys>,
    failed_at: Option<Instant>,
}

struct Keys {
    by_kid: FxHashMap<String, DecodingKey>,
    fetched: Instant,
    until: Instant,
}

#[derive(Deserialize)]
struct Claims {
    sub: String,
    #[serde(default)]
    email: Option<String>,
    #[serde(default, deserialize_with = "bool_or_string")]
    email_verified: bool,
    #[serde(default)]
    name: Option<String>,
    /// Google's Workspace domain.
    #[serde(default)]
    hd: Option<String>,
}

/// Apple has sent `email_verified` both as a JSON boolean and as the string `"true"`.
fn bool_or_string<'de, D: Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Flag {
        Bool(bool),
        Text(String),
    }
    Ok(match Flag::deserialize(deserializer)? {
        Flag::Bool(value) => value,
        Flag::Text(text) => text == "true",
    })
}

impl Verifier {
    pub(crate) fn new(provider: &'static Provider, client_id: String) -> Verifier {
        Verifier {
            provider,
            client_id,
            keys: Mutex::new(KeyCache::default()),
        }
    }

    /// A verifier that trusts only `key` (as key id `kid`) and never fetches, for tests that sign
    /// their own tokens.
    pub(crate) fn with_key(
        provider: &'static Provider,
        client_id: String,
        kid: &str,
        key: DecodingKey,
    ) -> Verifier {
        let now = Instant::now();
        let mut by_kid = FxHashMap::default();
        by_kid.insert(kid.to_owned(), key);
        Verifier {
            provider,
            client_id,
            keys: Mutex::new(KeyCache {
                keys: Some(Keys {
                    by_kid,
                    fetched: now,
                    until: now + Duration::from_secs(100 * 365 * 24 * 3600),
                }),
                failed_at: None,
            }),
        }
    }

    pub(crate) async fn verify(
        &self,
        http: &reqwest::Client,
        token: &str,
    ) -> Result<Identity, SignInError> {
        let kid = jsonwebtoken::decode_header(token)
            .map_err(|e| SignInError::Refused(format!("bad token header: {e}")))?
            .kid
            .ok_or_else(|| SignInError::Refused("token names no key".into()))?;
        let key = self.key(http, &kid).await?;
        identity(token, &key, self.provider, &self.client_id)
    }

    /// The decoding key `kid`, fetching the provider's keys if ours are stale or
    /// don't have it. While the provider can't be reached, stale keys still serve.
    async fn key(&self, http: &reqwest::Client, kid: &str) -> Result<DecodingKey, SignInError> {
        let mut cache = self.keys.lock().await;
        let now = Instant::now();
        let backing_off = cache
            .failed_at
            .is_some_and(|failed| now.duration_since(failed) < FETCH_FAILURE_BACKOFF);
        let mut failure = None;
        if !backing_off && should_fetch(cache.keys.as_ref(), kid, now) {
            match self.fetch_keys(http).await {
                Ok(keys) => {
                    cache.keys = Some(keys);
                    cache.failed_at = None;
                }
                Err(error) => {
                    cache.failed_at = Some(Instant::now());
                    failure = Some(error);
                }
            }
        }
        if let Some(key) = cache.keys.as_ref().and_then(|k| k.by_kid.get(kid)) {
            return Ok(key.clone());
        }
        Err(match failure {
            Some(error) => error,
            None if backing_off => {
                SignInError::Failed(format!("{} keys unavailable", self.provider.id))
            }
            None => SignInError::Refused(format!("unknown key {kid}")),
        })
    }

    async fn fetch_keys(&self, http: &reqwest::Client) -> Result<Keys, SignInError> {
        let id = self.provider.id;
        let failed = |e: reqwest::Error| SignInError::Failed(format!("fetching {id} keys: {e}"));
        let response = http
            .get(self.provider.certs_url)
            .timeout(FETCH_TIMEOUT)
            .send()
            .await
            .map_err(failed)?
            .error_for_status()
            .map_err(failed)?;
        let ttl = response
            .headers()
            .get(reqwest::header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok())
            .and_then(max_age)
            .unwrap_or(DEFAULT_KEYS_TTL);
        let set: JwkSet = response.json().await.map_err(failed)?;
        // A key we can't use (another algorithm, say) is skipped rather than failing the rest.
        let by_kid = set
            .keys
            .iter()
            .filter_map(|jwk| {
                let kid = jwk.common.key_id.clone()?;
                Some((kid, DecodingKey::from_jwk(jwk).ok()?))
            })
            .collect();
        let fetched = Instant::now();
        Ok(Keys {
            by_kid,
            fetched,
            until: fetched + ttl,
        })
    }
}

/// Whether to fetch the provider's keys before checking a token signed with `kid`: when we have
/// none, they're stale, or they lack `kid` (the provider rotated) and we haven't fetched in a while.
fn should_fetch(keys: Option<&Keys>, kid: &str, now: Instant) -> bool {
    let Some(keys) = keys else {
        return true;
    };
    now >= keys.until
        || (!keys.by_kid.contains_key(kid) && now.duration_since(keys.fetched) >= MIN_REFETCH)
}

/// Who a token signed by `key` vouches for, if it's an unexpired ID token from `provider` for
/// `client_id`.
fn identity(
    token: &str,
    key: &DecodingKey,
    provider: &Provider,
    client_id: &str,
) -> Result<Identity, SignInError> {
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_audience(&[client_id]);
    validation.set_issuer(provider.issuers);
    validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
    let claims = jsonwebtoken::decode::<Claims>(token, key, &validation)
        .map_err(|e| SignInError::Refused(e.to_string()))?
        .claims;
    if claims.sub.is_empty() {
        return Err(SignInError::Refused("empty subject".into()));
    }
    let email = claims.email.filter(|email| !email.trim().is_empty());
    let provider_hosts_email = claims.email_verified
        && email
            .as_deref()
            .is_some_and(|email| (provider.hosts_email)(email.trim(), claims.hd.as_deref()));
    Ok(Identity {
        subject: claims.sub,
        email,
        email_verified: claims.email_verified,
        provider_hosts_email,
        name: claims.name.filter(|name| !name.trim().is_empty()),
    })
}

/// `max-age` from a Cache-Control header.
fn max_age(header: &str) -> Option<Duration> {
    header
        .split(',')
        .filter_map(|part| part.trim().strip_prefix("max-age="))
        .find_map(|secs| secs.parse().ok())
        .map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header};
    use serde_json::json;

    /// A throwaway 2048-bit RSA key standing in for Google's and Apple's.
    const PRIVATE_KEY: &str = include_str!("../testdata/id-token-test-key.pem");
    const PUBLIC_KEY: &str = include_str!("../testdata/id-token-test-key.pub.pem");

    fn token(claims: serde_json::Value) -> String {
        let key = EncodingKey::from_rsa_pem(PRIVATE_KEY.as_bytes()).unwrap();
        jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &key).unwrap()
    }

    fn claims(aud: &str, iss: &str, exp_offset: i64) -> serde_json::Value {
        json!({
            "iss": iss,
            "aud": aud,
            "sub": "1234567890",
            "email": "ada@example.com",
            "email_verified": true,
            "name": "Ada Lovelace",
            "exp": crate::util::milliseconds_since_epoch() / 1000 + exp_offset,
        })
    }

    fn public_key() -> DecodingKey {
        DecodingKey::from_rsa_pem(PUBLIC_KEY.as_bytes()).unwrap()
    }

    #[test]
    fn a_provider_vouches_only_for_verified_addresses_it_hosts() {
        let key = public_key();
        let google = |email: &str, hd: Option<&str>, verified: bool| {
            let mut claims = claims("web-client", "https://accounts.google.com", 600);
            claims["email"] = json!(email);
            claims["email_verified"] = json!(verified);
            if let Some(hd) = hd {
                claims["hd"] = json!(hd);
            }
            identity(&token(claims), &key, &GOOGLE, "web-client")
                .unwrap()
                .provider_hosts_email
        };
        assert!(google("ada@Gmail.com", None, true));
        assert!(google("ada@googlemail.com", None, true));
        assert!(google("ada@corp.example", Some("corp.example"), true));
        assert!(!google("ada@corp.example", None, true));
        assert!(!google("ada@corp.example", Some(" "), true));
        assert!(!google("ada@gmail.com", None, false));
        assert!(!google("ada@gmail.com.evil.example", None, true));

        let apple = |email: &str| {
            let mut claims = claims("co.revscope.web", "https://appleid.apple.com", 600);
            claims["email"] = json!(email);
            claims["hd"] = json!("corp.example");
            identity(&token(claims), &key, &APPLE, "co.revscope.web")
                .unwrap()
                .provider_hosts_email
        };
        assert!(apple("ada@icloud.com"));
        assert!(apple("x1@privaterelay.appleid.com"));
        assert!(!apple("ada@corp.example"), "Apple's tokens carry no hd");
    }

    #[test]
    fn accepts_only_unexpired_google_tokens_for_our_client() {
        let key = public_key();
        let check = |claims| identity(&token(claims), &key, &GOOGLE, "web-client");

        let found = check(claims("web-client", "https://accounts.google.com", 600)).unwrap();
        assert_eq!(found.subject, "1234567890");
        assert_eq!(found.email.as_deref(), Some("ada@example.com"));
        assert!(found.email_verified);
        assert_eq!(found.name.as_deref(), Some("Ada Lovelace"));
        assert!(check(claims("web-client", "accounts.google.com", 600)).is_ok());

        assert!(check(claims("someone-else", "accounts.google.com", 600)).is_err());
        assert!(check(claims("web-client", "https://evil.example", 600)).is_err());
        assert!(check(claims("web-client", "accounts.google.com", -600)).is_err());
        let mut no_sub = claims("web-client", "accounts.google.com", 600);
        no_sub.as_object_mut().unwrap().remove("sub");
        assert!(check(no_sub).is_err());
    }

    #[test]
    fn accepts_each_provider_only_its_own_issuer() {
        let key = public_key();
        let apple = token(claims("co.revscope.web", "https://appleid.apple.com", 600));
        let google = token(claims("co.revscope.web", "accounts.google.com", 600));

        assert!(identity(&apple, &key, &APPLE, "co.revscope.web").is_ok());
        assert!(identity(&google, &key, &APPLE, "co.revscope.web").is_err());
        assert!(identity(&apple, &key, &GOOGLE, "co.revscope.web").is_err());
    }

    #[test]
    fn reads_apples_string_email_verified_and_missing_claims() {
        let key = public_key();
        let mut apple = claims("co.revscope.web", "https://appleid.apple.com", 600);
        apple["email_verified"] = json!("true");
        apple.as_object_mut().unwrap().remove("name");
        let found = identity(&token(apple.clone()), &key, &APPLE, "co.revscope.web").unwrap();
        assert!(found.email_verified);
        assert_eq!(found.name, None);

        apple["email_verified"] = json!("false");
        apple.as_object_mut().unwrap().remove("email");
        let found = identity(&token(apple), &key, &APPLE, "co.revscope.web").unwrap();
        assert!(!found.email_verified);
        assert_eq!(found.email, None);
    }

    #[test]
    fn refuses_tokens_signed_with_another_key_or_algorithm() {
        let key = public_key();
        let good = token(claims("web-client", "accounts.google.com", 600));
        let (header, rest) = good.split_once('.').unwrap();
        let (payload, _) = rest.split_once('.').unwrap();
        let forged = format!("{header}.{payload}.AAAA");
        assert!(identity(&forged, &key, &GOOGLE, "web-client").is_err());

        let hmac = jsonwebtoken::encode(
            &Header::new(Algorithm::HS256),
            &claims("web-client", "accounts.google.com", 600),
            &EncodingKey::from_secret(PUBLIC_KEY.as_bytes()),
        )
        .unwrap();
        assert!(identity(&hmac, &key, &GOOGLE, "web-client").is_err());
    }

    #[test]
    fn fetches_keys_when_missing_stale_or_rotated_but_not_for_every_unknown_key() {
        let start = Instant::now();
        let mut by_kid = FxHashMap::default();
        by_kid.insert("k1".to_owned(), public_key());
        let keys = Keys {
            by_kid,
            fetched: start,
            until: start + Duration::from_secs(3600),
        };
        assert!(should_fetch(None, "k1", start));
        assert!(!should_fetch(Some(&keys), "k1", start));
        assert!(!should_fetch(
            Some(&keys),
            "k2",
            start + Duration::from_secs(5)
        ));
        assert!(should_fetch(Some(&keys), "k2", start + MIN_REFETCH));
        assert!(should_fetch(
            Some(&keys),
            "k1",
            start + Duration::from_secs(3600)
        ));
    }

    #[tokio::test]
    async fn serves_stale_keys_and_skips_fetching_while_backing_off_a_failure() {
        let verifier = Verifier::with_key(&GOOGLE, "web-client".into(), "k1", public_key());
        {
            let mut cache = verifier.keys.lock().await;
            let now = Instant::now();
            let keys = cache.keys.as_mut().unwrap();
            keys.fetched = now - Duration::from_secs(7200);
            keys.until = now - Duration::from_secs(3600);
            cache.failed_at = Some(now);
        }
        // A client that can't connect anywhere: any fetch would fail.
        let http = reqwest::Client::builder()
            .proxy(reqwest::Proxy::all("http://127.0.0.1:1").unwrap())
            .build()
            .unwrap();
        assert!(verifier.key(&http, "k1").await.is_ok());
        assert!(matches!(
            verifier.key(&http, "k2").await,
            Err(SignInError::Failed(_))
        ));
    }

    #[test]
    fn reads_max_age() {
        assert_eq!(
            max_age("public, max-age=19745, must-revalidate, no-transform"),
            Some(Duration::from_secs(19745))
        );
        assert_eq!(max_age("no-cache"), None);
    }
}
