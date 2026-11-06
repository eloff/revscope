import { describe, expect, it } from "vitest";
import { parseBlocks, parseInline } from "./markdownLite";

const text = (value: string) => ({ kind: "text", text: value }) as const;

describe("parseInline", () => {
  it("splits bold and code spans", () => {
    expect(parseInline("Revenue **rose** by `12%` today")).toEqual([
      text("Revenue "),
      { kind: "bold", text: "rose" },
      text(" by "),
      { kind: "code", text: "12%" },
      text(" today"),
    ]);
  });

  it("leaves unclosed markers literal while an answer streams", () => {
    expect(parseInline("half **bold and `code")).toEqual([text("half **bold and `code")]);
  });

  it("keeps markup as text rather than HTML", () => {
    expect(parseInline("<img src=x onerror=alert(1)>")).toEqual([
      text("<img src=x onerror=alert(1)>"),
    ]);
  });
});

describe("parseBlocks", () => {
  it("joins wrapped lines into paragraphs split by blank lines", () => {
    expect(parseBlocks("one\ntwo\n\nthree")).toEqual([
      { kind: "paragraph", spans: [text("one two")] },
      { kind: "paragraph", spans: [text("three")] },
    ]);
  });

  it("parses bullet and numbered lists", () => {
    expect(parseBlocks("Intro\n- a\n* b\n\n1. x\n2) y")).toEqual([
      { kind: "paragraph", spans: [text("Intro")] },
      { kind: "list", ordered: false, items: [[text("a")], [text("b")]] },
      { kind: "list", ordered: true, items: [[text("x")], [text("y")]] },
    ]);
  });

  it("keeps fenced code verbatim, even unterminated", () => {
    expect(parseBlocks("```sql\nselect **1**\n  from t\n```\nafter")).toEqual([
      { kind: "code", text: "select **1**\n  from t" },
      { kind: "paragraph", spans: [text("after")] },
    ]);
    expect(parseBlocks("```\nstill streaming")).toEqual([
      { kind: "code", text: "still streaming" },
    ]);
  });

  it("parses a pipe table with alignments, escaped pipes and ragged rows", () => {
    const [table, after] = parseBlocks(
      "| Page | Views | Share |\n|:---|---:|:-:|\n| /a\\|b | 10 | 5% | extra |\n| /c |\n\nDone",
    );
    expect(table).toEqual({
      kind: "table",
      alignments: ["left", "right", "center"],
      header: [[text("Page")], [text("Views")], [text("Share")]],
      rows: [
        [[text("/a|b")], [text("10")], [text("5%")]],
        [[text("/c")], [], []],
      ],
    });
    expect(after).toEqual({ kind: "paragraph", spans: [text("Done")] });
  });

  it("treats a pipe without a divider row as prose", () => {
    expect(parseBlocks("cats | dogs\nare both pets")).toEqual([
      { kind: "paragraph", spans: [text("cats | dogs are both pets")] },
    ]);
  });

  it("keeps a header as prose until its divider matches the column count", () => {
    expect(parseBlocks("| a | b |\n|---|")[0]?.kind).toBe("paragraph");
    expect(parseBlocks("| a | b |")[0]?.kind).toBe("paragraph");
  });
});
