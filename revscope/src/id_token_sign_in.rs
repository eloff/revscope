//! Logging in and signing up with a Google or Apple ID token.

use std::sync::Arc;

use serde::Deserialize;

use crate::auth::{
    AuthError, AuthSuccess, MIN_USER_NAME_LENGTH, NewAccountRequest, NewLogin, NewUser,
    create_account, issue_session, map_query_error, normalize_email,
};
use crate::db::open_main_database_read;
use crate::id_token::{Identity, MAX_TOKEN_LEN, SignInError, Verifier};
use crate::server_context::ServerContext;

/// The user may have a password, an invite, or another provider's sign-in.
const EMAIL_HAS_ACCOUNT: &str = "An account with this email already exists. Log in the way you \
     signed up, or use Forgot password to set a password.";

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
    /// Whether this made a new account rather than finding one.
    pub(crate) signed_up: bool,
}

/// Signs in the user the token's identity signed up as, or, when `signup`
/// asks, makes them a new account.
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
    let linked = conn
        .query_opt(
            "SELECT user_id::text FROM user_identities WHERE provider = $1 AND subject = $2",
            &[&provider.id, &identity.subject],
        )
        .await
        .map_err(map_query_error)?;
    if let Some(row) = linked {
        drop(conn);
        return logged_in(server_context, &row.get::<_, String>(0)).await;
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
    // Never linked to an existing user by email: nothing proves a password
    // signup or an invite owns its address, so whoever made that user first
    // would share the account with the provider's verified owner.
    let existing = conn
        .query_opt("SELECT 1 FROM users WHERE email = $1", &[&email.normalized])
        .await
        .map_err(map_query_error)?;
    drop(conn);
    if existing.is_some() {
        return Err(AuthError::Conflict(EMAIL_HAS_ACCOUNT));
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
        },
    };
    Ok(SignedIn {
        success: create_account(server_context, account, user).await?,
        signed_up: true,
    })
}

async fn logged_in(
    server_context: Arc<ServerContext>,
    user_id: &str,
) -> Result<SignedIn, AuthError> {
    Ok(SignedIn {
        success: issue_session(server_context, user_id).await?,
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
