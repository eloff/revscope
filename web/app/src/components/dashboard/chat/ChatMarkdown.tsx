import { createMemo, For, type JSX } from "solid-js";
import { markdownCode, markdownList, markdownParagraph } from "./ChatMarkdown.css";
import MarkdownTable, { renderSpans } from "./ChatMarkdownTable";
import { type Block, parseBlocks } from "./markdownLite";

type ChatMarkdownProps = {
  text: string;
};

export default function ChatMarkdown(props: ChatMarkdownProps) {
  const blocks = createMemo(() => parseBlocks(props.text));
  return <For each={blocks()}>{(block) => renderBlock(block)}</For>;
}

function renderBlock(block: Block): JSX.Element {
  switch (block.kind) {
    case "code":
      return <pre class={markdownCode}>{block.text}</pre>;
    case "list": {
      const items = <For each={block.items}>{(item) => <li>{renderSpans(item)}</li>}</For>;
      return block.ordered ? (
        <ol class={markdownList}>{items}</ol>
      ) : (
        <ul class={markdownList}>{items}</ul>
      );
    }
    case "table":
      return <MarkdownTable block={block} />;
    default:
      return <p class={markdownParagraph}>{renderSpans(block.spans)}</p>;
  }
}
