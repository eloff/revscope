import { render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { mockApi, queryOf } from "../../../../../shared/test/api";
import { testSession } from "../../../../../shared/test/fixtures";
import { frame, streamOf } from "../../../../test/sse";
import AiChat from "./AiChat";

const BASE = "/v2/chat/projects/42";
const session = testSession();
const older = {
  id: 7,
  title: "Top referrers",
  created_at: 1_700_000_000,
  updated_at: 1_700_000_000,
};

function mockConversations(conversations: unknown[] = []) {
  return mockApi("get", `${BASE}/conversations`, { conversations });
}

function answer(...chunks: string[]) {
  return mockApi("post", BASE, () => streamOf(chunks));
}

async function ask(question: string) {
  const user = userEvent.setup();
  await user.type(screen.getByPlaceholderText(/Ask about traffic/), question);
  await user.click(screen.getByRole("button", { name: "Ask" }));
  return user;
}

describe("AiChat", () => {
  it("offers suggestions when there is no history", async () => {
    mockConversations();
    render(() => <AiChat projectId="42" session={session} />);
    expect(await screen.findByText("No conversations yet.")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /landing pages brought in the most revenue/ }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Ask" })).toBeDisabled();
  });

  it("shows a failure to load conversations", async () => {
    mockApi("get", `${BASE}/conversations`, { error: "Chat is unavailable." }, 503);
    render(() => <AiChat projectId="42" session={session} />);
    expect(await screen.findByText("Chat is unavailable.")).toBeInTheDocument();
  });

  it("streams an answer with its queries and lists the new conversation", async () => {
    const lists = mockConversations();
    const posts = answer(
      frame("conversation", { conversation_id: 12 }),
      frame("tool_call", { id: "q1", title: "Revenue by month", sql: "select 1" }),
      frame("tool_result", {
        id: "q1",
        sql: "select 1",
        columns: ["month", "revenue"],
        rows: [["Jan", 5]],
        has_more: false,
      }),
      frame("token", { text: "Revenue was **up**." }),
      frame("done", {}),
    );
    render(() => <AiChat projectId="42" session={session} />);
    await screen.findByText("No conversations yet.");
    mockConversations([{ ...older, id: 12, title: "Revenue trend" }]);
    await ask("  How is revenue?  ");

    expect(await screen.findByText("up")).toBeInTheDocument();
    expect(screen.getByText("How is revenue?")).toBeInTheDocument();
    expect(screen.getByText("Revenue by month")).toBeInTheDocument();
    expect(screen.getByRole("cell", { name: "Jan" })).toBeInTheDocument();
    expect(posts[0]?.body).toEqual({ message: "How is revenue?" });
    expect(posts[0]?.credentials).toBe("include");
    expect(await screen.findByRole("button", { name: "Revenue trend" })).toBeInTheDocument();
    expect(lists).toHaveLength(1);
    expect(screen.getByText(/Started by Ada Lovelace/)).toBeInTheDocument();
  });

  it("sends follow-ups into the same conversation", async () => {
    mockConversations([older]);
    const posts = answer(
      frame("conversation", { conversation_id: 7 }),
      frame("token", { text: "ok" }),
      frame("done", {}),
    );
    render(() => <AiChat projectId="42" session={session} />);
    await screen.findByText("Top referrers");
    await ask("first");
    await screen.findByText("ok");
    await ask("second");
    await vi.waitFor(() => expect(posts).toHaveLength(2));
    expect(posts[1]?.body).toMatchObject({ conversation_id: 7, message: "second" });
  });

  it("sends on Enter but not Shift+Enter", async () => {
    mockConversations();
    const posts = answer(frame("done", {}));
    render(() => <AiChat projectId="42" session={session} />);
    const user = userEvent.setup();
    const box = screen.getByPlaceholderText(/Ask about traffic/);
    await user.type(box, "line one{Shift>}{Enter}{/Shift}line two");
    expect(posts).toHaveLength(0);
    await user.keyboard("{Enter}");
    await vi.waitFor(() => expect(posts).toHaveLength(1));
    expect(posts[0]?.body).toMatchObject({ message: "line one\nline two" });
  });

  it("shows a server error event on the turn", async () => {
    mockConversations();
    answer(frame("error", { message: "The query timed out." }));
    render(() => <AiChat projectId="42" session={session} />);
    await ask("slow question");
    expect(await screen.findByText("The query timed out.")).toBeInTheDocument();
    expect(screen.queryByText("Thinking…")).toBeNull();
  });

  it("replays a stored conversation", async () => {
    mockConversations([older]);
    const loads = mockApi("get", `${BASE}/messages`, {
      messages: [
        { id: 1, seq: 1, role: "user", content: "Who refers?", created_at: 1 },
        {
          id: 2,
          seq: 2,
          role: "tool",
          content: "",
          tool_sql: "select ref",
          tool_result: {
            id: "t",
            title: "Referrers",
            sql: "select ref",
            columns: ["ref"],
            rows: [["hn"]],
            has_more: false,
          },
          created_at: 2,
        },
        { id: 3, seq: 3, role: "assistant", content: "Mostly HN.", created_at: 3 },
      ],
    });
    render(() => <AiChat projectId="42" session={session} />);
    await userEvent.click(await screen.findByRole("button", { name: "Top referrers" }));
    expect(await screen.findByText("Mostly HN.")).toBeInTheDocument();
    expect(screen.getByText("Who refers?")).toBeInTheDocument();
    expect(screen.getByRole("cell", { name: "hn" })).toBeInTheDocument();
    expect(queryOf(loads[0])).toEqual({ conversation_id: "7" });
  });

  it("drops a conversation deleted elsewhere when opened", async () => {
    mockConversations([older]);
    mockApi("get", `${BASE}/messages`, { error: "conversation not found" }, 404);
    render(() => <AiChat projectId="42" session={session} />);
    await userEvent.click(await screen.findByRole("button", { name: "Top referrers" }));
    expect(await screen.findByText("This conversation no longer exists.")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Top referrers" })).toBeNull();
  });

  it("deletes a conversation from the rail", async () => {
    mockConversations([older]);
    const deletes = mockApi("delete", `${BASE}/conversations`, {});
    render(() => <AiChat projectId="42" session={session} />);
    await userEvent.click(await screen.findByRole("button", { name: "Delete Top referrers" }));
    await vi.waitFor(() => expect(screen.queryByText("Top referrers")).toBeNull());
    expect(queryOf(deletes[0])).toEqual({ conversation_id: "7" });
  });

  it("starts over when asking into a conversation deleted elsewhere", async () => {
    mockConversations([older]);
    mockApi("get", `${BASE}/messages`, {
      messages: [{ id: 1, seq: 1, role: "user", content: "old question", created_at: 1 }],
    });
    mockApi("post", BASE, { error: "conversation not found" }, 404);
    render(() => <AiChat projectId="42" session={session} />);
    await userEvent.click(await screen.findByRole("button", { name: "Top referrers" }));
    await screen.findByText("old question");
    await ask("new question");
    expect(await screen.findByText("This conversation no longer exists.")).toBeInTheDocument();
    expect(screen.queryByText("old question")).toBeNull();
    expect(screen.getByText("new question")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Top referrers" })).toBeNull();
  });

  describe("public demo", () => {
    it("mints one challenge on first interaction and sends it with each question", async () => {
      mockConversations();
      const mints = mockApi("post", "/v2/public/chat/demo-token", {
        token: "challenge-1",
        expires_at: Date.now() / 1000 + 600,
      });
      const posts = answer(frame("done", {}));
      render(() => <AiChat projectId="42" />);
      await screen.findByText("No conversations yet.");
      expect(mints).toHaveLength(0);

      await ask("first");
      await vi.waitFor(() => expect(posts).toHaveLength(1));
      await ask("second");
      await vi.waitFor(() => expect(posts).toHaveLength(2));
      expect(mints).toHaveLength(1);
      expect(
        posts.map((post) => (post.body as { challenge_token?: string }).challenge_token),
      ).toEqual(["challenge-1", "challenge-1"]);
      expect(posts[0]?.credentials).toBe("omit");
    });

    it("refreshes a challenge that is about to expire", async () => {
      mockConversations();
      let minted = 0;
      const mints = mockApi("post", "/v2/public/chat/demo-token", () => {
        minted += 1;
        return Response.json({ token: `c${minted}`, expires_at: Date.now() / 1000 + 30 });
      });
      const posts = answer(frame("done", {}));
      render(() => <AiChat projectId="42" />);
      await ask("first");
      await vi.waitFor(() => expect(posts).toHaveLength(1));
      await ask("second");
      await vi.waitFor(() => expect(posts).toHaveLength(2));
      expect(mints.length).toBeGreaterThanOrEqual(2);
      expect(posts[1]?.body).not.toMatchObject({ challenge_token: "c1" });
    });

    it("reports a failed mint on the question", async () => {
      mockConversations();
      mockApi("post", "/v2/public/chat/demo-token", { error: "Too many demo questions." }, 429);
      const posts = answer(frame("done", {}));
      render(() => <AiChat projectId="42" />);
      await ask("first");
      expect(await screen.findByText("Too many demo questions.")).toBeInTheDocument();
      expect(posts).toHaveLength(0);
    });
  });
});
