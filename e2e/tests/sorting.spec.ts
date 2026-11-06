import { expect, type TabParam, test } from "./support/demo";
import { parseValue } from "./support/values";

type SortCase = {
  query: string;
  tab: string;
  /** Numeric columns only: text ordering depends on the database's collation. */
  columns: string[];
  /** A column only this grid has, when the tab holds another. */
  gridColumn?: string;
};

const tabQuery = (tab: TabParam, extra = "") => `?tab=${tab}${extra}`;

const SORT_CASES: SortCase[] = [
  {
    query: tabQuery("pages"),
    tab: "Pages",
    gridColumn: "Exit rate",
    columns: ["Visitors", "Attr. revenue", "Exit rate", "Avg duration"],
  },
  {
    query: tabQuery("pages", "&pages=landing"),
    tab: "Pages",
    gridColumn: "Bounce rate",
    columns: ["Visitors", "Bounce rate"],
  },
  { query: tabQuery("revenue_sources"), tab: "Sources", columns: ["Visitors", "Revenue"] },
  // Not Clicks or Est. revenue: descending, those deliberately promote highlighted terms to the top.
  {
    query: tabQuery("search_terms"),
    tab: "Search terms",
    columns: ["Position", "Impressions", "CTR"],
  },
  {
    query: tabQuery("audience"),
    tab: "Audience",
    gridColumn: "Country",
    columns: ["Visitors", "Revenue"],
  },
  { query: tabQuery("audience"), tab: "Audience", gridColumn: "Browser", columns: ["Visitors"] },
  {
    query: tabQuery("visitors"),
    tab: "Visitors",
    gridColumn: "Pages Viewed",
    columns: ["Pages Viewed", "Active Duration"],
  },
  { query: tabQuery("crawlers"), tab: "Crawlers", columns: ["Views"] },
  { query: tabQuery("events"), tab: "Events", columns: ["Visitors", "Conversions"] },
];

test.describe("grid sorting", () => {
  for (const { query, tab, columns, gridColumn } of SORT_CASES) {
    for (const column of columns) {
      test(`${tab} ${gridColumn ?? ""} sorts by ${column} both ways`, async ({ demo }) => {
        await demo.goto(query);
        const grid = demo.grid(tab, gridColumn ?? column);
        await grid.expectRows();

        await grid.sortBy(column, "descending");
        await grid.expectSorted(column, "descending");

        await grid.sortBy(column, "ascending");
        await grid.expectSorted(column, "ascending");

        // Exactly one column claims the sort.
        await expect(
          grid.root.locator('thead th[aria-sort="ascending"], thead th[aria-sort="descending"]'),
        ).toHaveCount(1);
      });
    }
  }

  test("the chosen sort survives a reload", async ({ demo, page }) => {
    await demo.goto("?tab=pages");
    const grid = demo.grid("Pages", "Exit rate");
    await grid.expectRows();
    await grid.sortBy("Avg duration", "ascending");

    await page.reload();

    await expect(grid.sortState("Avg duration")).toHaveAttribute("aria-sort", "ascending");
    await grid.expectRows();
    await grid.expectSorted("Avg duration", "ascending");
  });
});

test.describe("Overview breakdown sorting", () => {
  // The cells abbreviate; each row's title has the exact figures.
  const COLUMNS = [
    { header: "Visitors", figure: /^([\d,]+) visitors/m },
    { header: "Revenue", figure: /· (\S+) revenue$/m },
  ];

  for (const { header, figure } of COLUMNS) {
    test(`a breakdown sorts by ${header} both ways`, async ({ demo }) => {
      await demo.goto();
      const table = demo.panel("Overview").getByRole("tabpanel", { name: "Country", exact: true });
      const rows = table.locator("button[aria-pressed]");
      await expect(rows.nth(1)).toBeVisible();
      const button = table.getByRole("button", { name: new RegExp(`^${header}\\b`) });
      const values = async () =>
        (await rows.evaluateAll((all) => all.map((row) => row.getAttribute("title") ?? ""))).map(
          (title) => parseValue(figure.exec(title)?.[1] ?? "?") as number,
        );

      for (const direction of ["descending", "ascending"] as const) {
        await expect(async () => {
          if (!(await button.innerText()).includes(direction)) {
            await button.click();
          }
          await expect(button).toContainText(`sorted ${direction}`, { timeout: 1_000 });
        }).toPass();
        const ordered = await values();
        expect(ordered).toEqual(
          ordered.toSorted((a, b) => (direction === "ascending" ? a - b : b - a)),
        );
      }
    });
  }
});

test.describe("pagination", () => {
  test("Next and Previous page through visitors", async ({ demo }) => {
    await demo.goto("?tab=visitors");
    const panel = demo.panel("Visitors");
    const grid = demo.grid("Visitors", "Pages Viewed");
    await grid.expectRows();
    const next = panel.getByRole("button", { name: "Next", exact: true }).first();
    const previous = panel.getByRole("button", { name: "Previous", exact: true }).first();
    await expect(previous).toBeDisabled();
    await expect(next).toBeEnabled();

    const journey = grid.root.getByRole("button", { name: /^View journey for visitor / });
    const firstPageTop = await journey.first().getAttribute("aria-label");

    await next.click();
    await expect(journey.first()).not.toHaveAttribute("aria-label", firstPageTop ?? "");
    await expect(previous).toBeEnabled();

    await previous.click();
    await expect(journey.first()).toHaveAttribute("aria-label", firstPageTop ?? "");
    await expect(previous).toBeDisabled();
  });
});
