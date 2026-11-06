//! `POST /v2/public/mcp/register` — dynamic client registration (RFC 7591).
//!
//! Public on purpose: an MCP client that has never spoken to us needs a
//! `client_id` before it can send its user to the consent page, and there is no
//! human in the loop yet. A registration grants nothing on its own; every token
//! still comes from a signed-in user approving one of their projects.

use std::sync::Arc;

use revscope_api::oauth::{RegisterRequest, RegisterResponse};

use crate::auth::AuthError;
use crate::http::Response;
use crate::mcp::{MCP_SCOPE, clients, invalid_request, oauth_error, oauth_json};
use crate::routes::map_auth_error;
use crate::server_context::ServerContext;
use crate::util::{block_on, seconds_since_epoch};

pub(crate) fn register(server_context: Arc<ServerContext>, body: &[u8]) -> Response {
    let request = match serde_json::from_slice::<RegisterRequest>(body) {
        Ok(request) => request,
        Err(_) => return invalid_request("the registration body must be a JSON object"),
    };
    if request.grant_types.as_ref().is_some_and(|grant_types| {
        !grant_types
            .iter()
            .any(|grant| grant == "authorization_code")
    }) {
        return oauth_error(
            "400 Bad Request",
            "invalid_client_metadata",
            "only the authorization_code grant is supported",
        );
    }
    if request
        .token_endpoint_auth_method
        .as_deref()
        .is_some_and(|method| method != "none")
    {
        return oauth_error(
            "400 Bad Request",
            "invalid_client_metadata",
            "clients register as public clients, so token_endpoint_auth_method must be none",
        );
    }

    match block_on(clients::register_client(
        server_context,
        request.client_name.as_deref().unwrap_or_default(),
        &request.redirect_uris,
    )) {
        Ok(client) => oauth_json(
            "201 Created",
            RegisterResponse {
                client_id: client.client_id,
                client_id_issued_at: i64::from(seconds_since_epoch()),
                client_name: client.client_name,
                redirect_uris: client.redirect_uris,
                grant_types: vec!["authorization_code".to_string()],
                response_types: vec!["code".to_string()],
                token_endpoint_auth_method: "none".to_string(),
                scope: MCP_SCOPE.to_string(),
            },
        ),
        Err(AuthError::InvalidRequest(message)) => crate::mcp::oauth_error(
            "400 Bad Request",
            "invalid_redirect_uri",
            message.to_string(),
        ),
        Err(error) => map_auth_error(error, "mcp_register"),
    }
}
