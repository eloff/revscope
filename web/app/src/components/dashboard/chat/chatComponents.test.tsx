import { render, screen, within } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import ChatMarkdown from "./ChatMarkdown";
import ChatMessage from "./ChatMessage";
import ChatQueryBlock from "./ChatQueryBlock";
import ChatResultTable from "./ChatResultTable";

describe("ChatMarkdown", () => {
  it("renders paragraphs, lists, code and inline marks", () => {
    const { container } = render(() => (
      <ChatMarkdown text={"Revenue **rose** by `12%`\n\n1. one\n2. two\n\n```\nselect 1\n```"} />
    ));
    expect(screen.getByText("rose").tagName).toBe("STRONG");
    expect(screen.getByText("12%").tagName).toBe("CODE");
    expect(container.querySelectorAll("ol li")).toHaveLength(2);
    expect(container.querySelector("pre")).toHaveTextContent("select 1");
  });

  it("renders a table with aligned columns", () => {
    render(() => <ChatMarkdown text={"| Page | Views |\n|---|---:|\n| /a | 10 |"} />);
    const table = screen.getByRole("table");
    expect(
      within(table)
        .getAllByRole("columnheader")
        .map((cell) => cell.textContent),
    ).toEqual(["Page", "Views"]);
    const [left, right] = within(table).getAllByRole("cell");
    expect(left?.className).toBe("");
    expect(right?.className).not.toBe("");
  });

  it("never turns model text into markup", () => {
    const { container } = render(() => <ChatMarkdown text={"<img src=x onerror=alert(1)>"} />);
    expect(container.querySelector("img")).toBeNull();
    expect(container).toHaveTextContent("<img src=x onerror=alert(1)>");
  });
});

describe("ChatResultTable", () => {
  it("humanizes headers, right-aligns numeric columns and formats cells", () => {
    render(() => (
      <ChatResultTable
        result={{
          id: "q",
          sql: "",
          columns: ["landing_path", "visitors"],
          rows: [
            ["/a", 1234],
            [null, null],
          ],
          has_more: false,
        }}
      />
    ));
    const headers = screen.getAllByRole("columnheader");
    expect(headers.map((cell) => cell.textContent)).toEqual(["landing path", "visitors"]);
    expect(headers[0]?.className).toBe("");
    expect(headers[1]?.className).not.toBe("");
    expect(screen.getByText((1234).toLocaleString())).toBeInTheDocument();
    expect(screen.getAllByText("—")).toHaveLength(2);
    expect(screen.getByText("2 rows")).toBeInTheDocument();
  });

  it("says when more rows were available, and caps what it shows", () => {
    const rows = Array.from({ length: 60 }, (_, index) => [index]);
    render(() => (
      <ChatResultTable result={{ id: "q", sql: "", columns: ["n"], rows, has_more: true }} />
    ));
    expect(screen.getAllByRole("row")).toHaveLength(51);
    expect(
      screen.getByText("60 rows, and more were available past the limit (showing the first 50)"),
    ).toBeInTheDocument();
  });

  it("reports an empty result", () => {
    render(() => (
      <ChatResultTable result={{ id: "q", sql: "", columns: ["n"], rows: [], has_more: false }} />
    ));
    expect(screen.getByText("No rows matched.")).toBeInTheDocument();
    expect(screen.queryByRole("table")).toBeNull();
  });
});

describe("ChatQueryBlock", () => {
  it.each([
    [{ id: "1", sql: "s" }, "Running query…"],
    [{ id: "1", title: "Top pages", sql: "s" }, "Top pages…"],
    [
      { id: "1", title: "Top pages", sql: "s", result: { id: "1", sql: "s", error: "boom" } },
      "Top pages (failed)",
    ],
    [{ id: "1", sql: "s", result: { id: "1", sql: "s", error: "boom" } }, "Query failed"],
    [
      { id: "1", sql: "s", result: { id: "1", sql: "s", columns: [], rows: [], has_more: false } },
      "Query",
    ],
  ])("labels %j as %s", (step, label) => {
    render(() => <ChatQueryBlock step={step} />);
    expect(screen.getByText(label)).toBeInTheDocument();
  });

  it("keeps the SQL collapsed until asked and shows a query error", async () => {
    render(() => (
      <ChatQueryBlock
        step={{
          id: "1",
          sql: "select boom",
          result: { id: "1", sql: "select boom", error: "no such column" },
        }}
      />
    ));
    expect(screen.getByText("no such column")).toBeInTheDocument();
    const trigger = screen.getByRole("button", { name: "Query failed" });
    expect(trigger).toHaveAttribute("aria-expanded", "false");
    await userEvent.click(trigger);
    expect(trigger).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("select boom")).toBeVisible();
  });
});

describe("ChatMessage", () => {
  it("shows thinking until the first step or token arrives", () => {
    render(() => (
      <ChatMessage
        turn={{ key: "k", question: "How many?", steps: [], answer: "", pending: true }}
      />
    ));
    expect(screen.getByText("How many?")).toBeInTheDocument();
    expect(screen.getByText("Thinking…")).toBeInTheDocument();
  });

  it("shows the answer and an error without the thinking line", () => {
    render(() => (
      <ChatMessage
        turn={{
          key: "k",
          question: "q",
          steps: [],
          answer: "About **12**",
          error: "Cut short.",
          pending: true,
        }}
      />
    ));
    expect(screen.queryByText("Thinking…")).toBeNull();
    expect(screen.getByText("12").tagName).toBe("STRONG");
    expect(screen.getByText("Cut short.")).toBeInTheDocument();
  });
});
