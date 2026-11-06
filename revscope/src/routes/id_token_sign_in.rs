//! Logging in or signing up with a Google or Apple ID token.

use crate::auth::AuthError;
use crate::id_token_sign_in::{self, IdTokenRequest};
use crate::rate_limit::ip_key;
use crate::util::block_on;

use super::account::{check_ip, session_auth_response};
use super::request::{ApiRequest, RouteResult};

pub(super) fn id_token_sign_in(request: &ApiRequest) -> RouteResult {
    check_ip(request, |limits| &limits.login_ip)?;
    let outcome = match serde_json::from_slice::<IdTokenRequest>(request.body) {
        Ok(payload) => {
            let may_sign_up = payload.signup.is_some();
            if may_sign_up {
                check_ip(request, |limits| &limits.signup_ip)?;
            }
            let outcome = block_on(id_token_sign_in::sign_in(request.context(), payload));
            // As with password signup, a mistake fixed in the form doesn't use
            // up a signup, and here neither does a sign-in that found its
            // account. Other failures may have written and rolled back a whole
            // account.
            let used_a_signup = match &outcome {
                Ok(signed_in) => signed_in.signed_up,
                Err(error) => !crate::auth::is_fixable_signup_error(error),
            };
            if let (true, false, Some(ip)) = (may_sign_up, used_a_signup, request.client_ip) {
                request
                    .context()
                    .auth_rate_limits
                    .signup_ip
                    .refund(&ip_key(ip));
            }
            outcome.map(|signed_in| signed_in.success)
        }
        Err(_) => Err(AuthError::InvalidRequest("invalid sign-in payload")),
    };
    Ok(session_auth_response(
        request.context(),
        outcome,
        request.parsed.host,
        false,
    ))
}
