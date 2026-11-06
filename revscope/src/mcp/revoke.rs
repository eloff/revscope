//! `POST /v2/session/mcp/revoke` — deletes every MCP access token on the
//! signed-in admin's account.
//!
//! There is no token list in settings: each consent mints a new year-long
//! token and the plaintext is never shown again. The only user-facing revoke
//! is this, which drops every `kind = 'mcp'` row for the account. The process
//! that handles the request also forgets those tokens in its cache; other
//! processes may keep serving a just-revoked token for up to five minutes.

use std::sync::Arc;

use serde::Serialize;

use crate::auth::AuthError;
use crate::authorize::AdminAccount;
use crate::mcp::token;
use crate::server_context::ServerContext;
use crate::util::block_on;

#[derive(Serialize)]
pub(crate) struct RevokeResponse {
    /// How many rows were deleted. Zero is success: there was nothing to revoke.
    pub(crate) revoked: u64,
}

pub(crate) fn revoke(
    server_context: Arc<ServerContext>,
    admin: &AdminAccount,
) -> Result<RevokeResponse, AuthError> {
    let user = admin.user();
    let revoked = block_on(token::revoke_account_tokens(
        server_context,
        &user.account_id,
    ))?;
    Ok(RevokeResponse { revoked })
}
