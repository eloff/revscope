import { For, Show } from "solid-js";
import ChatMarkdown from "./ChatMarkdown";
import { assistantTurn, pendingDot, turnLabel, userBubble, userTurn } from "./ChatMessage.css";
import ChatQueryBlock, { type QueryStep } from "./ChatQueryBlock";

/**
 * One exchange: the question, the queries run to answer it, and the answer.
 * `answer` grows while the turn streams, so an in-flight turn is one whose
 * `pending` flag is set rather than a separate kind of thing.
 */
export type ChatTurnView = {
  key: string;
  question: string;
  steps: QueryStep[];
  answer: string;
  error?: string;
  pending?: boolean;
};

type ChatMessageProps = {
  turn: ChatTurnView;
};

export default function ChatMessage(props: ChatMessageProps) {
  const showThinking = () =>
    props.turn.pending === true && props.turn.answer === "" && props.turn.steps.length === 0;

  return (
    <div>
      <div class={userTurn}>
        <p class={userBubble}>{props.turn.question}</p>
      </div>

      <div class={assistantTurn}>
        <Show when={showThinking()}>
          <p class={turnLabel}>
            <span class={pendingDot} aria-hidden="true" />
            Thinking…
          </p>
        </Show>

        <For each={props.turn.steps}>{(step) => <ChatQueryBlock step={step} />}</For>

        <Show when={props.turn.answer !== ""}>
          <ChatMarkdown text={props.turn.answer} />
        </Show>

        <Show when={props.turn.error}>{(error) => <p class={turnLabel}>{error()}</p>}</Show>
      </div>
    </div>
  );
}
