// Hosted signup creates a per-website account with a plan; self-hosted signup does not.
#![cfg(not(feature = "self-hosted"))]

//! The Ask tab end to end: a question goes in, the agent asks a stub gateway
//! what to do, runs the SQL the stub asks for against the real project
//! database, and streams the answer back as server-sent events.
//!
//! `REVSCOPE_CHAT_URL` is what makes this possible: pointing it at a local
//! listener that replays canned completions exercises the whole route without
//! reaching a model.

mod common;

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use common::{DEMO_PROJECT_ID, HttpResponse, TestServer, TrackHit, page_id};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

const SIGNUP_PATH: &str = "/v2/public/auth/signup";
const DEMO_CHAT_TOKEN_PATH: &str = "/v2/public/chat/demo-token";
const CHAT_STATUS_PATH: &str = "/v2/public/chat/status";
const PASSWORD: &str = "a-long-enough-password";

fn chat_path(project_id: impl std::fmt::Display) -> String {
    format!("/v2/chat/projects/{project_id}")
}

fn conversations_path(project_id: impl std::fmt::Display) -> String {
    format!("/v2/chat/projects/{project_id}/conversations")
}

fn messages_path(project_id: impl std::fmt::Display) -> String {
    format!("/v2/chat/projects/{project_id}/messages")
}
const QUESTION: &str = "Which pages do I have?";
const ANSWER: &str = "You have two pages: /pricing and /signup.";
const TOOL_SQL: &str = "SELECT path FROM paths ORDER BY path";
const TOOL_TITLE: &str = "All tracked page paths";

/// One test rather than several because the gateway URL is process-global
/// environment state; running these concurrently would have them fight over it.
#[tokio::test]
async fn a_question_runs_sql_streams_the_answer_and_is_replayable() -> io::Result<()> {
    let mut server = TestServer::spawn("chat").await?;
    let account = signup(&server, "chat").await?;
    let cookie = format!("rr_session={}", account.session_id);
    let auth = [("Cookie", cookie.as_str())];

    for (index, path) in ["/pricing", "/signup"].into_iter().enumerate() {
        let hit = TrackHit::pageview(&account.project_name, path, &page_id(index as u8 + 1));
        let ingest = server
            .post_ingest_for(&account.project_name, &hit.to_form(), &[])
            .await?;
        assert_eq!(ingest.status_code(), 200, "{}", ingest.body_str());
    }
    server.flush().await;
    // Keeps the shared-memory project database alive for the rest of the test.
    let _anchor = server.project_db(&account.project_id.to_string());

    let gateway = StubGateway::start(vec![
        tool_call_completion(TOOL_TITLE, TOOL_SQL),
        answer_completion(ANSWER),
    ])
    .await?;
    // SAFETY: single-threaded setup before any request reads the configuration.
    unsafe {
        std::env::set_var("REVSCOPE_CHAT_URL", gateway.base_url());
        std::env::set_var("REVSCOPE_CHAT_API_KEY", "test-key");
        std::env::set_var("REVSCOPE_CHAT_MODEL", "grok/grok-4.7");
    }

    let status: Value = server.get_query(CHAT_STATUS_PATH, &[]).await?.json();
    assert_eq!(
        status["available"],
        json!(true),
        "the stub gateway counts as configured"
    );

    let response = server
        .post_json(
            &chat_path(account.project_id),
            &json!({
                "message": QUESTION,
            })
            .to_string(),
            &auth,
        )
        .await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    assert_eq!(
        response.header("content-type"),
        Some("text/event-stream; charset=utf-8")
    );
    assert_eq!(
        response.header("content-length"),
        None,
        "a stream of unknown length is delimited by the connection close"
    );

    let events = parse_sse(response.body_str());
    let names: Vec<&str> = events.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(
        names.first(),
        Some(&"conversation"),
        "the conversation id arrives before anything else: {names:?}"
    );
    assert_eq!(
        names.last(),
        Some(&"done"),
        "the turn ends explicitly: {names:?}"
    );
    assert!(
        names.contains(&"tool_call") && names.contains(&"tool_result"),
        "the query the model asked for is reported to the browser: {names:?}"
    );

    let conversation_id = event_data(&events, "conversation")["conversation_id"]
        .as_i64()
        .expect("conversation id");

    let tool_call = event_data(&events, "tool_call");
    assert_eq!(tool_call["sql"], json!(TOOL_SQL));
    assert_eq!(tool_call["title"], json!(TOOL_TITLE));

    let tool_result = event_data(&events, "tool_result");
    assert_eq!(tool_result["title"], json!(TOOL_TITLE));
    assert_eq!(tool_result["columns"], json!(["path"]));
    assert_eq!(tool_result["rows"], json!([["/pricing"], ["/signup"]]));

    let streamed: String = events
        .iter()
        .filter(|(name, _)| name == "token")
        .map(|(_, data)| data["text"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(streamed, ANSWER, "the prose arrives token by token");

    // The gateway saw the tool result on its second call, which is what lets
    // the model write prose about rows it never queried itself.
    let second_request = gateway.request(1).await;
    let messages = second_request["messages"].as_array().expect("messages");
    let tool_message = messages
        .iter()
        .find(|message| message["role"] == json!("tool"))
        .expect("the result is fed back as a tool message");
    assert!(
        tool_message["content"]
            .as_str()
            .unwrap_or_default()
            .contains("/pricing"),
        "{tool_message}"
    );
    assert_eq!(
        messages[0]["role"],
        json!("system"),
        "the system prompt is server-owned and comes first"
    );
    assert!(
        messages[0]["content"]
            .as_str()
            .unwrap_or_default()
            .contains("CREATE TABLE"),
        "the schema is handed to the model"
    );

    let conversations: Value = server
        .get_query(&conversations_path(account.project_id), &auth)
        .await?
        .json();
    assert_eq!(
        conversations["conversations"][0]["id"],
        json!(conversation_id)
    );
    assert_eq!(conversations["conversations"][0]["title"], json!(QUESTION));

    // Reopening replays the question, the table, and the answer without asking
    // the gateway anything.
    let stored: Value = server
        .get_query(
            &format!(
                "{}?conversation_id={conversation_id}",
                messages_path(account.project_id)
            ),
            &auth,
        )
        .await?
        .json();
    let stored_messages = stored["messages"].as_array().expect("messages");
    let roles: Vec<&str> = stored_messages
        .iter()
        .map(|message| message["role"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(
        roles,
        vec!["user", "tool", "assistant"],
        "{stored_messages:?}"
    );
    assert_eq!(stored_messages[0]["content"], json!(QUESTION));
    assert_eq!(stored_messages[1]["tool_sql"], json!(TOOL_SQL));
    assert_eq!(
        stored_messages[1]["tool_result"]["rows"],
        json!([["/pricing"], ["/signup"]]),
        "the table is stored so it redraws without re-running the query"
    );
    assert_eq!(
        stored_messages[1]["tool_result"]["title"],
        json!(TOOL_TITLE),
        "the title is stored with the table so a reopened thread keeps it"
    );
    assert_eq!(stored_messages[2]["content"], json!(ANSWER));

    let deleted = server
        .delete(
            &format!(
                "{}?conversation_id={conversation_id}",
                conversations_path(account.project_id)
            ),
            &auth,
        )
        .await?;
    assert_eq!(deleted.status_code(), 200, "{}", deleted.body_str());
    let after_delete: Value = server
        .get_query(&conversations_path(account.project_id), &auth)
        .await?
        .json();
    assert_eq!(after_delete["conversations"], json!([]));

    delete_account(&server, &account.account_name).await;
    Ok(())
}

#[tokio::test]
async fn chat_refuses_a_query_token_and_an_anonymous_caller() -> io::Result<()> {
    let server = TestServer::spawn("chat-auth").await?;
    let account = signup(&server, "chatauth").await?;

    let body = json!({
        "message": QUESTION,
    })
    .to_string();
    let chat = chat_path(account.project_id);

    let anonymous = server.post_json(&chat, &body, &[]).await?;
    assert_eq!(anonymous.status_code(), 401, "{}", anonymous.body_str());

    let minted: Value = server
        .post_json(
            &format!("/v2/session/projects/{}/query-token", account.project_id),
            "",
            &[("Cookie", &format!("rr_session={}", account.session_id))],
        )
        .await?
        .json();
    let token = minted["token"].as_str().expect("token");
    let with_token = server
        .post_json(
            &chat,
            &body,
            &[("Authorization", &format!("Bearer {token}"))],
        )
        .await?;
    assert_eq!(
        with_token.status_code(),
        401,
        "a query token must not be able to spend AI budget: {}",
        with_token.body_str()
    );

    delete_account(&server, &account.account_name).await;
    Ok(())
}

#[tokio::test]
async fn demo_chat_is_open_anonymously_and_history_stays_private() -> io::Result<()> {
    let server = TestServer::spawn("chat-demo").await?;
    let alice = signup(&server, "chatalice").await?;
    let bob = signup(&server, "chatbob").await?;
    let alice_cookie = format!("rr_session={}", alice.session_id);
    let bob_cookie = format!("rr_session={}", bob.session_id);
    let alice_auth = [("Cookie", alice_cookie.as_str())];
    let bob_auth = [("Cookie", bob_cookie.as_str())];
    let guest_a = [("CF-Connecting-IP", "203.0.113.10")];
    let guest_b = [("CF-Connecting-IP", "203.0.113.20")];

    // Seed one conversation per owner directly so we do not need the AI
    // gateway; the list route is what enforces access and isolation. Demo
    // transcripts live in the main database.
    {
        let conn = revscope::db::open_main_database_write(server.context.clone())
            .await
            .expect("open main db for seed");
        conn.execute(
            "INSERT INTO demo_chat_conversations (owner_key, client_ip, title)
             VALUES
                 ($1, '203.0.113.10', 'Alice thread'),
                 ($2, '203.0.113.11', 'Bob thread'),
                 ('203.0.113.10', '203.0.113.10', 'Guest A thread'),
                 ('203.0.113.20', '203.0.113.20', 'Guest B thread')",
            &[&alice.user_id.as_str(), &bob.user_id.as_str()],
        )
        .await
        .expect("seed demo conversations");
    }

    let anonymous_without_ip = server
        .get_query(
            &conversations_path(DEMO_PROJECT_ID),
            // Loopback peer IP still resolves; this just confirms the route is open.
            &[],
        )
        .await?;
    assert_eq!(
        anonymous_without_ip.status_code(),
        200,
        "demo Ask is open without a session: {}",
        anonymous_without_ip.body_str()
    );

    let alice_list: Value = server
        .get_query(&conversations_path(DEMO_PROJECT_ID), &alice_auth)
        .await?
        .json();
    let bob_list: Value = server
        .get_query(&conversations_path(DEMO_PROJECT_ID), &bob_auth)
        .await?
        .json();
    assert_eq!(
        alice_list["conversations"]
            .as_array()
            .expect("alice conversations")
            .len(),
        1,
        "{alice_list}"
    );
    assert_eq!(
        alice_list["conversations"][0]["title"],
        json!("Alice thread")
    );
    assert_eq!(
        bob_list["conversations"]
            .as_array()
            .expect("bob conversations")
            .len(),
        1,
        "{bob_list}"
    );
    assert_eq!(bob_list["conversations"][0]["title"], json!("Bob thread"));

    let guest_a_list: Value = server
        .get_query(&conversations_path(DEMO_PROJECT_ID), &guest_a)
        .await?
        .json();
    let guest_b_list: Value = server
        .get_query(&conversations_path(DEMO_PROJECT_ID), &guest_b)
        .await?
        .json();
    assert_eq!(
        guest_a_list["conversations"][0]["title"],
        json!("Guest A thread"),
        "{guest_a_list}"
    );
    assert_eq!(
        guest_b_list["conversations"][0]["title"],
        json!("Guest B thread"),
        "{guest_b_list}"
    );

    // A foreign conversation id must not be readable across owners.
    let bob_conversation_id = bob_list["conversations"][0]["id"].as_i64().expect("id");
    let alice_peek = server
        .get_query(
            &format!(
                "{}?conversation_id={bob_conversation_id}",
                messages_path(DEMO_PROJECT_ID)
            ),
            &alice_auth,
        )
        .await?;
    assert_eq!(
        alice_peek.status_code(),
        404,
        "alice must not read bob's demo thread: {}",
        alice_peek.body_str()
    );
    // Someone else's conversation answers exactly like one that never
    // existed, so ids cannot be probed.
    let missing = server
        .get_query(
            &format!(
                "{}?conversation_id={}",
                messages_path(DEMO_PROJECT_ID),
                i64::MAX
            ),
            &alice_auth,
        )
        .await?;
    assert_eq!(missing.status_code(), 404, "{}", missing.body_str());
    assert_eq!(missing.body_str(), alice_peek.body_str());

    let guest_b_id = guest_b_list["conversations"][0]["id"].as_i64().expect("id");
    let guest_a_peek = server
        .get_query(
            &format!(
                "{}?conversation_id={guest_b_id}",
                messages_path(DEMO_PROJECT_ID)
            ),
            &guest_a,
        )
        .await?;
    assert_eq!(
        guest_a_peek.status_code(),
        404,
        "guest A must not read guest B's demo thread: {}",
        guest_a_peek.body_str()
    );

    // Own-project Ask still requires a session.
    let own_project = server
        .get_query(&conversations_path(alice.project_id), &[])
        .await?;
    assert_eq!(
        own_project.status_code(),
        401,
        "non-demo Ask still requires a session: {}",
        own_project.body_str()
    );

    delete_account(&server, &alice.account_name).await;
    delete_account(&server, &bob.account_name).await;
    {
        let conn = revscope::db::open_main_database_write(server.context.clone())
            .await
            .expect("open main db for cleanup");
        conn.execute(
            "DELETE FROM demo_chat_conversations
             WHERE owner_key IN ($1, $2, '203.0.113.10', '203.0.113.20')",
            &[&alice.user_id.as_str(), &bob.user_id.as_str()],
        )
        .await
        .expect("cleanup demo conversations");
    }
    Ok(())
}

#[tokio::test]
async fn anonymous_demo_chat_requires_a_challenge_token() -> io::Result<()> {
    let server = TestServer::spawn("chat-demo-challenge").await?;
    let guest = [("CF-Connecting-IP", "203.0.113.40")];
    let body = json!({
        "message": QUESTION,
    })
    .to_string();
    let chat = chat_path(DEMO_PROJECT_ID);

    let without_challenge = server.post_json(&chat, &body, &guest).await?;
    assert_eq!(
        without_challenge.status_code(),
        400,
        "anonymous demo Ask must present a challenge: {}",
        without_challenge.body_str()
    );
    assert!(
        without_challenge
            .body_str()
            .contains("challenge token is required"),
        "{}",
        without_challenge.body_str()
    );

    let forged = server
        .post_json(
            &chat,
            &json!({
                "message": QUESTION,
                "challenge_token": "0123456789abcdef0123456789abcdef",
            })
            .to_string(),
            &guest,
        )
        .await?;
    assert_eq!(
        forged.status_code(),
        401,
        "an unknown challenge must not unlock Ask: {}",
        forged.body_str()
    );

    let minted: Value = server
        .post_json(DEMO_CHAT_TOKEN_PATH, "", &guest)
        .await?
        .json();
    let token = minted["token"].as_str().expect("token");
    assert_eq!(token.len(), 40, "{minted}");
    assert!(minted["expires_at"].as_u64().expect("expires_at") > 0);

    // Auth and challenge pass; without a configured gateway the turn stops
    // before spending. Another test may have left the gateway env set, so
    // accept anything that is not an auth rejection.
    let with_challenge = server
        .post_json(
            &chat,
            &json!({
                "message": QUESTION,
                "challenge_token": token,
            })
            .to_string(),
            &guest,
        )
        .await?;
    assert!(
        ![400, 401].contains(&with_challenge.status_code()),
        "a valid challenge should clear auth: {}",
        with_challenge.body_str()
    );

    // A challenge minted for one IP must not work from another.
    let other_guest = [("CF-Connecting-IP", "203.0.113.41")];
    let wrong_ip = server
        .post_json(
            &chat,
            &json!({
                "message": QUESTION,
                "challenge_token": token,
            })
            .to_string(),
            &other_guest,
        )
        .await?;
    assert_eq!(
        wrong_ip.status_code(),
        401,
        "challenge tokens are bound to the minting IP: {}",
        wrong_ip.body_str()
    );

    Ok(())
}

/// A minimal OpenAI-compatible endpoint that replays prepared responses in
/// order and records the request bodies it was sent.
struct StubGateway {
    addr: SocketAddr,
    requests: Arc<Mutex<Vec<Value>>>,
}

impl StubGateway {
    async fn start(responses: Vec<String>) -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();

        tokio::spawn(async move {
            for response in responses {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                if let Some(body) = serve_once(stream, &response).await {
                    recorded.lock().await.push(body);
                }
            }
        });

        Ok(Self { addr, requests })
    }

    fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// The body of the nth request, waited for rather than assumed to have
    /// arrived, since the agent runs on its own task.
    async fn request(&self, index: usize) -> Value {
        for _ in 0..100 {
            if let Some(request) = self.requests.lock().await.get(index) {
                return request.clone();
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("the gateway never received request {index}");
    }
}

async fn serve_once(mut stream: TcpStream, sse_body: &str) -> Option<Value> {
    let mut raw = Vec::new();
    let mut buffer = [0_u8; 4096];
    let body = loop {
        let read = stream.read(&mut buffer).await.ok()?;
        if read == 0 {
            return None;
        }
        raw.extend_from_slice(&buffer[..read]);
        let Some(split) = raw.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&raw[..split]).to_ascii_lowercase();
        let content_length = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(0);
        if raw.len() >= split + 4 + content_length {
            break raw[split + 4..split + 4 + content_length].to_vec();
        }
    };

    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{sse_body}",
        sse_body.len()
    );
    stream.write_all(response.as_bytes()).await.ok()?;
    stream.flush().await.ok()?;
    serde_json::from_slice(&body).ok()
}

/// A completion that asks for one `run_query` call, with the arguments split
/// across chunks the way a real gateway streams them.
fn tool_call_completion(title: &str, sql: &str) -> String {
    let arguments = json!({ "title": title, "sql": sql }).to_string();
    let split = arguments.len() / 2;
    [
        chunk(json!({"choices":[{"delta":{"tool_calls":[{
            "index": 0,
            "id": "call_test",
            "type": "function",
            "function": { "name": "run_query", "arguments": &arguments[..split] },
        }]}}]})),
        chunk(json!({"choices":[{"delta":{"tool_calls":[{
            "index": 0,
            "function": { "arguments": &arguments[split..] },
        }]}}]})),
        chunk(json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]})),
        chunk(json!({"choices":[],"usage":{"prompt_tokens":120,"completion_tokens":18}})),
        "data: [DONE]\n\n".to_string(),
    ]
    .concat()
}

/// A completion that streams prose one word at a time.
fn answer_completion(answer: &str) -> String {
    let mut frames = String::new();
    let mut remaining = answer;
    while !remaining.is_empty() {
        let end = remaining[1..]
            .find(' ')
            .map(|index| index + 2)
            .unwrap_or(remaining.len());
        frames.push_str(&chunk(
            json!({"choices":[{"delta":{"content":&remaining[..end]}}]}),
        ));
        remaining = &remaining[end..];
    }
    frames.push_str(&chunk(
        json!({"choices":[{"delta":{},"finish_reason":"stop"}]}),
    ));
    frames.push_str(&chunk(
        json!({"choices":[],"usage":{"prompt_tokens":260,"completion_tokens":32}}),
    ));
    frames.push_str("data: [DONE]\n\n");
    frames
}

fn chunk(value: Value) -> String {
    format!("data: {value}\n\n")
}

fn parse_sse(body: &str) -> Vec<(String, Value)> {
    let mut events = Vec::new();
    for frame in body.split("\n\n") {
        let mut name = String::new();
        let mut data = String::new();
        for line in frame.lines() {
            if let Some(rest) = line.strip_prefix("event:") {
                name = rest.trim().to_string();
            } else if let Some(rest) = line.strip_prefix("data:") {
                data.push_str(rest.trim_start());
            }
        }
        if let Ok(value) = serde_json::from_str(&data) {
            events.push((name, value));
        }
    }
    events
}

fn event_data(events: &[(String, Value)], name: &str) -> Value {
    events
        .iter()
        .find(|(event, _)| event == name)
        .map(|(_, data)| data.clone())
        .unwrap_or_else(|| panic!("no {name} event in {events:?}"))
}

struct TestAccount {
    account_name: String,
    project_name: String,
    project_id: i32,
    user_id: String,
    session_id: String,
}

async fn signup(server: &TestServer, slug: &str) -> io::Result<TestAccount> {
    let slug = format!(
        "{slug}{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after the unix epoch")
            .as_micros()
    );
    let account_name = format!("{slug}.example");
    let project_name = format!("{slug}example");
    let response = server
        .post_json(
            SIGNUP_PATH,
            &json!({
                "name": "Ada Lovelace",
                "account_name": account_name,
                "email": format!("{slug}@example.com"),
                "password": PASSWORD,
                "plan": "starter",
                "events": "100k",
                "billing": "monthly",
            })
            .to_string(),
            &[],
        )
        .await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    let body: Value = response.json();
    let project_id = body["projects"][&project_name]["id"]
        .as_str()
        .expect("project id")
        .parse()
        .expect("numeric project id");
    let user_id = body["user_id"].as_str().expect("user id").to_string();

    Ok(TestAccount {
        account_name,
        project_name,
        project_id,
        user_id,
        session_id: session_id_from_cookie(&response),
    })
}

fn session_id_from_cookie(response: &HttpResponse) -> String {
    response
        .header("set-cookie")
        .expect("set-cookie header")
        .split(';')
        .next()
        .and_then(|pair| pair.trim().strip_prefix("rr_session="))
        .expect("session cookie value")
        .to_string()
}

async fn delete_account(server: &TestServer, account_name: &str) {
    let conn = revscope::db::open_main_database_write(server.context.clone())
        .await
        .expect("open main database");
    conn.execute("DELETE FROM accounts WHERE name = $1", &[&account_name])
        .await
        .expect("delete test account");
}
