import { For, Show } from "solid-js";
import type { ChatToolResult } from "../../../lib/chatApi";
import { gridNumberCell, gridShell, gridTable } from "../grid.css";
import { resultCaption, resultCell, resultScroller } from "./ChatResultTable.css";

/** Rows shown before the table scrolls rather than growing the conversation. */
const VISIBLE_ROW_LIMIT = 50;

type ChatResultTableProps = {
  result: ChatToolResult;
};

export default function ChatResultTable(props: ChatResultTableProps) {
  const columns = () => props.result.columns ?? [];
  const rows = () => props.result.rows ?? [];

  const caption = () => {
    const count = rows().length;
    if (count === 0) {
      return "No rows matched.";
    }
    const noun = count === 1 ? "row" : "rows";
    return props.result.has_more
      ? `${count.toLocaleString()} ${noun}, and more were available past the limit`
      : `${count.toLocaleString()} ${noun}`;
  };

  return (
    <div class={gridShell}>
      <Show when={rows().length > 0} fallback={<p class={resultCaption}>{caption()}</p>}>
        <div class={resultScroller}>
          <table class={gridTable}>
            <thead>
              <tr>
                <For each={columns()}>
                  {(column, index) => (
                    <th class={isNumericColumn(rows(), index()) ? gridNumberCell : undefined}>
                      {column.replaceAll("_", " ")}
                    </th>
                  )}
                </For>
              </tr>
            </thead>
            <tbody>
              <For each={rows().slice(0, VISIBLE_ROW_LIMIT)}>
                {(row) => (
                  <tr>
                    <For each={columns()}>
                      {(_column, index) => (
                        <td
                          class={`${resultCell} ${
                            isNumericColumn(rows(), index()) ? gridNumberCell : ""
                          }`}
                        >
                          {formatCell(row[index()])}
                        </td>
                      )}
                    </For>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
        </div>
        <p class={resultCaption}>
          {caption()}
          <Show when={rows().length > VISIBLE_ROW_LIMIT}>
            {` (showing the first ${VISIBLE_ROW_LIMIT})`}
          </Show>
        </p>
      </Show>
    </div>
  );
}

/** Right-align a column only when every value present in it is a number. */
function isNumericColumn(rows: ChatToolResult["rows"], index: number): boolean {
  if (!rows || rows.length === 0) {
    return false;
  }
  let sawNumber = false;
  for (const row of rows) {
    const value = row[index];
    if (value === null || value === undefined) {
      continue;
    }
    if (typeof value !== "number") {
      return false;
    }
    sawNumber = true;
  }
  return sawNumber;
}

function formatCell(value: string | number | boolean | null | undefined): string {
  if (value === null || value === undefined) {
    return "—";
  }
  if (typeof value === "number") {
    return value.toLocaleString();
  }
  return String(value);
}
