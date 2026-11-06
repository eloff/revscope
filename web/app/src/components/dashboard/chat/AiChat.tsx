import type { AuthSession } from "@revscope/web-shared/authState";
import { docsUrl } from "@revscope/web-shared/links";
import { createEffect, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { createStore, produce } from "solid-js/store";
import {
  type ChatToolResult,
  ConversationNotFoundError,
  type ConversationSummary,
  deleteConversation,
  fetchConversations,
  fetchDemoChatChallenge,
  fetchMessages,
  streamChat,
} from "../../../lib/chatApi";
import { errorMessage, message } from "../messages.css";
import TabDocsLink from "../TabDocsLink";
import {
  chatLayout,
  composer,
  composerHint,
  composerHintLink,
  composerInput,
  composerRow,
  conversationDelete,
  conversationItem,
  conversationItemActive,
  conversationRail,
  emptyState,
  emptyStateHint,
  newChatButton,
  railHeading,
  sendButton,
  thread,
  threadDocsLink,
  threadHeader,
  threadHeaderMeta,
  threadHeaderOpen,
  threadHeaderText,
  threadHeaderTitle,
  threadScroller,
} from "./AiChat.css";
import ChatMessage, { type ChatTurnView } from "./ChatMessage";
import type { QueryStep } from "./ChatQueryBlock";

const SUGGESTIONS = [
  "Which landing pages brought in the most revenue last month?",
  "What are my top 10 referrers by converting visitors?",
  "How has monthly recurring revenue changed over the last six months?",
];

/** Bounds on the fitted conversation height, in pixels. */
const MIN_CHAT_HEIGHT = 320;
const MAX_CHAT_HEIGHT = 760;

/** Refresh a guest challenge this many seconds before it expires. */
const CHALLENGE_REFRESH_SKEW_SECONDS = 60;

type DemoChallenge = {
  token: string;
  expiresAt: number;
};

type AiChatProps = {
  projectId: string;
  /** Absent on the public demo when the visitor is not signed in. */
  session?: AuthSession;
};

export default function AiChat(props: AiChatProps) {
  const [turns, setTurns] = createStore<ChatTurnView[]>([]);
  const [conversations, setConversations] = createSignal<ConversationSummary[]>([]);
  const [conversationId, setConversationId] = createSignal<number | undefined>();
  const [draft, setDraft] = createSignal("");
  const [sending, setSending] = createSignal(false);
  const [loadError, setLoadError] = createSignal<string | undefined>();

  let controller: AbortController | undefined;
  let scroller: HTMLDivElement | undefined;
  let layout: HTMLDivElement | undefined;
  // Anonymous demo Ask needs a short-lived server token minted after a real
  // interaction. Kept off reactive state so a mint does not re-render the tab.
  let demoChallenge: DemoChallenge | undefined;
  let challengeInFlight: Promise<string> | undefined;

  onCleanup(() => controller?.abort());

  /**
   * Takes the height left in the viewport so the thread scrolls and the page
   * around it does not. The shells this tab renders in hold themselves at a
   * viewport minimum, which hides any spare room from a plain measurement, so
   * this claims the ceiling first and then gives back whatever the document
   * turns out to overflow by. Working from the overflow means the padding of
   * the surrounding shell never has to be known here.
   */
  const fitToViewport = () => {
    // A hidden tab measures as zero and would fit against another tab's height.
    if (!layout || layout.offsetHeight === 0) {
      return;
    }
    layout.style.height = `${MAX_CHAT_HEIGHT}px`;
    const overflow = document.documentElement.scrollHeight - window.innerHeight;
    // A pixel of slack keeps sub-pixel rounding from bringing the scrollbar
    // back after all this.
    const fitted = MAX_CHAT_HEIGHT - overflow - 1;
    layout.style.height = `${Math.floor(Math.min(MAX_CHAT_HEIGHT, Math.max(MIN_CHAT_HEIGHT, fitted)))}px`;
  };

  onMount(() => {
    fitToViewport();
    window.addEventListener("resize", fitToViewport);
    // The surrounding shell can settle after this tab mounts (filter bars and
    // the tab list load their own data), which moves the conversation down.
    const observer = new ResizeObserver(fitToViewport);
    observer.observe(document.body);
    onCleanup(() => {
      window.removeEventListener("resize", fitToViewport);
      observer.disconnect();
    });
  });

  const refreshConversations = async () => {
    try {
      setConversations(await fetchConversations(props.session, props.projectId));
    } catch (error) {
      setLoadError(error instanceof Error ? error.message : "Unable to load conversations.");
    }
  };

  // Switching projects makes every stored conversation irrelevant: they belong
  // to the project database being left behind.
  createEffect(() => {
    const projectId = props.projectId;
    controller?.abort();
    setTurns([]);
    setConversationId(undefined);
    setLoadError(undefined);
    demoChallenge = undefined;
    challengeInFlight = undefined;
    void projectId;
    void refreshConversations();
  });

  const challengeStillValid = () => {
    if (!demoChallenge) {
      return false;
    }
    return demoChallenge.expiresAt > Date.now() / 1000 + CHALLENGE_REFRESH_SKEW_SECONDS;
  };

  /**
   * Ensures an anonymous visitor has a live challenge before Ask spends tokens.
   * Signed-in callers skip this; the session is enough.
   */
  const ensureDemoChallenge = (): Promise<string | undefined> => {
    if (props.session) {
      return Promise.resolve(undefined);
    }
    if (challengeStillValid() && demoChallenge) {
      return Promise.resolve(demoChallenge.token);
    }
    if (!challengeInFlight) {
      challengeInFlight = fetchDemoChatChallenge()
        .then((challenge) => {
          demoChallenge = challenge;
          return challenge.token;
        })
        .finally(() => {
          challengeInFlight = undefined;
        });
    }
    return challengeInFlight;
  };

  /** Mints (or refreshes) the guest challenge as soon as someone uses the UI. */
  const noteGuestInteraction = () => {
    if (props.session || challengeStillValid() || challengeInFlight) {
      return;
    }
    void ensureDemoChallenge().catch(() => {
      // Ask will surface a mint failure; an idle visitor need not see it yet.
    });
  };

  const scrollToLatest = () => {
    // Queued so the newly rendered text is measured before we scroll past it.
    requestAnimationFrame(() => {
      scroller?.scrollTo({ top: scroller.scrollHeight, behavior: "smooth" });
    });
  };

  /**
   * Bumped by everything that replaces what the transcript shows, so a
   * conversation load that resolves afterwards knows to drop its answer.
   */
  let view = 0;

  const startNewConversation = () => {
    view++;
    controller?.abort();
    setTurns([]);
    setConversationId(undefined);
    setLoadError(undefined);
  };

  const openConversation = async (id: number) => {
    controller?.abort();
    setLoadError(undefined);
    setConversationId(id);
    const opened = ++view;
    try {
      const messages = await fetchMessages(props.session, props.projectId, id);
      // The user may have opened another conversation, or asked, meanwhile.
      if (view !== opened) {
        return;
      }
      setTurns(replayTurns(messages));
      scrollToLatest();
    } catch (error) {
      if (error instanceof ConversationNotFoundError) {
        forgetConversation(id);
      }
      if (view !== opened) {
        return;
      }
      setTurns([]);
      if (error instanceof ConversationNotFoundError) {
        setConversationId(undefined);
      }
      setLoadError(error instanceof Error ? error.message : "Unable to load this conversation.");
    }
  };

  /** Drops a conversation the server says no longer exists from the rail. */
  const forgetConversation = (id: number) => {
    setConversations((current) => current.filter((conversation) => conversation.id !== id));
  };

  const removeConversation = async (id: number) => {
    try {
      await deleteConversation(props.session, props.projectId, id);
    } catch (error) {
      setLoadError(error instanceof Error ? error.message : "Unable to delete this conversation.");
      return;
    }
    setConversations((current) => current.filter((conversation) => conversation.id !== id));
    if (conversationId() === id) {
      startNewConversation();
    }
  };

  const ask = async (question: string) => {
    const trimmed = question.trim();
    if (trimmed === "" || sending()) {
      return;
    }
    setDraft("");
    setSending(true);
    setLoadError(undefined);

    view++;
    let index = turns.length;
    setTurns(
      produce((list) => {
        list.push({
          key: `turn-${Date.now()}`,
          question: trimmed,
          steps: [],
          answer: "",
          pending: true,
        });
      }),
    );
    scrollToLatest();

    controller = new AbortController();
    const askedInto = conversationId();
    try {
      const challengeToken = await ensureDemoChallenge();
      await streamChat(
        props.session,
        props.projectId,
        conversationId(),
        trimmed,
        {
          onConversation: (id) => {
            const isNew = conversationId() === undefined;
            setConversationId(id);
            if (isNew) {
              void refreshConversations();
            }
          },
          onToken: (text) => {
            setTurns(index, "answer", (answer) => answer + text);
            scrollToLatest();
          },
          onToolCall: (call) => {
            setTurns(index, "steps", (steps) => [
              ...steps,
              { id: call.id, title: call.title, sql: call.sql },
            ]);
            scrollToLatest();
          },
          onToolResult: (result) => {
            setTurns(index, "steps", (steps) => attachResult(steps, result));
            scrollToLatest();
          },
          onError: (text) => setTurns(index, "error", text),
        },
        controller.signal,
        challengeToken,
      );
    } catch (error) {
      // Deleted from another tab: start over rather than retry into it.
      // Its history goes too, so the next question doesn't look like it has
      // that context.
      if (error instanceof ConversationNotFoundError && askedInto !== undefined) {
        forgetConversation(askedInto);
        if (conversationId() === askedInto) {
          setConversationId(undefined);
          setTurns((list) => [list[index]]);
          index = 0;
        }
      }
      if (!controller.signal.aborted) {
        setTurns(
          index,
          "error",
          error instanceof Error ? error.message : "The assistant could not be reached.",
        );
      }
    } finally {
      setTurns(index, "pending", false);
      setSending(false);
      scrollToLatest();
    }
  };

  const openedConversation = () =>
    conversations().find((conversation) => conversation.id === conversationId());

  const handleKeyDown = (event: KeyboardEvent) => {
    noteGuestInteraction();
    // Enter sends; Shift+Enter is how you get a second line.
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      void ask(draft());
    }
  };

  return (
    <div class={chatLayout} ref={layout}>
      <aside class={conversationRail}>
        <button type="button" class={newChatButton} onClick={startNewConversation}>
          New chat
        </button>
        <h3 class={railHeading}>Recent</h3>
        <For each={conversations()} fallback={<p class={message}>No conversations yet.</p>}>
          {(conversation) => (
            <div
              class={
                conversation.id === conversationId()
                  ? `${conversationItem} ${conversationItemActive}`
                  : conversationItem
              }
            >
              <button type="button" onClick={() => void openConversation(conversation.id)}>
                {conversation.title || "Untitled"}
              </button>
              <button
                type="button"
                class={conversationDelete}
                aria-label={`Delete ${conversation.title || "conversation"}`}
                onClick={() => void removeConversation(conversation.id)}
              >
                ×
              </button>
            </div>
          )}
        </For>
      </aside>

      <div class={thread}>
        <div class={openedConversation() ? `${threadHeader} ${threadHeaderOpen}` : threadHeader}>
          {/* Conversations are private to the owner key (signed-in user, or
              client IP on the anonymous demo), so the author label is only shown
              when we know who that is. */}
          <Show when={openedConversation()}>
            {(conversation) => (
              <div class={threadHeaderText}>
                <span class={threadHeaderTitle}>{conversation().title || "Untitled"}</span>
                <span class={threadHeaderMeta}>
                  <Show when={props.session} fallback={formatTimestamp(conversation().created_at)}>
                    {(session) => (
                      <>
                        Started by {session().name} · {formatTimestamp(conversation().created_at)}
                      </>
                    )}
                  </Show>
                </span>
              </div>
            )}
          </Show>
          <TabDocsLink href={docsUrl("/docs/ai")} label="AI Docs" class={threadDocsLink} />
        </div>

        <div class={threadScroller} ref={scroller}>
          <Show
            when={turns.length > 0}
            fallback={
              <div class={emptyState}>
                <h3>Ask about this project</h3>
                <p class={message}>
                  Questions are answered by querying your analytics database directly, so every
                  number comes with the query that produced it.
                </p>
                <For each={SUGGESTIONS}>
                  {(suggestion) => (
                    <button
                      type="button"
                      class={emptyStateHint}
                      onPointerDown={noteGuestInteraction}
                      onClick={() => void ask(suggestion)}
                    >
                      {suggestion}
                    </button>
                  )}
                </For>
              </div>
            }
          >
            <For each={turns}>{(turn) => <ChatMessage turn={turn} />}</For>
          </Show>
          <Show when={loadError()}>{(error) => <p class={errorMessage}>{error()}</p>}</Show>
        </div>

        <form
          class={composer}
          onSubmit={(event) => {
            event.preventDefault();
            void ask(draft());
          }}
        >
          <p class={composerHint}>
            Need more power?{" "}
            <a class={composerHintLink} href={docsUrl("/docs/ai")}>
              Connect Claude, ChatGPT, Grok, or other AI tools
            </a>
          </p>
          <div class={composerRow}>
            <textarea
              class={composerInput}
              rows={4}
              placeholder="Ask about traffic, attribution, or revenue…"
              value={draft()}
              onFocus={noteGuestInteraction}
              onPointerDown={noteGuestInteraction}
              onInput={(event) => {
                noteGuestInteraction();
                setDraft(event.currentTarget.value);
              }}
              onKeyDown={handleKeyDown}
            />
            <button type="submit" class={sendButton} disabled={sending() || draft().trim() === ""}>
              {sending() ? "Asking…" : "Ask"}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}

/** Stored timestamps are unix seconds; shown in the reader's own timezone. */
function formatTimestamp(seconds: number): string {
  return new Date(seconds * 1000).toLocaleString(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  });
}

/** Fills in the table for the query it belongs to, matched on the call id. */
function attachResult(steps: QueryStep[], result: ChatToolResult): QueryStep[] {
  const index = steps.findIndex((step) => step.id === result.id);
  if (index === -1) {
    return [...steps, { id: result.id, title: result.title, sql: result.sql, result }];
  }
  return steps.map((step, position) =>
    position === index ? { ...step, title: step.title ?? result.title, result } : step,
  );
}

/**
 * Rebuilds the exchanges from a stored transcript. A user row opens a turn and
 * everything after it belongs to that turn, which is the same shape the live
 * stream produces.
 */
function replayTurns(messages: Awaited<ReturnType<typeof fetchMessages>>): ChatTurnView[] {
  const turns: ChatTurnView[] = [];
  for (const stored of messages) {
    if (stored.role === "user") {
      turns.push({
        key: `stored-${stored.id}`,
        question: stored.content,
        steps: [],
        answer: "",
      });
      continue;
    }
    const turn = turns[turns.length - 1];
    if (!turn) {
      continue;
    }
    if (stored.role === "assistant") {
      turn.answer = turn.answer === "" ? stored.content : `${turn.answer}\n\n${stored.content}`;
    } else if (stored.tool_result) {
      turn.steps.push({
        id: `stored-${stored.id}`,
        title: stored.tool_result.title,
        sql: stored.tool_sql ?? "",
        result: stored.tool_result,
      });
    }
  }
  return turns;
}
