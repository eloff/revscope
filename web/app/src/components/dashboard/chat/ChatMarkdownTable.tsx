import { For, type JSX } from "solid-js";
import {
  markdownCellCenter,
  markdownCellRight,
  markdownInlineCode,
  markdownTable,
  markdownTableScroller,
} from "./ChatMarkdown.css";
import type { Block, ColumnAlign, InlineSpan } from "./markdownLite";

type MarkdownTableProps = {
  block: Extract<Block, { kind: "table" }>;
};

export default function MarkdownTable(props: MarkdownTableProps) {
  const align = (index: number) => props.block.alignments[index];
  return (
    <div class={markdownTableScroller}>
      <table class={markdownTable}>
        <thead>
          <tr>
            <For each={props.block.header}>
              {(cell, index) => <th class={columnClass(align(index()))}>{renderSpans(cell)}</th>}
            </For>
          </tr>
        </thead>
        <tbody>
          <For each={props.block.rows}>
            {(row) => (
              <tr>
                <For each={row}>
                  {(cell, index) => (
                    <td class={columnClass(align(index()))}>{renderSpans(cell)}</td>
                  )}
                </For>
              </tr>
            )}
          </For>
        </tbody>
      </table>
    </div>
  );
}

function columnClass(align: ColumnAlign | undefined): string | undefined {
  if (align === "right") {
    return markdownCellRight;
  }
  if (align === "center") {
    return markdownCellCenter;
  }
  return undefined;
}

export function renderSpans(spans: InlineSpan[]): JSX.Element {
  return <For each={spans}>{(span) => renderSpan(span)}</For>;
}

function renderSpan(span: InlineSpan): JSX.Element {
  switch (span.kind) {
    case "bold":
      return <strong>{span.text}</strong>;
    case "code":
      return <code class={markdownInlineCode}>{span.text}</code>;
    default:
      return <>{span.text}</>;
  }
}
