//! Ask: chatting with an agent about a project's data, and the conversations
//! it keeps. Owners chat with a session; the public demo also takes anonymous
//! callers, rate-limited by client IP.

use serde_json::json;

use crate::authorize::ChatGrant;
use crate::chat::{self, ChatRequest};
use crate::chat_config::ChatConfig;
use crate::http::Response;

use super::request::{ApiRequest, RouteResult};
use super::responses::{bad_request, map_auth_error, map_project_auth_error};

pub(super) fn chat_status(_request: &ApiRequest) -> RouteResult {
    // Deliberately unauthenticated and free of project detail: the dashboard
    // only needs to know whether to offer the tab at all.
    Ok(Response::ok_json(
        json!({ "available": ChatConfig::is_configured() }),
    ))
}

pub(super) fn demo_chat_token(request: &ApiRequest) -> RouteResult {
    // Public on purpose: the Ask UI mints this after a visitor interacts with
    // the composer. The token is bound to the client IP and only unlocks
    // anonymous demo Ask for a short window.
    chat::create_demo_chat_challenge(request.context(), request.client_ip)
        .map(Response::ok_json)
        .map_err(|error| map_auth_error(error, "demo_chat_token"))
}

pub(super) fn chat(request: &ApiRequest, grant: &ChatGrant) -> RouteResult {
    // Query tokens are deliberately not accepted here: a turn spends money.
    let payload = request.json_body::<ChatRequest>("invalid chat payload")?;
    Ok(chat::chat(
        request.context(),
        grant,
        request.client_ip,
        payload,
    ))
}

pub(super) fn chat_conversations(request: &ApiRequest, grant: &ChatGrant) -> RouteResult {
    chat::conversations(request.context(), grant)
        .map(Response::ok_json)
        .map_err(|error| map_project_auth_error(error, "chat_conversations"))
}

pub(super) fn delete_chat_conversation(request: &ApiRequest, grant: &ChatGrant) -> RouteResult {
    let conversation_id =
        chat::conversation_id_from_query(request.target()).map_err(bad_request)?;
    chat::delete_conversation(request.context(), grant, conversation_id)
        .map_err(|error| map_project_auth_error(error, "chat_conversations"))?;
    Ok(Response::ok_json(json!({ "ok": true })))
}

pub(super) fn chat_messages(request: &ApiRequest, grant: &ChatGrant) -> RouteResult {
    let conversation_id =
        chat::conversation_id_from_query(request.target()).map_err(bad_request)?;
    chat::messages(request.context(), grant, conversation_id)
        .map(Response::ok_json)
        .map_err(|error| map_project_auth_error(error, "chat_messages"))
}
