import { Collapsible } from "@ark-ui/solid/collapsible";
import { Show } from "solid-js";
import type { ChatToolResult } from "../../../lib/chatApi";
import { queryBlock, queryError, querySql, queryStatus, queryTrigger } from "./ChatQueryBlock.css";
import ChatResultTable from "./ChatResultTable";

export type QueryStep = {
  id: string;
  /** Absent for queries stored before titles existed. */
  title?: string | null;
  sql: string;
  /** Absent while the query is still running. */
  result?: ChatToolResult;
};

type ChatQueryBlockProps = {
  step: QueryStep;
};

export default function ChatQueryBlock(props: ChatQueryBlockProps) {
  const status = () => {
    const title = props.step.title;
    if (!props.step.result) {
      return title ? `${title}…` : "Running query…";
    }
    if (props.step.result.error) {
      return title ? `${title} (failed)` : "Query failed";
    }
    return title ?? "Query";
  };

  return (
    <div class={queryBlock}>
      {/* The SQL is collapsed by default: it is evidence for the answer above
          it, not the answer itself. */}
      <Collapsible.Root>
        <Collapsible.Trigger class={queryTrigger}>
          <span class={queryStatus}>{status()}</span>
          <span aria-hidden="true">Show SQL</span>
        </Collapsible.Trigger>
        <Collapsible.Content>
          <pre class={querySql}>{props.step.sql}</pre>
        </Collapsible.Content>
      </Collapsible.Root>

      <Show when={props.step.result}>
        {(result) => (
          <Show when={!result().error} fallback={<p class={queryError}>{result().error}</p>}>
            <ChatResultTable result={result()} />
          </Show>
        )}
      </Show>
    </div>
  );
}
