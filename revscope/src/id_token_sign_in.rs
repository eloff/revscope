//! Logging in and signing up with a Google or Apple ID token.

use std::sync::Arc;

use serde::Deserialize;

use crate::auth::{
    AuthError, AuthSuccess, MIN_USER_NAME_LENGTH, NewAccountRequest, NewLogin, NewUser,
    create_account, issue_session, link_identity, map_query_error, normalize_email,
    record_email_proof,
};
use crate::db::{open_main_database_read, open_main_database_write};
use crate::id_token::{Identity, MAX_TOKEN_LEN, SignInError, Verifier};
use crate::server_context::ServerContext;

/// The user has a password or an invite, but hasn't proved they read this address.
const EMAIL_NOT_PROVEN: &str = "An account with this email already exists. To connect this \
     sign-in, use Forgot password to set a new password, then try again. That confirms the email \
     is yours and signs out your other devices and connected tools.";

const EMAIL_HAS_ACCOUNT: &str =
    "An account with this email already exists. Log in with this sign-in instead.";

const EMAIL_HAS_OTHER_LOGIN: &str = "An account with this email already uses a different login \
     with this provider. Log in with that one, or use Forgot password.";

/// Longest name we take from the browser; Apple's full names are far shorter.
const MAX_NAME_LEN: usize = 200;

#[derive(Debug, Deserialize)]
pub(crate) struct IdTokenRequest {
    /// `google` or `apple`.
    pub(crate) provider: String,
    pub(crate) id_token: String,
    /// Apple puts no name in its tokens and tells the browser only on the first
    /// sign-in, so the browser passes it along.
    #[serde(default)]
    pub(crate) name: Option<String>,
    /// Present on the signup pages: create an account if this sign-in has none.
    /// Without it, an unknown sign-in is refused rather than making an account
    /// nobody chose a plan for.
    #[serde(default)]
    pub(crate) signup: Option<NewAccountRequest>,
}

pub(crate) struct SignedIn {
    pub(crate) success: AuthSuccess,
    /// The provider's id, `google` or `apple`.
    pub(crate) provider: &'static str,
    /// Whether this made a new account rather than finding one.
    pub(crate) signed_up: bool,
}

/// Signs in the user the token's identity belongs to, linking it to the user
/// with its email if that user has proved the address and the provider hosts
/// it, or, when `signup` asks, makes them a new account.
pub(crate) async fn sign_in(
    server_context: Arc<ServerContext>,
    payload: IdTokenRequest,
) -> Result<SignedIn, AuthError> {
    let verifier = server_context
        .sign_in_verifier(payload.provider.as_str())
        .ok_or(AuthError::InvalidRequest(
            "that sign-in method isn't available",
        ))?;
    let provider = verifier.provider;
    let identity = verify(&server_context, verifier, payload.id_token.as_str()).await?;

    let conn = open_main_database_read(server_context.clone()).await?;
    if let Some(user_id) = linked_user(&conn, provider.id, identity.subject.as_str()).await? {
        drop(conn);
        // Users who signed in before proof was recorded, or whose Google
        // account has since joined Workspace, gain it as they log in.
        if identity.provider_hosts_email
            && let Some(email) = identity.email.as_deref()
        {
            let email = normalize_email(email)?;
            let conn = open_main_database_write(server_context.clone()).await?;
            record_email_proof(&conn, &user_id, &email.normalized).await?;
        }
        return logged_in(server_context, provider.id, &user_id).await;
    }

    let email = identity
        .email
        .as_deref()
        .ok_or(AuthError::InvalidRequest(provider.no_email))
        .and_then(normalize_email)?;
    // An unchecked address would take the email away from its real owner, and
    // checked first so it can't ask whether someone else's address has a user.
    if !identity.email_verified {
        return Err(AuthError::InvalidRequest(provider.unverified_email));
    }
    // Linked by email only when both sides vouch for the address now: the user
    // proved it with a reset link or a provider that hosts it, and so does this
    // sign-in. Nothing proves a password signup or an invite owns its address,
    // so whoever made that user first would share the account with its owner.
    let existing = conn
        .query_opt(
            "SELECT u.id::text,
                    u.email_verified_at IS NOT NULL,
                    EXISTS (
                        SELECT 1 FROM user_identities i
                        WHERE i.user_id = u.id AND i.provider = $2
                    )
             FROM users u
             WHERE u.email = $1",
            &[&email.normalized, &provider.id],
        )
        .await
        .map_err(map_query_error)?;
    drop(conn);
    if let Some(row) = existing {
        // These two first, since a reset, which the next one asks for, can't help.
        if !identity.provider_hosts_email {
            return Err(AuthError::Conflict(provider.email_not_hosted));
        }
        // One person's login at a provider is the one already linked; another
        // subject claiming the address is someone else, or a mailbox reassigned.
        if row.get::<_, bool>(2) {
            return Err(AuthError::Conflict(EMAIL_HAS_OTHER_LOGIN));
        }
        if !row.get::<_, bool>(1) {
            return Err(AuthError::Conflict(EMAIL_NOT_PROVEN));
        }
        // The signup page would otherwise count this as a new customer and
        // start a checkout for a plan against the existing account.
        if payload.signup.is_some() {
            return Err(AuthError::Conflict(EMAIL_HAS_ACCOUNT));
        }
        let user_id: String = row.get(0);
        let conn = open_main_database_write(server_context.clone()).await?;
        if link_identity(&conn, provider.id, identity.subject.as_str(), &user_id).await? {
            drop(conn);
            return logged_in(server_context, provider.id, &user_id).await;
        }
        // A double submit of this same sign-in got there first.
        let raced = linked_user(&conn, provider.id, identity.subject.as_str()).await?;
        drop(conn);
        return match raced {
            Some(user_id) => logged_in(server_context, provider.id, &user_id).await,
            None => Err(AuthError::Conflict(provider.already_linked)),
        };
    }

    let Some(account) = payload.signup.as_ref() else {
        return Err(AuthError::NotFound(provider.no_account));
    };
    let browser_name = payload
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty() && name.len() <= MAX_NAME_LEN);
    let name = user_name(identity.name.as_deref().or(browser_name), &email.normalized);
    #[cfg(not(feature = "self-hosted"))]
    crate::hosted_signup::validate_account(account)?;
    let user = NewUser {
        name: name.as_str(),
        email,
        login: NewLogin::Identity {
            provider,
            subject: identity.subject.as_str(),
            provider_hosts_email: identity.provider_hosts_email,
        },
    };
    Ok(SignedIn {
        success: create_account(server_context, account, user).await?,
        provider: provider.id,
        signed_up: true,
    })
}

async fn linked_user(
    conn: &crate::db::main::Connection,
    provider: &str,
    subject: &str,
) -> Result<Option<String>, AuthError> {
    let row = conn
        .query_opt(
            "SELECT user_id::text FROM user_identities WHERE provider = $1 AND subject = $2",
            &[&provider, &subject],
        )
        .await
        .map_err(map_query_error)?;
    Ok(row.map(|row| row.get(0)))
}

async fn logged_in(
    server_context: Arc<ServerContext>,
    provider: &'static str,
    user_id: &str,
) -> Result<SignedIn, AuthError> {
    Ok(SignedIn {
        success: issue_session(server_context, user_id).await?,
        provider,
        signed_up: false,
    })
}

async fn verify(
    server_context: &ServerContext,
    verifier: &Verifier,
    token: &str,
) -> Result<Identity, AuthError> {
    let provider = verifier.provider;
    if token.len() > MAX_TOKEN_LEN {
        return Err(AuthError::InvalidRequest(provider.refused));
    }
    verifier
        .verify(&server_context.http, token)
        .await
        .map_err(|error| match error {
            SignInError::Refused(why) => {
                log::info!("{} sign-in refused: {why}", provider.id);
                AuthError::InvalidRequest(provider.refused)
            }
            SignInError::Failed(why) => {
                log::warn!("{} sign-in failed: {why}", provider.id);
                AuthError::Temporary(provider.unreachable)
            }
        })
}

/// The name a new user goes by: the one the provider gave, or else their
/// email's local part, or the whole email when that is too short for a name.
fn user_name(given: Option<&str>, email: &str) -> String {
    let given = given.map(str::trim).unwrap_or_default();
    if given.chars().count() >= MIN_USER_NAME_LENGTH {
        return given.to_owned();
    }
    let local = email.split('@').next().unwrap_or_default();
    if local.chars().count() >= MIN_USER_NAME_LENGTH {
        local.to_owned()
    } else {
        email.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_user_is_named_by_the_provider_or_their_email() {
        assert_eq!(
            user_name(Some(" Ada Lovelace "), "ada@x.com"),
            "Ada Lovelace"
        );
        assert_eq!(user_name(None, "ada@x.com"), "ada");
        assert_eq!(user_name(Some("A"), "ada@x.com"), "ada");
        assert_eq!(user_name(Some(""), "a@x.com"), "a@x.com");
    }
}
