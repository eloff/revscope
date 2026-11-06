//! The OAuth endpoints an MCP client uses to get a token for `/v2/mcp`.

use crate::authorize::{Account, AdminAccount};
use crate::mcp;
use crate::rate_limit::ip_key;

use super::request::{ApiRequest, RouteResult};
use super::responses::too_many_requests;

/// Dynamic client registration, before any user is involved. Each one is a row
/// in the main database, so it is limited per address. A request with no known
/// address (only in tests that bypass the listener) skips the limit.
pub(super) fn register(request: &ApiRequest) -> RouteResult {
    if let Some(ip) = request.client_ip {
        request
            .context()
            .public_write_rate_limits
            .mcp_register_ip
            .check(&ip_key(ip))
            .map_err(too_many_requests)?;
    }
    Ok(mcp::register::register(request.context(), request.body))
}

/// Redeems an authorization code for a token, after the user approved it.
pub(super) fn token(request: &ApiRequest) -> RouteResult {
    Ok(mcp::grant::grant(request.context(), request.body))
}

/// Describes the client asking for access, for the consent page. Behind a
/// verified session so the page never answers a session id that was never
/// issued.
pub(super) fn describe_client(request: &ApiRequest, _account: &Account) -> RouteResult {
    Ok(mcp::authorize::describe_client(
        request.context(),
        request.target(),
    ))
}

/// Records the user's answer on the consent page.
pub(super) fn authorize(request: &ApiRequest, account: &Account) -> RouteResult {
    Ok(mcp::authorize::authorize(
        request.context(),
        account,
        request.body,
    ))
}

pub(super) fn revoke(request: &ApiRequest, admin: &AdminAccount) -> RouteResult {
    let outcome = mcp::revoke::revoke(request.context(), admin);
    request.session_json(admin, outcome, "mcp_revoke")
}
