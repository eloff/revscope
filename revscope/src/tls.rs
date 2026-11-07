use std::fmt;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use fxhash::{FxHashMap, FxHashSet};
use parking_lot::RwLock;
use rustls::ServerConfig;
use rustls::crypto::aws_lc_rs;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::ResolvesServerCert;
use rustls::sign::CertifiedKey;
use x509_parser::prelude::{FromDer, GeneralName, X509Certificate};

/// The certificate chain and key currently served for one or more names, with
/// the leaf's `notAfter` so the renewal task can decide when to order a
/// replacement.
struct StoredCertificate {
    certified_key: Arc<CertifiedKey>,
    not_after: SystemTime,
    names: Vec<String>,
}

/// Certificates the TLS listener can serve, keyed by the SNI hostname they
/// cover. A renewal replaces the entries in place, so new handshakes pick up
/// the new certificate without a restart and without disturbing established
/// connections.
#[derive(Default)]
pub(crate) struct CertificateStore {
    by_name: RwLock<FxHashMap<String, Arc<StoredCertificate>>>,
}

impl CertificateStore {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Parses a PEM certificate chain and private key and publishes them under
    /// each of `names`, returning the leaf certificate's expiry. Fails if the
    /// leaf does not list every one of `names`, so a cached certificate for
    /// hostnames this deployment no longer serves is not installed.
    pub(crate) fn install_pem(
        &self,
        names: &[String],
        certificate_chain_pem: &str,
        private_key_pem: &str,
    ) -> Result<SystemTime, CertificateError> {
        if names.is_empty() {
            return Err(CertificateError::NoNames);
        }
        let chain = CertificateDer::pem_slice_iter(certificate_chain_pem.as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| CertificateError::Pem(error.to_string()))?;
        let Some(leaf) = chain.first() else {
            return Err(CertificateError::EmptyChain);
        };
        let not_after = check_leaf(leaf, names)?;
        let private_key = PrivateKeyDer::from_pem_slice(private_key_pem.as_bytes())
            .map_err(|error| CertificateError::Pem(error.to_string()))?;
        let signing_key =
            aws_lc_rs::sign::any_supported_type(&private_key).map_err(CertificateError::Key)?;

        let certified_key = CertifiedKey::new(chain, signing_key);
        // The certificate and key are separate files, so a crash during a renewal
        // can leave a new certificate beside the old key. Handshakes would fail
        // for months; rejecting the pair makes the renewal task order a new one.
        certified_key.keys_match().map_err(CertificateError::Key)?;

        let stored = Arc::new(StoredCertificate {
            certified_key: Arc::new(certified_key),
            not_after,
            names: names.to_vec(),
        });
        let mut by_name = self.by_name.write();
        for name in names {
            by_name.insert(name.clone(), stored.clone());
        }
        Ok(not_after)
    }

    pub(crate) fn certified_key(&self, name: &str) -> Option<Arc<CertifiedKey>> {
        self.by_name
            .read()
            .get(name)
            .map(|stored| stored.certified_key.clone())
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn not_after(&self, name: &str) -> Option<SystemTime> {
        self.by_name.read().get(name).map(|stored| stored.not_after)
    }

    /// Distinct installed certificates, each with the names they were published
    /// under, so the renewal task can order a replacement per certificate
    /// rather than per SAN.
    pub(crate) fn unique_certificates(&self) -> Vec<(Vec<String>, SystemTime)> {
        let by_name = self.by_name.read();
        let mut seen = FxHashSet::default();
        let mut certificates = Vec::new();
        for stored in by_name.values() {
            let Some(canonical) = stored.names.first() else {
                continue;
            };
            if !seen.insert(canonical.clone()) {
                continue;
            }
            for name in &stored.names {
                seen.insert(name.clone());
            }
            certificates.push((stored.names.clone(), stored.not_after));
        }
        certificates
    }
}

impl fmt::Debug for CertificateStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<String> = self.by_name.read().keys().cloned().collect();
        formatter
            .debug_struct("CertificateStore")
            .field("names", &names)
            .finish()
    }
}

#[derive(Debug)]
pub(crate) enum CertificateError {
    Pem(String),
    EmptyChain,
    /// `install_pem` was called without a hostname to publish the certificate under.
    NoNames,
    /// The leaf certificate parsed but carries a validity we cannot read.
    Validity(String),
    /// The leaf certificate does not list this hostname among its names.
    MissingName(String),
    Key(rustls::Error),
}

impl fmt::Display for CertificateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pem(message) => write!(formatter, "invalid PEM: {message}"),
            Self::EmptyChain => write!(formatter, "certificate chain is empty"),
            Self::NoNames => write!(formatter, "certificate has no names to serve"),
            Self::Validity(message) => {
                write!(formatter, "unreadable certificate validity: {message}")
            }
            Self::MissingName(name) => write!(formatter, "certificate does not cover {name}"),
            Self::Key(error) => write!(formatter, "unusable private key: {error}"),
        }
    }
}

/// Returns the leaf's `notAfter` after checking that its subject alternative
/// names include every one of `names`.
fn check_leaf(leaf: &CertificateDer<'_>, names: &[String]) -> Result<SystemTime, CertificateError> {
    let (_, certificate) = X509Certificate::from_der(leaf.as_ref())
        .map_err(|error| CertificateError::Validity(error.to_string()))?;
    let san_names = certificate
        .subject_alternative_name()
        .ok()
        .flatten()
        .map(|extension| {
            extension
                .value
                .general_names
                .iter()
                .filter_map(|name| match name {
                    GeneralName::DNSName(dns_name) => Some(*dns_name),
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if let Some(missing) = names.iter().find(|name| {
        !san_names
            .iter()
            .any(|san_name| san_name.eq_ignore_ascii_case(name))
    }) {
        return Err(CertificateError::MissingName(missing.clone()));
    }
    let seconds = u64::try_from(certificate.validity().not_after.timestamp())
        .map_err(|_| CertificateError::Validity("notAfter precedes the unix epoch".to_string()))?;
    Ok(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds))
}

/// rustls and jsonwebtoken each keep a separate process-wide `CryptoProvider`.
///
/// rustls 0.23 will not pick a default when both `aws-lc-rs` and `ring` are
/// enabled. Our tree enables both: we and reqwest 0.13 want aws-lc-rs, while
/// dodopayments' reqwest 0.12 enables ring. ACME's hyper-rustls client then
/// panics in `ClientConfig::builder()` unless rustls's provider is installed.
///
/// jsonwebtoken 10 copies that pattern but does not read rustls's choice.
/// With neither of its `aws_lc_rs` / `rust_crypto` features selected it
/// installs a stub that panics on the first `encode`, which is what signs
/// the Google Search Console service-account assertion. Installing its
/// aws-lc-rs provider here covers that path; the crate feature is what makes
/// the provider exist, and what `get_default` falls back to if this runs late.
pub fn install_default_crypto_provider() {
    let _ = aws_lc_rs::default_provider().install_default();
    let _ = jsonwebtoken::crypto::aws_lc::DEFAULT_PROVIDER.install_default();
}

/// Server-side TLS settings. The provider is passed explicitly so the
/// handshake does not depend on which crate happened to install a process-wide
/// default first. The resolver picks a certificate from SNI.
pub(crate) fn server_config(resolver: Arc<dyn ResolvesServerCert>) -> Arc<ServerConfig> {
    let mut config = ServerConfig::builder_with_provider(Arc::new(aws_lc_rs::default_provider()))
        .with_safe_default_protocol_versions()
        .expect("aws-lc-rs supports the default protocol versions")
        .with_no_client_auth()
        .with_cert_resolver(resolver);
    // The server speaks HTTP/1.1 only, so never let a client negotiate h2.
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Arc::new(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_NOT_AFTER_UNIX: i64 = 2_000_000_000;

    /// Self-signed certificate for `example.com` expiring at a known instant.
    fn self_signed_certificate(names: &[&str]) -> (String, String) {
        let mut params = rcgen::CertificateParams::new(
            names
                .iter()
                .map(|name| (*name).to_string())
                .collect::<Vec<_>>(),
        )
        .expect("certificate parameters");
        params.not_after = time::OffsetDateTime::from_unix_timestamp(TEST_NOT_AFTER_UNIX)
            .expect("representable notAfter");
        let signing_key = rcgen::KeyPair::generate().expect("generate key pair");
        let certificate = params.self_signed(&signing_key).expect("self-sign");
        (certificate.pem(), signing_key.serialize_pem())
    }

    fn name_list(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_string()).collect()
    }

    #[test]
    fn client_config_builder_uses_installed_provider() {
        install_default_crypto_provider();
        let _ = rustls::ClientConfig::builder();
    }

    #[test]
    fn jsonwebtoken_signs_with_installed_provider() {
        install_default_crypto_provider();
        let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_RSA_SHA256).expect("rsa key");
        let encoding_key = jsonwebtoken::EncodingKey::from_rsa_pem(key.serialize_pem().as_bytes())
            .expect("rsa pem");
        let token = jsonwebtoken::encode(
            &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256),
            &serde_json::json!({ "sub": "gsc" }),
            &encoding_key,
        )
        .expect("sign service-account assertion");
        assert_eq!(token.split('.').count(), 3);
    }

    #[test]
    fn installs_a_pem_certificate_and_reads_its_expiry() {
        let expected_not_after =
            SystemTime::UNIX_EPOCH + Duration::from_secs(TEST_NOT_AFTER_UNIX as u64);
        let (certificate_pem, key_pem) = self_signed_certificate(&["example.com"]);
        let store = CertificateStore::new();
        assert!(store.not_after("example.com").is_none());
        assert!(store.certified_key("example.com").is_none());

        let not_after = store
            .install_pem(&name_list(&["example.com"]), &certificate_pem, &key_pem)
            .expect("install certificate");

        assert_eq!(not_after, expected_not_after);
        assert_eq!(store.not_after("example.com"), Some(expected_not_after));
        assert!(store.certified_key("example.com").is_some());
        assert!(store.certified_key("other.example.com").is_none());
    }

    #[test]
    fn publishes_one_certificate_under_each_name() {
        let (certificate_pem, key_pem) =
            self_signed_certificate(&["api.revscope.co", "eu1.revscope.co"]);
        let store = CertificateStore::new();
        store
            .install_pem(
                &name_list(&["api.revscope.co", "eu1.revscope.co"]),
                &certificate_pem,
                &key_pem,
            )
            .expect("install certificate");

        let api = store.certified_key("api.revscope.co").expect("api cert");
        let eu1 = store.certified_key("eu1.revscope.co").expect("eu1 cert");
        assert_eq!(api.cert, eu1.cert);
        assert_eq!(store.unique_certificates().len(), 1);
    }

    #[test]
    fn replacing_a_certificate_is_visible_to_new_lookups() {
        let (first_pem, first_key_pem) = self_signed_certificate(&["example.com"]);
        let (second_pem, second_key_pem) = self_signed_certificate(&["example.com"]);
        let store = CertificateStore::new();
        store
            .install_pem(&name_list(&["example.com"]), &first_pem, &first_key_pem)
            .expect("install first certificate");
        let first = store.certified_key("example.com").expect("first key");

        store
            .install_pem(&name_list(&["example.com"]), &second_pem, &second_key_pem)
            .expect("install second certificate");

        let second = store.certified_key("example.com").expect("second key");
        assert_ne!(first.cert, second.cert);
    }

    #[test]
    fn rejects_a_certificate_with_another_certificates_key() {
        let (certificate_pem, _) = self_signed_certificate(&["example.com"]);
        let (_, other_key_pem) = self_signed_certificate(&["example.com"]);
        let store = CertificateStore::new();

        let error = store
            .install_pem(
                &name_list(&["example.com"]),
                &certificate_pem,
                &other_key_pem,
            )
            .expect_err("mismatched key is rejected");

        assert!(matches!(error, CertificateError::Key(_)), "{error:?}");
        assert!(store.certified_key("example.com").is_none());
    }

    #[test]
    fn rejects_a_chain_without_a_certificate() {
        let (_, key_pem) = self_signed_certificate(&["example.com"]);
        let store = CertificateStore::new();

        let error = store
            .install_pem(&name_list(&["example.com"]), "", &key_pem)
            .expect_err("empty chain is rejected");

        assert!(matches!(error, CertificateError::EmptyChain), "{error:?}");
        assert!(store.not_after("example.com").is_none());
    }

    #[test]
    fn rejects_a_certificate_without_names() {
        let (certificate_pem, key_pem) = self_signed_certificate(&["example.com"]);
        let store = CertificateStore::new();

        let error = store
            .install_pem(&[], &certificate_pem, &key_pem)
            .expect_err("names are required");

        assert!(matches!(error, CertificateError::NoNames), "{error:?}");
    }

    #[test]
    fn rejects_a_certificate_that_does_not_cover_every_name() {
        let (certificate_pem, key_pem) = self_signed_certificate(&["old.example.com"]);
        let store = CertificateStore::new();

        let error = store
            .install_pem(&name_list(&["new.example.com"]), &certificate_pem, &key_pem)
            .expect_err("certificate for other names is rejected");

        assert!(
            matches!(&error, CertificateError::MissingName(name) if name == "new.example.com"),
            "{error:?}"
        );
        assert!(store.certified_key("new.example.com").is_none());
    }
}
