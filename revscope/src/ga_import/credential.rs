//! A Google credential as an import holds it, and as it's stored encrypted.

use crate::crypto;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum GaCredential {
    ServiceAccount {
        client_email: String,
        private_key: String,
    },
    OAuth {
        refresh_token: String,
    },
}

impl GaCredential {
    /// The `encrypted_client_email` and `encrypted_secret` columns.
    pub(super) fn encrypt(&self) -> (Option<Vec<u8>>, Vec<u8>) {
        match self {
            Self::ServiceAccount {
                client_email,
                private_key,
            } => (
                Some(crypto::encrypt(client_email.as_bytes())),
                crypto::encrypt(private_key.as_bytes()),
            ),
            Self::OAuth { refresh_token } => (None, crypto::encrypt(refresh_token.as_bytes())),
        }
    }

    pub(super) fn decrypt(
        encrypted_client_email: Option<&[u8]>,
        encrypted_secret: &[u8],
    ) -> Option<Self> {
        let decrypt_text = |stored: &[u8]| String::from_utf8(crypto::decrypt(stored)?).ok();
        let secret = decrypt_text(encrypted_secret)?;
        Some(match encrypted_client_email {
            Some(email) => Self::ServiceAccount {
                client_email: decrypt_text(email)?,
                private_key: secret,
            },
            None => Self::OAuth {
                refresh_token: secret,
            },
        })
    }

    /// Reads a downloaded service account key. Only these two fields are used:
    /// `token_uri` is deliberately ignored, since following a URL from pasted
    /// JSON would have the server post a signed assertion wherever it points.
    pub(super) fn from_service_account_json(json: &str) -> Option<Self> {
        let key: serde_json::Value = serde_json::from_str(json).ok()?;
        let text = |field: &str| {
            key.get(field)
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };
        Some(Self::ServiceAccount {
            client_email: text("client_email")?,
            private_key: text("private_key")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_service_account_key_needs_an_email_and_a_private_key() {
        let credential = GaCredential::from_service_account_json(
            r#"{"type":"service_account","client_email":"a@b.iam","private_key":"pem","token_uri":"https://evil.test"}"#,
        );
        assert_eq!(
            credential,
            Some(GaCredential::ServiceAccount {
                client_email: "a@b.iam".to_string(),
                private_key: "pem".to_string(),
            })
        );
        assert_eq!(
            GaCredential::from_service_account_json(r#"{"client_email":"a@b.iam"}"#),
            None
        );
        assert_eq!(GaCredential::from_service_account_json("not json"), None);
    }

    #[test]
    fn both_kinds_survive_the_stored_form() {
        for credential in [
            GaCredential::ServiceAccount {
                client_email: "a@b.iam".to_string(),
                private_key: "pem".to_string(),
            },
            GaCredential::OAuth {
                refresh_token: "token".to_string(),
            },
        ] {
            let (email, secret) = credential.encrypt();
            assert_eq!(
                GaCredential::decrypt(email.as_deref(), &secret),
                Some(credential)
            );
        }
    }
}
