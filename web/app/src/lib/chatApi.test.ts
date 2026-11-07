import { useAuthState } from "@revscope/web-shared/authState";
import { renderHook } from "@solidjs/testing-library";
import { HttpResponse } from "msw";
import { afterEach, describe, expect, it, vi } from "vitest";
import { mockApi, queryOf } from "../../../shared/test/api";
import { signIn, signOut } from "../../../shared/test/auth";
import { testSession } from "../../../shared/test/fixtures";
import { frame, streamOf } from "../../test/sse";
import {
  ConversationNotFoundError,
  deleteConversation,
  fetchChatAvailable,
  fetchConversations,
  fetchDemoChatChallenge,
  fetchMessages,
  streamChat,
} from "./chatApi";

const session = testSession();
const CHAT = "/v2/chat/projects/42";

function handlers() {
  return {
    onConversation: vi.fn(),
    onToken: vi.fn(),
    onToolCall: vi.fn(),
    onToolResult: vi.fn(),
    onError: vi.fn(),
  };
}

describe("fetchChatAvailable", () => {
  it("is true only when the server says so", async () => {
    mockApi("get", "/v2/public/chat/status", { available: true });
    await expect(fetchChatAvailable()).resolves.toBe(true);
    mockApi("get", "/v2/public/chat/status", { available: "yes" });
    await expect(fetchChatAvailable()).resolves.toBe(false);
    mockApi("get", "/v2/public/chat/status", { available: true }, 503);
    await expect(fetchChatAvailable()).resolves.toBe(false);
  });

  it("is false when the API can't be reached", async () => {
    mockApi("get", "/v2/public/chat/status", () => HttpResponse.error());
    await expect(fetchChatAvailable()).resolves.toBe(false);
  });
});

describe("fetchDemoChatChallenge", () => {
  it("returns the token without sending cookies", async () => {
    const calls = mockApi("post", "/v2/public/chat/demo-token", { token: "t", expires_at: 9 });
    await expect(fetchDemoChatChallenge()).resolves.toEqual({ token: "t", expiresAt: 9 });
    expect(calls[0]?.credentials).toBe("omit");
  });

  it.each([
    ["an error status", { error: "Too many requests." }, 429, "Too many requests."],
    ["a missing token", { expires_at: 9 }, 200, "Unable to start the demo assistant."],
    ["a missing expiry", { token: "t" }, 200, "Unable to start the demo assistant."],
  ])("rejects %s", async (_label, body, status, message) => {
    mockApi("post", "/v2/public/chat/demo-token", body, status);
    await expect(fetchDemoChatChallenge()).rejects.toThrow(message);
  });

  it("rejects a non-JSON body", async () => {
    mockApi("post", "/v2/public/chat/demo-token", () => new HttpResponse("oops", { status: 502 }));
    await expect(fetchDemoChatChallenge()).rejects.toThrow("Unable to start the demo assistant.");
  });
});

describe("conversations", () => {
  it("lists conversations, defaulting to none", async () => {
    mockApi("get", `${CHAT}/conversations`, {});
    await expect(fetchConversations(session, "42")).resolves.toEqual([]);
    mockApi("get", `${CHAT}/conversations`, { error: "No access." }, 403);
    await expect(fetchConversations(undefined, "42")).rejects.toThrow("No access.");
  });

  it("loads messages for a conversation", async () => {
    const calls = mockApi("get", `${CHAT}/messages`, { messages: [{ id: 1 }] });
    await expect(fetchMessages(session, "42", 3)).resolves.toEqual([{ id: 1 }]);
    expect(queryOf(calls[0])).toEqual({ conversation_id: "3" });
  });

  it("raises ConversationNotFoundError only for the server's own not-found answer", async () => {
    mockApi("get", `${CHAT}/messages`, { error: "conversation not found" }, 404);
    await expect(fetchMessages(session, "42", 3)).rejects.toBeInstanceOf(ConversationNotFoundError);

    mockApi("get", `${CHAT}/messages`, { error: "project not found" }, 404);
    const error = await fetchMessages(session, "42", 3).catch((caught) => caught);
    expect(error).not.toBeInstanceOf(ConversationNotFoundError);
    expect(error.message).toBe("project not found");

    mockApi("get", `${CHAT}/messages`, () => new HttpResponse("Not Found", { status: 404 }));
    await expect(fetchMessages(session, "42", 3)).rejects.toThrow(
      "Unable to load this conversation.",
    );
  });

  it("deletes a conversation, treating an already-deleted one as success", async () => {
    const calls = mockApi("delete", `${CHAT}/conversations`, {});
    await deleteConversation(session, "42", 3);
    expect(queryOf(calls[0])).toEqual({ conversation_id: "3" });

    mockApi("delete", `${CHAT}/conversations`, { error: "conversation not found" }, 404);
    await expect(deleteConversation(session, "42", 3)).resolves.toBeUndefined();

    mockApi("delete", `${CHAT}/conversations`, {}, 500);
    await expect(deleteConversation(session, "42", 3)).rejects.toThrow(
      "Unable to delete this conversation.",
    );
  });
});

describe("a project gone since the tab loaded", () => {
  afterEach(async () => {
    await signOut();
  });

  const gone = { error: "project not found", code: "not_found" };

  it("re-reads the account when deleting a conversation", async () => {
    signIn(session);
    const me = mockApi("get", "/v2/session/me", session);
    mockApi("delete", `${CHAT}/conversations`, gone, 404);
    await expect(deleteConversation(session, "42", 3)).rejects.toThrow("project not found");
    await vi.waitFor(() => expect(me).toHaveLength(1));
  });

  it("re-reads the account when asking a question", async () => {
    signIn(session);
    const me = mockApi("get", "/v2/session/me", session);
    mockApi("post", CHAT, gone, 404);
    await expect(streamChat(session, "42", 3, "q", handlers())).rejects.toThrow(
      "project not found",
    );
    await vi.waitFor(() => expect(me).toHaveLength(1));
  });
});

describe("streamChat", () => {
  it("posts the question and dispatches each event in order", async () => {
    const calls = mockApi("post", CHAT, () =>
      streamOf([
        frame("conversation", { conversation_id: 12 }),
        `${frame("token", { text: "Hel" })}event: tok`,
        `en\ndata: {"text":"lo"}\n\n`,
        frame("tool_call", { id: "q1", sql: "select 1" }),
        frame("tool_result", { id: "q1", sql: "select 1", columns: ["a"], rows: [[1]] }),
        // Same chunk, so it is buffered rather than left unread: msw's mocked
        // body never settles `reader.cancel()` while chunks are still queued.
        frame("done", {}) + frame("token", { text: "ignored after done" }),
      ]),
    );
    const on = handlers();
    await streamChat(session, "42", undefined, "How much?", on, undefined, "challenge");

    expect(calls[0]?.body).toEqual({ message: "How much?", challenge_token: "challenge" });
    expect(on.onConversation).toHaveBeenCalledWith(12);
    expect(on.onToken.mock.calls).toEqual([["Hel"], ["lo"]]);
    expect(on.onToolCall).toHaveBeenCalledWith({ id: "q1", sql: "select 1" });
    expect(on.onToolResult).toHaveBeenCalledWith(
      expect.objectContaining({ id: "q1", rows: [[1]] }),
    );
    expect(on.onError).not.toHaveBeenCalled();
  });

  it("joins multi-line data and skips malformed or unknown frames", async () => {
    mockApi("post", CHAT, () =>
      streamOf([
        'event: token\ndata: {"text":\ndata: "joined"}\n\n',
        "event: token\ndata: not json\n\n",
        "event: token\n\n",
        frame("mystery", { x: 1 }),
        frame("done", {}),
      ]),
    );
    const on = handlers();
    await streamChat(session, "42", 3, "q", on);
    expect(on.onToken.mock.calls).toEqual([["joined"]]);
  });

  it("ends the turn on a server error event", async () => {
    mockApi("post", CHAT, () => streamOf([frame("error", { message: "Query failed." })]));
    const on = handlers();
    await streamChat(session, "42", 3, "q", on);
    expect(on.onError).toHaveBeenCalledWith("Query failed.");
  });

  it("reports a stream that ends without done or error", async () => {
    mockApi("post", CHAT, () => streamOf([frame("token", { text: "par" })]));
    await expect(streamChat(session, "42", 3, "q", handlers())).rejects.toThrow(
      "The answer was cut short.",
    );
  });

  it("maps HTTP failures to errors", async () => {
    mockApi("post", CHAT, { error: "conversation not found" }, 404);
    await expect(streamChat(session, "42", 3, "q", handlers())).rejects.toBeInstanceOf(
      ConversationNotFoundError,
    );
    mockApi("post", CHAT, { error: "Rate limited." }, 429);
    await expect(streamChat(session, "42", 3, "q", handlers())).rejects.toThrow("Rate limited.");
    mockApi("post", CHAT, () => new HttpResponse("bad gateway", { status: 502 }));
    await expect(streamChat(session, "42", 3, "q", handlers())).rejects.toThrow(
      "The assistant could not be reached.",
    );
  });
});

describe("a 401 from a dead session", () => {
  afterEach(async () => {
    await signOut();
  });

  it.each([
    [
      "deleting a conversation",
      () => {
        mockApi("delete", `${CHAT}/conversations`, { error: "invalid session" }, 401);
        return deleteConversation(session, "42", 3);
      },
    ],
    [
      "asking a question",
      () => {
        mockApi("post", CHAT, { error: "invalid session" }, 401);
        return streamChat(session, "42", 3, "q", handlers());
      },
    ],
  ])("signs the tab out when %s", async (_label, call) => {
    signIn(session);
    mockApi("get", "/v2/session/me", { error: "invalid session" }, 401);
    const { result } = renderHook(useAuthState);
    await expect(call()).rejects.toThrow("invalid session");
    await vi.waitFor(() => expect(result.authUser()).toBeNull());
  });
});
