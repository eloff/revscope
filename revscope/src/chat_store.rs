//! Transcript storage for the Ask tab, plus its usage limits.
//!
//! Conversations live in the project database because that is where the data
//! being discussed lives; usage lives in the main database because the limits
//! span an account's projects. The public demo is the exception: its transcripts
//! and anonymous spend live in the main database so they survive demo resets
//! and stay out of the demo database that signed-in accounts can download.
//!
//! A signed-in account's Ask spend draws on its plan's monthly allowance (see
//! `usage_budget.rs`). Anonymous demo Ask is capped at a flat lifetime token
//! budget per client IP.

use std::net::IpAddr;
use std::sync::Arc;

use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use tokio_postgres::types::ToSql;

use crate::auth::{AuthError, map_row_error};
use crate::db::{
    ProjectConnection, open_main_database_read, open_main_database_write,
    open_project_database_read, open_project_database_write, parse_project_id,
};
use crate::download_db::DEMO_PROJECT_ID;
use crate::server_context::ServerContext;
use crate::usage_budget::{self, Allowance};
use crate::util::{block_on, seconds_since_epoch};

/// Turns one user may start per hour. Sized so a working session never notices
/// it while a runaway script does.
const MAX_TURNS_PER_HOUR: i64 = 60;
/// Lifetime token ceiling for anonymous Ask on the public demo, per client IP.
pub(crate) const DEMO_GUEST_TOKEN_BUDGET: i64 = 100_000;
/// Longest question accepted. Long pastes belong in the data, not the prompt.
pub(crate) const MAX_MESSAGE_CHARS: usize = 16_384;
/// A conversation that does not exist, or is not the caller's: the same answer
/// either way, so another user's conversation ids cannot be probed.
const CONVERSATION_NOT_FOUND: AuthError = AuthError::NotFound("conversation not found");
/// Conversation titles are the opening question, clipped to fit the rail.
const MAX_TITLE_CHARS: usize = 80;

/// Where a conversation's rows live.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChatStorage {
    /// Project SQLite (normal accounts).
    Project,
    /// Main Postgres `demo_chat_*` tables.
    Demo,
}

impl ChatStorage {
    pub(crate) fn for_project(project_id: &str) -> Self {
        if project_id == DEMO_PROJECT_ID {
            Self::Demo
        } else {
            Self::Project
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct ConversationSummary {
    pub(crate) id: i64,
    pub(crate) title: String,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
}

#[derive(Debug, Serialize)]
pub(crate) struct StoredMessage {
    pub(crate) id: i64,
    pub(crate) seq: i64,
    pub(crate) role: String,
    pub(crate) content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tool_sql: Option<String>,
    /// Raw JSON, re-emitted verbatim so the browser redraws the same table it
    /// saw while the answer was streaming.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tool_result: Option<serde_json::Value>,
    pub(crate) created_at: i64,
}

#[derive(Debug, Serialize)]
pub(crate) struct ConversationsResponse {
    pub(crate) conversations: Vec<ConversationSummary>,
}

#[derive(Debug, Serialize)]
pub(crate) struct MessagesResponse {
    pub(crate) conversation_id: i64,
    pub(crate) messages: Vec<StoredMessage>,
}

/// A row to append. `tool_sql` and `tool_result` are only set for tool rows.
pub(crate) struct NewMessage<'a> {
    pub(crate) role: &'a str,
    pub(crate) content: &'a str,
    pub(crate) tool_sql: Option<&'a str>,
    pub(crate) tool_result: Option<&'a str>,
}

impl<'a> NewMessage<'a> {
    pub(crate) fn text(role: &'a str, content: &'a str) -> Self {
        Self {
            role,
            content,
            tool_sql: None,
            tool_result: None,
        }
    }
}

/// Starts a conversation, or confirms the caller owns the one they named.
/// Reusing another user's conversation id must not be possible, so ownership is
/// part of the lookup rather than a separate check.
pub(crate) fn open_conversation(
    server_context: Arc<ServerContext>,
    storage: ChatStorage,
    project_id: &str,
    owner_key: &str,
    client_ip: Option<IpAddr>,
    conversation_id: Option<i64>,
    first_message: &str,
) -> Result<i64, AuthError> {
    match storage {
        ChatStorage::Project => open_project_conversation(
            server_context,
            project_id,
            owner_key,
            conversation_id,
            first_message,
        ),
        ChatStorage::Demo => open_demo_conversation(
            server_context,
            owner_key,
            client_ip,
            conversation_id,
            first_message,
        ),
    }
}

fn open_project_conversation(
    server_context: Arc<ServerContext>,
    project_id: &str,
    user_id: &str,
    conversation_id: Option<i64>,
    first_message: &str,
) -> Result<i64, AuthError> {
    let conn = open_project_database_write(server_context.as_ref(), project_id)
        .map_err(|error| AuthError::Db(error.to_string()))?;
    let now = seconds_since_epoch();

    if let Some(id) = conversation_id {
        let owned: Option<i64> = conn
            .query_row(
                "SELECT id FROM chat_conversations WHERE id = ?1 AND user_id = ?2",
                params![id, user_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(map_row_error)?;
        let id = owned.ok_or(CONVERSATION_NOT_FOUND)?;
        conn.execute(
            "UPDATE chat_conversations SET updated_at = ?1 WHERE id = ?2",
            params![now, id],
        )
        .map_err(map_row_error)?;
        return Ok(id);
    }

    conn.execute(
        "INSERT INTO chat_conversations (user_id, title, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?3)",
        params![user_id, conversation_title(first_message), now],
    )
    .map_err(map_row_error)?;
    Ok(conn.last_insert_rowid())
}

fn open_demo_conversation(
    server_context: Arc<ServerContext>,
    owner_key: &str,
    client_ip: Option<IpAddr>,
    conversation_id: Option<i64>,
    first_message: &str,
) -> Result<i64, AuthError> {
    let owner_key = owner_key.to_string();
    let title = conversation_title(first_message);
    let ip_text = client_ip.map(|ip| ip.to_string());
    block_on(async move {
        let conn = open_main_database_write(server_context).await?;
        if let Some(id) = conversation_id {
            let row = conn
                .query_opt(
                    "SELECT id FROM demo_chat_conversations
                     WHERE id = $1 AND owner_key = $2",
                    &[
                        &id as &(dyn ToSql + Sync),
                        &owner_key as &(dyn ToSql + Sync),
                    ],
                )
                .await
                .map_err(AuthError::from)?;
            let Some(row) = row else {
                return Err(CONVERSATION_NOT_FOUND);
            };
            let id: i64 = row.get(0);
            conn.execute(
                "UPDATE demo_chat_conversations SET updated_at = NOW() WHERE id = $1",
                &[&id as &(dyn ToSql + Sync)],
            )
            .await
            .map_err(AuthError::from)?;
            return Ok(id);
        }
        let row = conn
            .query_one(
                "INSERT INTO demo_chat_conversations (owner_key, client_ip, title)
                 VALUES ($1, $2::text::inet, $3)
                 RETURNING id",
                &[
                    &owner_key as &(dyn ToSql + Sync),
                    &ip_text as &(dyn ToSql + Sync),
                    &title as &(dyn ToSql + Sync),
                ],
            )
            .await
            .map_err(AuthError::from)?;
        Ok(row.get(0))
    })
}

/// Appends a row at the next sequence number and returns it.
pub(crate) fn append_message(
    server_context: Arc<ServerContext>,
    storage: ChatStorage,
    project_id: &str,
    conversation_id: i64,
    message: NewMessage<'_>,
) -> Result<i64, AuthError> {
    match storage {
        ChatStorage::Project => {
            let conn = open_project_database_write(server_context.as_ref(), project_id)
                .map_err(|error| AuthError::Db(error.to_string()))?;
            append_message_with(&conn, conversation_id, message)
        }
        ChatStorage::Demo => append_demo_message(server_context, conversation_id, message),
    }
}

fn append_demo_message(
    server_context: Arc<ServerContext>,
    conversation_id: i64,
    message: NewMessage<'_>,
) -> Result<i64, AuthError> {
    let role = message.role.to_string();
    let content = message.content.to_string();
    let tool_sql = message.tool_sql.map(str::to_string);
    let tool_result = message.tool_result.map(str::to_string);
    block_on(async move {
        let conn = open_main_database_write(server_context).await?;
        let row = conn
            .query_one(
                "INSERT INTO demo_chat_messages
                     (conversation_id, seq, role, content, tool_sql, tool_result)
                 VALUES (
                     $1,
                     (SELECT COALESCE(MAX(seq), 0) + 1
                      FROM demo_chat_messages WHERE conversation_id = $1),
                     $2, $3, $4, $5
                 )
                 RETURNING id",
                &[
                    &conversation_id as &(dyn ToSql + Sync),
                    &role as &(dyn ToSql + Sync),
                    &content as &(dyn ToSql + Sync),
                    &tool_sql as &(dyn ToSql + Sync),
                    &tool_result as &(dyn ToSql + Sync),
                ],
            )
            .await
            .map_err(AuthError::from)?;
        conn.execute(
            "UPDATE demo_chat_conversations SET updated_at = NOW() WHERE id = $1",
            &[&conversation_id as &(dyn ToSql + Sync)],
        )
        .await
        .map_err(AuthError::from)?;
        Ok(row.get(0))
    })
}

fn append_message_with(
    conn: &ProjectConnection,
    conversation_id: i64,
    message: NewMessage<'_>,
) -> Result<i64, AuthError> {
    let now = seconds_since_epoch();
    conn.execute(
        "INSERT INTO chat_messages
             (conversation_id, seq, role, content, tool_sql, tool_result, created_at)
         VALUES (
             ?1,
             (SELECT COALESCE(MAX(seq), 0) + 1 FROM chat_messages WHERE conversation_id = ?1),
             ?2, ?3, ?4, ?5, ?6
         )",
        params![
            conversation_id,
            message.role,
            message.content,
            message.tool_sql,
            message.tool_result,
            now,
        ],
    )
    .map_err(map_row_error)?;
    conn.execute(
        "UPDATE chat_conversations SET updated_at = ?1 WHERE id = ?2",
        params![now, conversation_id],
    )
    .map_err(map_row_error)?;
    Ok(conn.last_insert_rowid())
}

pub(crate) fn list_conversations(
    server_context: Arc<ServerContext>,
    storage: ChatStorage,
    project_id: &str,
    owner_key: &str,
) -> Result<ConversationsResponse, AuthError> {
    match storage {
        ChatStorage::Project => list_project_conversations(server_context, project_id, owner_key),
        ChatStorage::Demo => list_demo_conversations(server_context, owner_key),
    }
}

fn list_project_conversations(
    server_context: Arc<ServerContext>,
    project_id: &str,
    user_id: &str,
) -> Result<ConversationsResponse, AuthError> {
    let conn = open_project_database_read(server_context.as_ref(), project_id)
        .map_err(|error| AuthError::Db(error.to_string()))?;
    let mut statement = conn
        .prepare(
            "SELECT id, title, created_at, updated_at
             FROM chat_conversations
             WHERE user_id = ?1
             ORDER BY updated_at DESC
             LIMIT 100",
        )
        .map_err(map_row_error)?;
    let rows = statement
        .query_map(params![user_id], |row| {
            Ok(ConversationSummary {
                id: row.get(0)?,
                title: row.get(1)?,
                created_at: row.get(2)?,
                updated_at: row.get(3)?,
            })
        })
        .map_err(map_row_error)?;
    let mut conversations = Vec::new();
    for row in rows {
        conversations.push(row.map_err(map_row_error)?);
    }
    Ok(ConversationsResponse { conversations })
}

fn list_demo_conversations(
    server_context: Arc<ServerContext>,
    owner_key: &str,
) -> Result<ConversationsResponse, AuthError> {
    let owner_key = owner_key.to_string();
    block_on(async move {
        let conn = open_main_database_read(server_context).await?;
        let rows = conn
            .query(
                "SELECT id, title,
                        EXTRACT(EPOCH FROM created_at)::bigint,
                        EXTRACT(EPOCH FROM updated_at)::bigint
                 FROM demo_chat_conversations
                 WHERE owner_key = $1
                 ORDER BY updated_at DESC
                 LIMIT 100",
                &[&owner_key as &(dyn ToSql + Sync)],
            )
            .await
            .map_err(AuthError::from)?;
        let conversations = rows
            .into_iter()
            .map(|row| ConversationSummary {
                id: row.get(0),
                title: row.get(1),
                created_at: row.get(2),
                updated_at: row.get(3),
            })
            .collect();
        Ok(ConversationsResponse { conversations })
    })
}

pub(crate) fn load_messages(
    server_context: Arc<ServerContext>,
    storage: ChatStorage,
    project_id: &str,
    owner_key: &str,
    conversation_id: i64,
) -> Result<MessagesResponse, AuthError> {
    match storage {
        ChatStorage::Project => {
            load_project_messages(server_context, project_id, owner_key, conversation_id)
        }
        ChatStorage::Demo => load_demo_messages(server_context, owner_key, conversation_id),
    }
}

fn load_project_messages(
    server_context: Arc<ServerContext>,
    project_id: &str,
    user_id: &str,
    conversation_id: i64,
) -> Result<MessagesResponse, AuthError> {
    let conn = open_project_database_read(server_context.as_ref(), project_id)
        .map_err(|error| AuthError::Db(error.to_string()))?;
    ensure_conversation_owner(&conn, conversation_id, user_id)?;

    let mut statement = conn
        .prepare(
            "SELECT id, seq, role, content, tool_sql, tool_result, created_at
             FROM chat_messages
             WHERE conversation_id = ?1
             ORDER BY seq",
        )
        .map_err(map_row_error)?;
    let rows = statement
        .query_map(params![conversation_id], |row| {
            let tool_result: Option<String> = row.get(5)?;
            Ok(StoredMessage {
                id: row.get(0)?,
                seq: row.get(1)?,
                role: row.get(2)?,
                content: row.get(3)?,
                tool_sql: row.get(4)?,
                tool_result: tool_result.and_then(|json| serde_json::from_str(&json).ok()),
                created_at: row.get(6)?,
            })
        })
        .map_err(map_row_error)?;
    let mut messages = Vec::new();
    for row in rows {
        messages.push(row.map_err(map_row_error)?);
    }
    Ok(MessagesResponse {
        conversation_id,
        messages,
    })
}

fn load_demo_messages(
    server_context: Arc<ServerContext>,
    owner_key: &str,
    conversation_id: i64,
) -> Result<MessagesResponse, AuthError> {
    let owner_key = owner_key.to_string();
    block_on(async move {
        let conn = open_main_database_read(server_context).await?;
        let owned = conn
            .query_opt(
                "SELECT id FROM demo_chat_conversations
                 WHERE id = $1 AND owner_key = $2",
                &[
                    &conversation_id as &(dyn ToSql + Sync),
                    &owner_key as &(dyn ToSql + Sync),
                ],
            )
            .await
            .map_err(AuthError::from)?;
        if owned.is_none() {
            return Err(CONVERSATION_NOT_FOUND);
        }
        let rows = conn
            .query(
                "SELECT id, seq::bigint, role, content, tool_sql, tool_result,
                        EXTRACT(EPOCH FROM created_at)::bigint
                 FROM demo_chat_messages
                 WHERE conversation_id = $1
                 ORDER BY seq",
                &[&conversation_id as &(dyn ToSql + Sync)],
            )
            .await
            .map_err(AuthError::from)?;
        let messages = rows
            .into_iter()
            .map(|row| {
                let tool_result: Option<String> = row.get(5);
                StoredMessage {
                    id: row.get(0),
                    seq: row.get(1),
                    role: row.get(2),
                    content: row.get(3),
                    tool_sql: row.get(4),
                    tool_result: tool_result.and_then(|json| serde_json::from_str(&json).ok()),
                    created_at: row.get(6),
                }
            })
            .collect();
        Ok(MessagesResponse {
            conversation_id,
            messages,
        })
    })
}

pub(crate) fn delete_conversation(
    server_context: Arc<ServerContext>,
    storage: ChatStorage,
    project_id: &str,
    owner_key: &str,
    conversation_id: i64,
) -> Result<(), AuthError> {
    match storage {
        ChatStorage::Project => {
            let conn = open_project_database_write(server_context.as_ref(), project_id)
                .map_err(|error| AuthError::Db(error.to_string()))?;
            delete_conversation_with(&conn, conversation_id, owner_key)
        }
        ChatStorage::Demo => delete_demo_conversation(server_context, owner_key, conversation_id),
    }
}

fn delete_demo_conversation(
    server_context: Arc<ServerContext>,
    owner_key: &str,
    conversation_id: i64,
) -> Result<(), AuthError> {
    let owner_key = owner_key.to_string();
    block_on(async move {
        let conn = open_main_database_write(server_context).await?;
        let deleted = conn
            .execute(
                "DELETE FROM demo_chat_conversations WHERE id = $1 AND owner_key = $2",
                &[
                    &conversation_id as &(dyn ToSql + Sync),
                    &owner_key as &(dyn ToSql + Sync),
                ],
            )
            .await
            .map_err(AuthError::from)?;
        if deleted == 0 {
            return Err(CONVERSATION_NOT_FOUND);
        }
        Ok(())
    })
}

/// Deleting the conversation is what authorizes the delete, so it goes first;
/// should the second statement fail, the messages left behind are unreachable
/// because every read path checks ownership through the conversation. The
/// messages are removed explicitly rather than by the declared cascade, which
/// only fires when `PRAGMA foreign_keys` is on.
fn delete_conversation_with(
    conn: &ProjectConnection,
    conversation_id: i64,
    user_id: &str,
) -> Result<(), AuthError> {
    let deleted = conn
        .execute(
            "DELETE FROM chat_conversations WHERE id = ?1 AND user_id = ?2",
            params![conversation_id, user_id],
        )
        .map_err(map_row_error)?;
    if deleted == 0 {
        return Err(CONVERSATION_NOT_FOUND);
    }
    conn.execute(
        "DELETE FROM chat_messages WHERE conversation_id = ?1",
        params![conversation_id],
    )
    .map_err(map_row_error)?;
    Ok(())
}

fn ensure_conversation_owner(
    conn: &ProjectConnection,
    conversation_id: i64,
    user_id: &str,
) -> Result<(), AuthError> {
    let owned: Option<i64> = conn
        .query_row(
            "SELECT id FROM chat_conversations WHERE id = ?1 AND user_id = ?2",
            params![conversation_id, user_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(map_row_error)?;
    owned.map(|_| ()).ok_or(CONVERSATION_NOT_FOUND)
}

/// The opening question, clipped on a word boundary where one is near enough.
fn conversation_title(message: &str) -> String {
    let collapsed = message.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= MAX_TITLE_CHARS {
        return collapsed;
    }
    let clipped: String = collapsed.chars().take(MAX_TITLE_CHARS).collect();
    match clipped.rsplit_once(' ') {
        Some((head, _)) if head.chars().count() >= MAX_TITLE_CHARS / 2 => format!("{head}…"),
        _ => format!("{clipped}…"),
    }
}

#[derive(Debug)]
pub(crate) enum QuotaError {
    /// Human-readable reason, shown to the user as a 429.
    Exceeded(String),
    Db(AuthError),
}

/// Refuses a turn that would put the caller over the hourly cap or the
/// account's monthly allowance.
pub(crate) fn check_quota(
    server_context: Arc<ServerContext>,
    account_id: &str,
    user_id: &str,
) -> Result<(), QuotaError> {
    let account_id = account_id.to_string();
    let user_id = user_id.to_string();
    let (allowance, turns_this_hour) = block_on(async move {
        let conn = open_main_database_read(server_context)
            .await
            .map_err(|error| QuotaError::Db(AuthError::Db(error.to_string())))?;
        let params: [&(dyn ToSql + Sync); 2] = [&account_id, &user_id];
        let turns = conn.query_one(
            "SELECT COUNT(*)::bigint
             FROM chat_usage
             WHERE account_id = $1::text::uuid
               AND user_id = $2::text::uuid
               AND created_at > NOW() - INTERVAL '1 hour'",
            &params,
        );
        // Pipelined on the one connection, so the second query adds no round trips.
        let (allowance, turns) = tokio::join!(usage_budget::allowance(&conn, &account_id), turns);
        let turns: i64 = turns
            .map_err(AuthError::from)
            .map_err(QuotaError::Db)?
            .get(0);
        Ok::<_, QuotaError>((allowance.map_err(QuotaError::Db)?, turns))
    })?;

    if allowance == Allowance::NotIncluded {
        return Err(QuotaError::Exceeded(
            "AI-powered insights are not included in your plan; upgrade to a paid plan to use them"
                .to_string(),
        ));
    }
    if turns_this_hour >= MAX_TURNS_PER_HOUR {
        return Err(QuotaError::Exceeded(format!(
            "you have reached the limit of {MAX_TURNS_PER_HOUR} questions per hour; try again shortly"
        )));
    }
    if allowance == Allowance::Exhausted {
        return Err(QuotaError::Exceeded(
            "this account has used its monthly allowance for AI and other usage-based features; it resets at the start of next month"
                .to_string(),
        ));
    }
    Ok(())
}

/// Refuses a demo guest turn that would put this IP over either limit.
pub(crate) fn check_demo_guest_quota(
    server_context: Arc<ServerContext>,
    client_ip: IpAddr,
) -> Result<(), QuotaError> {
    let ip_text = client_ip.to_string();
    let (turns_this_hour, tokens_lifetime) = block_on(async move {
        let conn = open_main_database_read(server_context)
            .await
            .map_err(|error| QuotaError::Db(AuthError::Db(error.to_string())))?;
        let row = conn
            .query_one(
                "SELECT
                     (
                         SELECT COUNT(*)::bigint
                         FROM demo_chat_usage
                         WHERE client_ip = $1::text::inet
                           AND created_at > NOW() - INTERVAL '1 hour'
                     ),
                     (
                         SELECT COALESCE(SUM(prompt_tokens + completion_tokens), 0)::bigint
                         FROM demo_chat_usage
                         WHERE client_ip = $1::text::inet
                     )",
                &[&ip_text as &(dyn ToSql + Sync)],
            )
            .await
            .map_err(|error| QuotaError::Db(AuthError::from(error)))?;
        let turns: i64 = row.get(0);
        let tokens: i64 = row.get(1);
        Ok::<_, QuotaError>((turns, tokens))
    })?;

    if turns_this_hour >= MAX_TURNS_PER_HOUR {
        return Err(QuotaError::Exceeded(format!(
            "you have reached the limit of {MAX_TURNS_PER_HOUR} questions per hour; try again shortly"
        )));
    }
    if tokens_lifetime >= DEMO_GUEST_TOKEN_BUDGET {
        return Err(QuotaError::Exceeded(
            "this demo has reached its AI usage limit; sign up for a free account to keep asking"
                .to_string(),
        ));
    }
    Ok(())
}

/// Records what a finished turn cost. Failures are logged by the caller rather
/// than surfaced: the user already has their answer.
pub(crate) async fn record_usage(
    server_context: Arc<ServerContext>,
    account_id: &str,
    user_id: &str,
    project_id: &str,
    model: &str,
    prompt_tokens: i32,
    completion_tokens: i32,
) -> Result<(), AuthError> {
    let project_key =
        parse_project_id(project_id).map_err(|error| AuthError::Db(error.to_string()))?;
    let conn = open_main_database_write(server_context).await?;
    // A turn can finish after an admin removed its user. Its usage still
    // counts against the account, unattributed, as their earlier turns do.
    // The subquery covers a removal already committed; one committing while
    // this insert waits on the account row passes the subquery and then fails
    // the foreign key, so that case is retried without the user.
    let insert = async |user_id: Option<&str>| {
        conn.execute(
            "INSERT INTO chat_usage
                 (account_id, user_id, project_id, model, prompt_tokens, completion_tokens)
             VALUES (
                 $1::text::uuid,
                 (SELECT id FROM users WHERE id = $2::text::uuid),
                 $3, $4, $5, $6
             )",
            &[
                &account_id as &(dyn ToSql + Sync),
                &user_id as &(dyn ToSql + Sync),
                &project_key as &(dyn ToSql + Sync),
                &model as &(dyn ToSql + Sync),
                &prompt_tokens as &(dyn ToSql + Sync),
                &completion_tokens as &(dyn ToSql + Sync),
            ],
        )
        .await
    };
    match insert(Some(user_id)).await {
        Err(error) if crate::auth::db_constraint(&error) == Some("chat_usage_user_id_fkey") => {
            insert(None).await.map_err(AuthError::from)?;
        }
        result => {
            result.map_err(AuthError::from)?;
        }
    }
    Ok(())
}

pub(crate) async fn record_demo_guest_usage(
    server_context: Arc<ServerContext>,
    client_ip: IpAddr,
    model: &str,
    prompt_tokens: i32,
    completion_tokens: i32,
) -> Result<(), AuthError> {
    let ip_text = client_ip.to_string();
    let conn = open_main_database_write(server_context).await?;
    conn.execute(
        "INSERT INTO demo_chat_usage
             (client_ip, model, prompt_tokens, completion_tokens)
         VALUES ($1::text::inet, $2, $3, $4)",
        &[
            &ip_text as &(dyn ToSql + Sync),
            &model as &(dyn ToSql + Sync),
            &prompt_tokens as &(dyn ToSql + Sync),
            &completion_tokens as &(dyn ToSql + Sync),
        ],
    )
    .await
    .map_err(AuthError::from)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat_connection() -> ProjectConnection {
        let conn = crate::test_utils::open_migrated_db();
        conn.execute(
            "INSERT INTO chat_conversations (id, user_id, title, created_at, updated_at)
             VALUES (1, 'user-a', 'first question', 100, 100)",
            [],
        )
        .expect("insert conversation");
        conn
    }

    #[tokio::test]
    async fn a_turn_finishing_after_its_user_was_removed_still_counts() {
        let (server_context, conn) = crate::test_utils::main_test_db("usage-removed").await;
        let account_id = crate::test_utils::insert_test_account(&conn, "usage_removed").await;
        let project_name = format!("p{}", account_id.replace('-', ""));
        let project_id: i32 = conn
            .query_one(
                "INSERT INTO projects (account_id, name) VALUES ($1::text::uuid, $2) RETURNING id",
                &[&account_id, &project_name],
            )
            .await
            .expect("insert project")
            .get(0);
        let user_id: String = conn
            .query_one(
                "INSERT INTO users (account_id, name, email)
                 VALUES ($1::text::uuid, 'Gone', $2) RETURNING id::text",
                &[&account_id, &format!("{project_name}@example.com")],
            )
            .await
            .expect("insert user")
            .get(0);
        conn.execute("DELETE FROM users WHERE id = $1::text::uuid", &[&user_id])
            .await
            .expect("remove user");

        record_usage(
            server_context,
            &account_id,
            &user_id,
            &project_id.to_string(),
            "model",
            10,
            5,
        )
        .await
        .expect("usage recorded");
        let row = conn
            .query_one(
                "SELECT user_id::text, prompt_tokens + completion_tokens
                 FROM chat_usage WHERE account_id = $1::text::uuid",
                &[&account_id],
            )
            .await
            .expect("usage row");
        assert_eq!(row.get::<_, Option<String>>(0), None);
        assert_eq!(row.get::<_, i32>(1), 15);

        conn.execute(
            "DELETE FROM accounts WHERE id = $1::text::uuid",
            &[&account_id],
        )
        .await
        .expect("delete account");
    }

    #[tokio::test]
    async fn a_turn_finishing_while_its_user_is_removed_still_counts() {
        let (server_context, conn) = crate::test_utils::main_test_db("usage-racing").await;
        let account_id = crate::test_utils::insert_test_account(&conn, "usage_racing").await;
        let project_name = format!("p{}", account_id.replace('-', ""));
        let project_id: i32 = conn
            .query_one(
                "INSERT INTO projects (account_id, name) VALUES ($1::text::uuid, $2) RETURNING id",
                &[&account_id, &project_name],
            )
            .await
            .expect("insert project")
            .get(0);
        let user_id: String = conn
            .query_one(
                "INSERT INTO users (account_id, name, email)
                 VALUES ($1::text::uuid, 'Leaving', $2) RETURNING id::text",
                &[&account_id, &format!("{project_name}@example.com")],
            )
            .await
            .expect("insert user")
            .get(0);

        // The removal holds the account row, as remove_user does, while the
        // turn's usage is recorded, then deletes the user and commits.
        let (usage_account, usage_user) = (account_id.clone(), user_id.clone());
        let recorded = conn
            .in_transaction(async |conn| {
                conn.query_one(
                    "SELECT 1 FROM accounts WHERE id = $1::text::uuid FOR UPDATE",
                    &[&account_id],
                )
                .await?;
                let recorded = tokio::spawn(async move {
                    record_usage(
                        server_context,
                        &usage_account,
                        &usage_user,
                        &project_id.to_string(),
                        "model",
                        10,
                        5,
                    )
                    .await
                });
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                conn.execute("DELETE FROM users WHERE id = $1::text::uuid", &[&user_id])
                    .await?;
                Ok::<_, AuthError>(recorded)
            })
            .await
            .expect("removal commits");
        recorded.await.expect("usage task").expect("usage recorded");
        let row = conn
            .query_one(
                "SELECT user_id::text, prompt_tokens + completion_tokens
                 FROM chat_usage WHERE account_id = $1::text::uuid",
                &[&account_id],
            )
            .await
            .expect("usage row");
        assert_eq!(row.get::<_, Option<String>>(0), None);
        assert_eq!(row.get::<_, i32>(1), 15);

        conn.execute(
            "DELETE FROM accounts WHERE id = $1::text::uuid",
            &[&account_id],
        )
        .await
        .expect("delete account");
    }

    #[test]
    fn appended_messages_get_dense_sequence_numbers() {
        let conn = chat_connection();
        append_message_with(&conn, 1, NewMessage::text("user", "hello")).expect("append");
        append_message_with(&conn, 1, NewMessage::text("assistant", "hi")).expect("append");
        append_message_with(
            &conn,
            1,
            NewMessage {
                role: "tool",
                content: "1 row",
                tool_sql: Some("SELECT 1"),
                tool_result: Some(r#"{"columns":["1"],"rows":[[1]]}"#),
            },
        )
        .expect("append");

        let seqs: Vec<i64> = conn
            .prepare("SELECT seq FROM chat_messages WHERE conversation_id = 1 ORDER BY seq")
            .expect("prepare")
            .query_map([], |row| row.get(0))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("rows");
        assert_eq!(seqs, vec![1, 2, 3]);
    }

    #[test]
    fn deleting_a_conversation_removes_its_messages() {
        let conn = chat_connection();
        append_message_with(&conn, 1, NewMessage::text("user", "hello")).expect("append");
        append_message_with(&conn, 1, NewMessage::text("assistant", "hi")).expect("append");

        delete_conversation_with(&conn, 1, "user-a").expect("delete");

        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM chat_messages", [], |row| row.get(0))
            .expect("count");
        assert_eq!(remaining, 0);
    }

    #[test]
    fn deleting_another_users_conversation_leaves_it_alone() {
        let conn = chat_connection();
        append_message_with(&conn, 1, NewMessage::text("user", "hello")).expect("append");

        assert!(matches!(
            delete_conversation_with(&conn, 1, "user-b"),
            Err(AuthError::NotFound(_))
        ));

        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM chat_messages", [], |row| row.get(0))
            .expect("count");
        assert_eq!(remaining, 1);
    }

    #[test]
    fn ownership_is_part_of_the_lookup() {
        let conn = chat_connection();
        assert!(ensure_conversation_owner(&conn, 1, "user-a").is_ok());
        assert!(matches!(
            ensure_conversation_owner(&conn, 1, "user-b"),
            Err(AuthError::NotFound(_))
        ));
    }

    #[test]
    fn titles_collapse_whitespace_and_clip_on_a_word_boundary() {
        assert_eq!(
            conversation_title("  which  pages\nconvert best? "),
            "which pages convert best?"
        );
        let long = "revenue ".repeat(30);
        let title = conversation_title(&long);
        assert!(title.chars().count() <= MAX_TITLE_CHARS + 1);
        assert!(title.ends_with('…'));
        assert!(!title.contains("  "));
    }

    #[test]
    fn a_single_long_word_is_clipped_mid_word() {
        let title = conversation_title(&"x".repeat(200));
        assert_eq!(title.chars().count(), MAX_TITLE_CHARS + 1);
    }

    /// What `check_quota` says to a turn from an account on `plan_type` that has
    /// used more than an unpaid trial's allowance.
    #[cfg(not(feature = "self-hosted"))]
    async fn quota_for(plan_type: &str, order: Option<(i32, bool, &str)>) -> Result<(), String> {
        let tag = "quota_gate";
        let (server_context, conn) = crate::test_utils::main_test_db(tag).await;
        let (account_id, user_id) = crate::usage_budget::insert_metered_account(
            &conn,
            tag,
            plan_type,
            Some("-7 days"),
            order,
        )
        .await;
        let quota_account_id = account_id.clone();
        let result = tokio::task::spawn_blocking(move || {
            check_quota(server_context, &quota_account_id, &user_id)
        })
        .await
        .expect("join quota check");
        conn.execute(
            "DELETE FROM accounts WHERE id = $1::text::uuid",
            &[&account_id],
        )
        .await
        .expect("delete account");
        match result {
            Ok(()) => Ok(()),
            Err(QuotaError::Exceeded(message)) => Err(message),
            Err(QuotaError::Db(_)) => panic!("quota check failed"),
        }
    }

    #[cfg(not(feature = "self-hosted"))]
    #[tokio::test(flavor = "multi_thread")]
    async fn quota_refuses_a_turn_the_allowance_does_not_cover() {
        assert_eq!(quota_for("growth", Some((900, true, "paid"))).await, Ok(()));
        let exhausted = quota_for("growth", None).await.unwrap_err();
        assert!(exhausted.contains("monthly allowance"), "{exhausted}");
        let not_included = quota_for("free", None).await.unwrap_err();
        assert!(
            not_included.contains("not included in your plan"),
            "{not_included}"
        );
    }
}
