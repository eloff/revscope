//! Routes behind the dashboard's Ask tab.
//!
//! `POST /v2/chat/projects/{id}` answers one question as a stream of server-sent events;
//! the rest read and delete stored transcripts. The router authorizes every
//! route into a [`ChatGrant`] first: own-project Ask requires a dashboard
//! session, and the public demo also accepts anonymous callers, identified and
//! rate-limited by client IP (shared history on a shared IP is intentional).
//! Anonymous demo turns also require a short-lived challenge token minted after
//! the visitor interacts with the Ask UI, so a bare `curl` cannot spend tokens.

use std::net::IpAddr;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::mpsc;

use crate::auth::AuthError;
use crate::authorize::{ChatCaller, ChatGrant};
use crate::chat_agent::{self, ChatBilling, ChatTurn};
use crate::chat_config::ChatConfig;
use crate::chat_store::{self, ChatStorage, MAX_MESSAGE_CHARS, QuotaError};
use crate::http::Response;
use crate::query_params::query_pairs;
use crate::routes::map_auth_error;
use crate::server_context::ServerContext;
use crate::tokens::DEMO_CHAT_CHALLENGE_TTL_SECONDS;

/// Events buffered between the agent and the socket. Enough that a burst of
/// short text deltas never blocks the agent, small enough that a client which
/// has stopped reading cannot pin much memory.
const EVENT_BUFFER: usize = 64;

#[derive(Deserialize)]
pub(crate) struct ChatRequest {
    /// Absent starts a new conversation.
    conversation_id: Option<i64>,
    message: String,
    /// Required for anonymous demo Ask; ignored when a session is present.
    challenge_token: Option<String>,
}

impl ChatCaller {
    fn owner_key(&self) -> String {
        match self {
            Self::SignedIn(user) => user.user_id.clone(),
            Self::DemoGuest { client_ip } => client_ip.to_string(),
        }
    }

    fn client_ip(&self, request_ip: Option<IpAddr>) -> Option<IpAddr> {
        match self {
            Self::DemoGuest { client_ip } => Some(*client_ip),
            Self::SignedIn(_) => request_ip,
        }
    }

    fn billing(&self) -> ChatBilling {
        match self {
            Self::SignedIn(user) => ChatBilling::Account {
                account_id: user.account_id.clone(),
                user_id: user.user_id.clone(),
            },
            Self::DemoGuest { client_ip } => ChatBilling::DemoIp {
                client_ip: *client_ip,
            },
        }
    }
}

#[derive(Serialize)]
pub(crate) struct DemoChatChallengeResponse {
    token: String,
    /// Seconds since the unix epoch.
    expires_at: u32,
}

/// Mints a short-lived token bound to the caller's IP. The Ask UI requests this
/// after a real interaction (focus, click, or keypress) so anonymous `POST
/// /v2/chat/projects/{id}` cannot be driven by a naive scraper that never loads the page.
pub(crate) fn create_demo_chat_challenge(
    server_context: Arc<ServerContext>,
    client_ip: Option<IpAddr>,
) -> Result<DemoChatChallengeResponse, AuthError> {
    let Some(client_ip) = client_ip else {
        return Err(AuthError::InvalidRequest(
            "a client IP is required for a demo Ask challenge",
        ));
    };
    let issued = server_context
        .demo_chat_challenges
        .issue(client_ip, DEMO_CHAT_CHALLENGE_TTL_SECONDS);
    Ok(DemoChatChallengeResponse {
        token: issued.token,
        expires_at: issued.expires_at,
    })
}

/// Anonymous demo Ask must present a live challenge minted for this IP.
fn require_demo_challenge(
    server_context: &ServerContext,
    client_ip: IpAddr,
    challenge_token: Option<&str>,
) -> Result<(), AuthError> {
    let Some(raw) = challenge_token
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Err(AuthError::InvalidRequest(
            "a challenge token is required for anonymous demo Ask",
        ));
    };
    if !server_context.demo_chat_challenges.verify(raw, client_ip) {
        return Err(AuthError::InvalidSession);
    }
    Ok(())
}

fn check_caller_quota(
    server_context: Arc<ServerContext>,
    caller: &ChatCaller,
) -> Result<(), QuotaError> {
    match caller {
        ChatCaller::SignedIn(user) => {
            chat_store::check_quota(server_context, &user.account_id, &user.user_id)
        }
        ChatCaller::DemoGuest { client_ip } => {
            chat_store::check_demo_guest_quota(server_context, *client_ip)
        }
    }
}

/// Starts a turn: authorize, check the budget, record the question, then hand
/// the socket a receiver the agent task writes into.
pub(crate) fn chat(
    server_context: Arc<ServerContext>,
    grant: &ChatGrant,
    client_ip: Option<IpAddr>,
    request: ChatRequest,
) -> Response {
    let message = request.message.trim();
    if message.is_empty() {
        return bad_request("message is required");
    }
    if message.chars().count() > MAX_MESSAGE_CHARS {
        return bad_request("message is too long");
    }

    let caller = grant.caller();
    let project_id = grant.project().id();
    // Before the gateway config check, like the router's authorization, so an
    // anonymous caller without a challenge learns nothing about the config.
    if let ChatCaller::DemoGuest { client_ip } = caller
        && let Err(error) = require_demo_challenge(
            server_context.as_ref(),
            *client_ip,
            request.challenge_token.as_deref(),
        )
    {
        return map_auth_error(error, "chat challenge");
    }

    let config = match ChatConfig::from_env() {
        Ok(config) => config,
        Err(error) => {
            return Response::json(
                "503 Service Unavailable",
                json!({ "error": "the assistant is not configured on this server" }).to_string(),
            )
            .with_internal_error(error.to_string());
        }
    };

    match check_caller_quota(server_context.clone(), caller) {
        Ok(()) => {}
        Err(QuotaError::Exceeded(message)) => {
            return Response::json(
                "429 Too Many Requests",
                json!({ "error": message }).to_string(),
            );
        }
        Err(QuotaError::Db(error)) => return map_auth_error(error, "chat quota"),
    }

    let storage = ChatStorage::for_project(project_id);
    let owner_key = caller.owner_key();
    let conversation_id = match chat_store::open_conversation(
        server_context.clone(),
        storage,
        project_id,
        &owner_key,
        caller.client_ip(client_ip),
        request.conversation_id,
        message,
    ) {
        Ok(id) => id,
        Err(error) => return map_auth_error(error, "chat conversation"),
    };
    if let Err(error) = chat_store::append_message(
        server_context.clone(),
        storage,
        project_id,
        conversation_id,
        chat_store::NewMessage::text("user", message),
    ) {
        return map_auth_error(error, "chat message");
    }

    let (sender, receiver) = mpsc::channel(EVENT_BUFFER);
    // Announced before the model is even called so a new conversation can be
    // selected in the rail while its first answer is still streaming. A header
    // would need an extra CORS exposure rule to be readable cross-origin.
    let _ = sender.try_send(
        format!(
            "event: conversation\ndata: {}\n\n",
            json!({ "conversation_id": conversation_id })
        )
        .into_bytes(),
    );
    let turn = ChatTurn {
        server_context: server_context.clone(),
        config,
        project_id: project_id.to_string(),
        conversation_id,
        owner_key,
        storage,
        billing: caller.billing(),
    };
    crate::task::spawn(server_context, async move {
        chat_agent::run_turn(turn, sender).await;
    });

    Response::event_stream(receiver)
}

pub(crate) fn conversations(
    server_context: Arc<ServerContext>,
    grant: &ChatGrant,
) -> Result<chat_store::ConversationsResponse, AuthError> {
    let project_id = grant.project().id();
    chat_store::list_conversations(
        server_context,
        ChatStorage::for_project(project_id),
        project_id,
        &grant.caller().owner_key(),
    )
}

pub(crate) fn messages(
    server_context: Arc<ServerContext>,
    grant: &ChatGrant,
    conversation_id: i64,
) -> Result<chat_store::MessagesResponse, AuthError> {
    let project_id = grant.project().id();
    chat_store::load_messages(
        server_context,
        ChatStorage::for_project(project_id),
        project_id,
        &grant.caller().owner_key(),
        conversation_id,
    )
}

pub(crate) fn delete_conversation(
    server_context: Arc<ServerContext>,
    grant: &ChatGrant,
    conversation_id: i64,
) -> Result<(), AuthError> {
    let project_id = grant.project().id();
    chat_store::delete_conversation(
        server_context,
        ChatStorage::for_project(project_id),
        project_id,
        &grant.caller().owner_key(),
        conversation_id,
    )
}

/// `conversation_id` from the query string, which every read route needs.
pub(crate) fn conversation_id_from_query(target: &str) -> Result<i64, &'static str> {
    let (_key, value) = query_pairs(target)
        .find(|(key, _value)| *key == "conversation_id")
        .ok_or("conversation_id is required")?;
    value
        .parse::<i64>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or("invalid conversation_id")
}

fn bad_request(message: &str) -> Response {
    Response::json("400 Bad Request", json!({ "error": message }).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_conversation_id_from_the_query_string() {
        assert_eq!(
            conversation_id_from_query("/v2/chat/projects/1/messages?conversation_id=42"),
            Ok(42)
        );
    }

    #[test]
    fn rejects_a_missing_or_unusable_conversation_id() {
        assert!(conversation_id_from_query("/v2/chat/projects/1/messages").is_err());
        assert!(
            conversation_id_from_query("/v2/chat/projects/1/messages?conversation_id=0").is_err()
        );
        assert!(
            conversation_id_from_query("/v2/chat/projects/1/messages?conversation_id=abc").is_err()
        );
    }

    #[test]
    fn a_demo_challenge_is_bound_to_the_ip_it_was_minted_for() {
        let server_context = Arc::new(ServerContext::new_in_memory("chat-challenge"));
        let ip: IpAddr = "203.0.113.10".parse().expect("ip");
        let other: IpAddr = "203.0.113.20".parse().expect("ip");
        let minted = create_demo_chat_challenge(server_context.clone(), Some(ip)).expect("mint");

        require_demo_challenge(server_context.as_ref(), ip, Some(&minted.token))
            .expect("matching IP");
        assert!(
            require_demo_challenge(server_context.as_ref(), other, Some(&minted.token)).is_err()
        );
        assert!(require_demo_challenge(server_context.as_ref(), ip, None).is_err());
        assert!(require_demo_challenge(server_context.as_ref(), ip, Some("not-a-token")).is_err());
    }
}
