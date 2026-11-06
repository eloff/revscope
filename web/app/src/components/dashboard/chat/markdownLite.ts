/**
 * The small slice of Markdown the assistant actually produces: paragraphs,
 * bullet and numbered lists, fenced code, pipe tables, plus bold and inline
 * code.
 *
 * Parsing to a structure the renderer walks (rather than to an HTML string)
 * means model output is never handed to `innerHTML`, so a table cell echoing
 * visitor-supplied markup cannot become script.
 */

export type InlineSpan =
  | { kind: "text"; text: string }
  | { kind: "bold"; text: string }
  | { kind: "code"; text: string };

/** Taken from the divider row: `---:` right, `:---:` center, otherwise left. */
export type ColumnAlign = "left" | "center" | "right";

export type Block =
  | { kind: "paragraph"; spans: InlineSpan[] }
  | { kind: "list"; ordered: boolean; items: InlineSpan[][] }
  | { kind: "code"; text: string }
  | { kind: "table"; alignments: ColumnAlign[]; header: InlineSpan[][]; rows: InlineSpan[][][] };

const FENCE = "```";
const BULLET = /^\s*[-*]\s+(.*)$/;
const NUMBERED = /^\s*\d+[.)]\s+(.*)$/;

export function parseBlocks(source: string): Block[] {
  const lines = source.split("\n");
  const blocks: Block[] = [];
  let paragraph: string[] = [];

  const flushParagraph = () => {
    if (paragraph.length > 0) {
      blocks.push({ kind: "paragraph", spans: parseInline(paragraph.join(" ")) });
      paragraph = [];
    }
  };

  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index] ?? "";

    if (line.trimStart().startsWith(FENCE)) {
      flushParagraph();
      const code: string[] = [];
      index += 1;
      while (index < lines.length && !(lines[index] ?? "").trimStart().startsWith(FENCE)) {
        code.push(lines[index] ?? "");
        index += 1;
      }
      blocks.push({ kind: "code", text: code.join("\n") });
      continue;
    }

    const table = parseTable(lines, index);
    if (table) {
      flushParagraph();
      blocks.push(table.block);
      index = table.nextIndex - 1;
      continue;
    }

    const bullet = BULLET.exec(line);
    const numbered = bullet ? null : NUMBERED.exec(line);
    if (bullet || numbered) {
      flushParagraph();
      const ordered = numbered !== null;
      const items: InlineSpan[][] = [];
      while (index < lines.length) {
        const current = lines[index] ?? "";
        const match = ordered ? NUMBERED.exec(current) : BULLET.exec(current);
        if (!match) {
          break;
        }
        items.push(parseInline(match[1] ?? ""));
        index += 1;
      }
      index -= 1;
      blocks.push({ kind: "list", ordered, items });
      continue;
    }

    if (line.trim() === "") {
      flushParagraph();
      continue;
    }
    paragraph.push(line.trim());
  }

  flushParagraph();
  return blocks;
}

const DIVIDER_CELL = /^:?-+:?$/;

/**
 * A pipe table is a row of cells, a divider (`|---|---:|`), then body rows.
 * The divider is required, so a sentence that merely contains `|` stays prose.
 * A header whose divider has not arrived yet (the answer is still streaming)
 * stays a paragraph until the divider is complete.
 */
function parseTable(
  lines: string[],
  start: number,
): { block: Extract<Block, { kind: "table" }>; nextIndex: number } | null {
  const headerLine = lines[start] ?? "";
  const dividerLine = lines[start + 1];
  if (dividerLine === undefined || !headerLine.includes("|") || !dividerLine.includes("|")) {
    return null;
  }
  const alignments = parseDivider(dividerLine);
  const header = splitRow(headerLine);
  if (!alignments || header.length !== alignments.length) {
    return null;
  }

  const rows: string[][] = [];
  let cursor = start + 2;
  while (cursor < lines.length) {
    const line = lines[cursor] ?? "";
    if (line.trim() === "" || !line.includes("|")) {
      break;
    }
    rows.push(fitColumns(splitRow(line), alignments.length));
    cursor += 1;
  }

  return {
    block: {
      kind: "table",
      alignments,
      header: header.map((cell) => parseInline(cell)),
      rows: rows.map((row) => row.map((cell) => parseInline(cell))),
    },
    nextIndex: cursor,
  };
}

/** Column alignments, or null when the line is not a divider. */
function parseDivider(line: string): ColumnAlign[] | null {
  const cells = splitRow(line);
  if (cells.length === 0) {
    return null;
  }
  const alignments: ColumnAlign[] = [];
  for (const cell of cells) {
    if (!DIVIDER_CELL.test(cell)) {
      return null;
    }
    const left = cell.startsWith(":");
    const right = cell.endsWith(":");
    if (left && right) {
      alignments.push("center");
    } else if (right) {
      alignments.push("right");
    } else {
      alignments.push("left");
    }
  }
  return alignments;
}

/** Splits a row on `|`. A leading or trailing pipe is the border, and `\|` is a literal pipe. */
function splitRow(line: string): string[] {
  let text = line.trim();
  if (text.startsWith("|")) {
    text = text.slice(1);
  }
  if (text.endsWith("|") && !text.endsWith("\\|")) {
    text = text.slice(0, -1);
  }

  const cells: string[] = [];
  let cell = "";
  for (let index = 0; index < text.length; index += 1) {
    const char = text[index] ?? "";
    if (char === "\\" && text[index + 1] === "|") {
      cell += "|";
      index += 1;
      continue;
    }
    if (char === "|") {
      cells.push(cell.trim());
      cell = "";
      continue;
    }
    cell += char;
  }
  cells.push(cell.trim());
  return cells;
}

function fitColumns(cells: string[], count: number): string[] {
  const fitted = cells.slice(0, count);
  while (fitted.length < count) {
    fitted.push("");
  }
  return fitted;
}

/**
 * Splits on `**bold**` and `` `code` ``. Unclosed markers stay literal so a
 * half-streamed sentence reads as the model wrote it.
 */
export function parseInline(source: string): InlineSpan[] {
  const spans: InlineSpan[] = [];
  let text = "";
  let index = 0;

  const flushText = () => {
    if (text.length > 0) {
      spans.push({ kind: "text", text });
      text = "";
    }
  };

  while (index < source.length) {
    if (source.startsWith("**", index)) {
      const end = source.indexOf("**", index + 2);
      if (end !== -1) {
        flushText();
        spans.push({ kind: "bold", text: source.slice(index + 2, end) });
        index = end + 2;
        continue;
      }
    }
    if (source[index] === "`") {
      const end = source.indexOf("`", index + 1);
      if (end !== -1) {
        flushText();
        spans.push({ kind: "code", text: source.slice(index + 1, end) });
        index = end + 1;
        continue;
      }
    }
    text += source[index];
    index += 1;
  }

  flushText();
  return spans;
}
