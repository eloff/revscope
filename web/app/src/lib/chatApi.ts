import { getApiBaseUrl } from "@revscope/web-shared/apiBaseUrl";
import { projectApiPath, publicApiPath } from "@revscope/web-shared/apiPaths";
import {
  authRequestInit,
  fetchJson,
  noticeUnauthorized,
  readJson,
} from "@revscope/web-shared/apiRequest";
import type { AuthSession } from "@revscope/web-shared/authState";

/** Shape of a `run_query` result, matching the query API. */
export type QueryTable = {
  columns: string[];
  rows: (string | number | boolean | null)[][];
  has_more: boolean;
};

export type ChatToolResult = {
  id: string;
  /** The model's short description of what the query fetches. */
  title?: string | null;
  sql: string;
  /** Set when the query failed, in which case there is no table. */
  error?: string;
} & Partial<QueryTable>;

export type ChatToolCall = {
  id: string;
  title?: string | null;
  sql: string;
  limit?: number;
  offset?: number;
};

export type ConversationSummary = {
  id: number;
  title: string;
  created_at: number;
  updated_at: number;
};

export type StoredMessage = {
  id: number;
  seq: number;
  role: "user" | "assistant" | "tool";
  content: string;
  tool_sql?: string;
  tool_result?: ChatToolResult;
  created_at: number;
};

export type ChatStreamHandlers = {
  /** The conversation this turn belongs to; arrives before any other event. */
  onConversation: (conversationId: number) => void;
  onToken: (text: string) => void;
  onToolCall: (call: ChatToolCall) => void;
  onToolResult: (result: ChatToolResult) => void;
  onError: (message: string) => void;
};

export async function fetchChatAvailable(): Promise<boolean> {
  try {
    const response = await fetch(new URL(publicApiPath("chat/status"), getApiBaseUrl()));
    if (!response.ok) {
      return false;
    }
    const payload = (await response.json()) as { available?: boolean };
    return payload.available === true;
  } catch {
    // A dashboard that cannot reach the API has bigger problems to report than
    // a missing tab, so treat this as "not offered" and stay quiet.
    return false;
  }
}

/**
 * Mints a short-lived token that unlocks anonymous demo Ask. Bound to the
 * caller's IP on the server; the Ask UI only requests this after a real
 * interaction so a bare script cannot spend tokens.
 */
export async function fetchDemoChatChallenge(): Promise<{ token: string; expiresAt: number }> {
  const response = await fetch(new URL(publicApiPath("chat/demo-token"), getApiBaseUrl()), {
    method: "POST",
    credentials: "omit",
  });
  const payload = await readJson<{ token?: string; expires_at?: number }>(
    response,
    "Unable to start the demo assistant.",
  );
  if (!payload.token || typeof payload.expires_at !== "number") {
    throw new Error("Unable to start the demo assistant.");
  }
  return { token: payload.token, expiresAt: payload.expires_at };
}

export async function fetchConversations(
  session: AuthSession | undefined,
  projectId: string,
): Promise<ConversationSummary[]> {
  const url = new URL(projectApiPath("chat", projectId, "conversations"), getApiBaseUrl());
  const payload = await fetchJson<{ conversations?: ConversationSummary[] }>(
    url,
    authRequestInit(session),
    "Unable to load conversations.",
  );
  return payload.conversations ?? [];
}

/**
 * The conversation is gone, typically deleted from another tab. The server
 * answers the same for one owned by someone else.
 */
export class ConversationNotFoundError extends Error {
  constructor() {
    super("This conversation no longer exists.");
    this.name = "ConversationNotFoundError";
  }
}

/**
 * True only for the server's own "this conversation is gone" answer. A 404 can
 * also mean an unknown route or a missing project database, and treating those
 * as a deleted conversation would drop it from the list while it still exists.
 */
async function isConversationNotFound(response: Response): Promise<boolean> {
  if (response.status !== 404) {
    return false;
  }
  const payload = (await response
    .clone()
    .json()
    .catch(() => ({}))) as { error?: string };
  return payload.error === "conversation not found";
}

export async function fetchMessages(
  session: AuthSession | undefined,
  projectId: string,
  conversationId: number,
): Promise<StoredMessage[]> {
  const url = new URL(projectApiPath("chat", projectId, "messages"), getApiBaseUrl());
  url.searchParams.set("conversation_id", String(conversationId));
  const response = await fetch(url, authRequestInit(session));
  if (await isConversationNotFound(response)) {
    throw new ConversationNotFoundError();
  }
  const payload = await readJson<{ messages?: StoredMessage[] }>(
    response,
    "Unable to load this conversation.",
  );
  return payload.messages ?? [];
}

export async function deleteConversation(
  session: AuthSession | undefined,
  projectId: string,
  conversationId: number,
): Promise<void> {
  const url = new URL(projectApiPath("chat", projectId, "conversations"), getApiBaseUrl());
  url.searchParams.set("conversation_id", String(conversationId));
  const response = await fetch(url, authRequestInit(session, { method: "DELETE" }));
  noticeUnauthorized(response);
  // Already gone, which is what the caller wanted.
  if (!response.ok && !(await isConversationNotFound(response))) {
    const payload = (await response.json().catch(() => ({}))) as { error?: string };
    throw new Error(payload.error ?? "Unable to delete this conversation.");
  }
}

/**
 * Asks a question and consumes the answer as it is produced.
 *
 * `EventSource` cannot POST, so this reads the response body directly. The
 * stream is delimited by the connection close, which means an abrupt end is
 * indistinguishable from a finished one at the transport level; the server
 * therefore ends every turn with an explicit `done` or `error` event, and this
 * reports a stream missing both as a failure.
 */
export async function streamChat(
  session: AuthSession | undefined,
  projectId: string,
  conversationId: number | undefined,
  message: string,
  handlers: ChatStreamHandlers,
  signal?: AbortSignal,
  challengeToken?: string,
): Promise<void> {
  const response = await fetch(
    new URL(projectApiPath("chat", projectId, ""), getApiBaseUrl()),
    authRequestInit(session, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        conversation_id: conversationId,
        message,
        ...(challengeToken ? { challenge_token: challengeToken } : {}),
      }),
      signal,
    }),
  );

  noticeUnauthorized(response);
  if (await isConversationNotFound(response)) {
    throw new ConversationNotFoundError();
  }
  if (!response.ok) {
    const payload = (await response.json().catch(() => ({}))) as { error?: string };
    throw new Error(payload.error ?? "The assistant could not be reached.");
  }
  if (!response.body) {
    throw new Error("The assistant returned an empty response.");
  }

  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  let finished = false;

  try {
    while (!finished) {
      const { done, value } = await reader.read();
      if (done) {
        break;
      }
      buffer += decoder.decode(value, { stream: true });
      let boundary = buffer.indexOf("\n\n");
      while (boundary !== -1) {
        const frame = buffer.slice(0, boundary);
        buffer = buffer.slice(boundary + 2);
        if (dispatchFrame(frame, handlers)) {
          finished = true;
          break;
        }
        boundary = buffer.indexOf("\n\n");
      }
    }
  } finally {
    await reader.cancel().catch(() => {});
  }

  if (!finished) {
    throw new Error("The answer was cut short. Try asking again.");
  }
}

/** Returns true once the turn has ended, either with an answer or an error. */
function dispatchFrame(frame: string, handlers: ChatStreamHandlers): boolean {
  const parsed = parseFrame(frame);
  if (!parsed) {
    return false;
  }
  const { event, data } = parsed;
  switch (event) {
    case "conversation":
      handlers.onConversation(Number(data.conversation_id));
      return false;
    case "token":
      handlers.onToken(String(data.text ?? ""));
      return false;
    case "tool_call":
      handlers.onToolCall(data as ChatToolCall);
      return false;
    case "tool_result":
      handlers.onToolResult(data as ChatToolResult);
      return false;
    case "error":
      handlers.onError(String(data.message ?? "Something went wrong."));
      return true;
    case "done":
      return true;
    default:
      return false;
  }
}

// biome-ignore lint/suspicious/noExplicitAny: event payloads are narrowed by the dispatcher
function parseFrame(frame: string): { event: string; data: any } | undefined {
  let event = "message";
  const dataLines: string[] = [];
  for (const line of frame.split("\n")) {
    if (line.startsWith("event:")) {
      event = line.slice("event:".length).trim();
    } else if (line.startsWith("data:")) {
      dataLines.push(line.slice("data:".length).replace(/^ /, ""));
    }
  }
  if (dataLines.length === 0) {
    return undefined;
  }
  try {
    return { event, data: JSON.parse(dataLines.join("\n")) };
  } catch {
    return undefined;
  }
}
