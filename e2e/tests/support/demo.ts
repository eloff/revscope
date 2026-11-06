import { test as base, expect, type Locator, type Page } from "@playwright/test";
import { parsePickerDate, parseValue } from "./values";

/**
 * The demo's data is regenerated nightly and differs between environments, so
 * these tests assert relations rather than values: a column is ordered, a filter
 * narrows a total, a wider range raises it. They never assert a specific number,
 * name, or date, and they don't expect data in very recent periods, since a demo
 * database can be a day old. They do need a demo shaped like the generated one:
 * traffic older than 30 days, spread over several countries, pages and sources.
 */

export const DEMO_TABS = [
  { label: "Overview", param: "overview" },
  { label: "Revenue", param: "revenue" },
  { label: "Sources", param: "revenue_sources" },
  { label: "Campaigns", param: "campaigns" },
  { label: "Search terms", param: "search_terms" },
  { label: "Audience", param: "audience" },
  { label: "Pages", param: "pages" },
  { label: "Visitors", param: "visitors" },
  { label: "Crawlers", param: "crawlers" },
  { label: "Events", param: "events" },
  { label: "Funnels", param: "funnels" },
] as const;

export type DemoTab = (typeof DEMO_TABS)[number];
export type TabParam = DemoTab["param"];

const READ_ONLY_METHODS = new Set(["GET", "HEAD", "OPTIONS"]);

/** Production's own analytics. A test run isn't a visitor, so these are dropped without complaint. */
function isTelemetry(url: URL): boolean {
  return url.hostname === "in.revscope.co" || url.pathname.startsWith("/cdn-cgi/rum");
}

function isExpectedApiFailure(url: URL, status: number): boolean {
  // How an anonymous visitor's session check comes back on some origins.
  return status === 401 && url.pathname === "/v2/session/me";
}

export const test = base.extend<{ demo: DemoDashboard; guard: undefined }>({
  // These run against production too, so a page must not be able to write, and must
  // not spend money: Ask AI posts a question to a paid model. This sees what the
  // browser sends, not page.request or context.request, so tests must not use those.
  guard: [
    async ({ context }, use) => {
      const problems: string[] = [];
      await context.route("**/*", async (route) => {
        const request = route.request();
        if (isTelemetry(new URL(request.url()))) {
          await route.abort("blockedbyclient");
          return;
        }
        if (READ_ONLY_METHODS.has(request.method())) {
          await route.continue();
          return;
        }
        problems.push(`blocked ${request.method()} ${request.url()}`);
        await route.abort("blockedbyclient");
      });
      context.on("weberror", (error) => {
        problems.push(`uncaught: ${error.error().message}`);
      });
      context.on("response", (response) => {
        const url = new URL(response.url());
        if (
          url.pathname.startsWith("/v2/") &&
          response.status() >= 400 &&
          !isExpectedApiFailure(url, response.status())
        ) {
          problems.push(`${response.status()} ${url.pathname}`);
        }
      });
      await use(undefined);
      expect(problems, "uncaught errors, failed API calls or blocked writes").toEqual([]);
    },
    { auto: true },
  ],
  demo: async ({ page }, use) => {
    await use(new DemoDashboard(page));
  },
});

export type { Locator };
export { expect };

/** Grids put their loading and empty messages in a row, so a row alone doesn't mean data. */
const PLACEHOLDER_ROW =
  /^(Loading.*…|No [\w ]+ (for|in) this period\b.*|No [\w ]+ match this filter)$/;

export async function expectDataRows(rows: Locator): Promise<void> {
  await expect(rows.first(), "grid rows").toBeVisible();
  await expect(rows.first()).not.toHaveText(PLACEHOLDER_ROW);
}

export type SortDirection = "ascending" | "descending";

export class DemoDashboard {
  constructor(readonly page: Page) {}

  /** The base URL can carry a path (`/ui` on a self-hosted build), so paths stay relative. */
  async goto(query = ""): Promise<void> {
    await this.page.goto(`demo/${query}`);
    await expect(this.tab("Overview")).toBeVisible();
  }

  tab(label: string): Locator {
    return this.page.getByRole("tablist").first().getByRole("tab", { name: label, exact: true });
  }

  panel(label: string): Locator {
    return this.page.getByRole("tabpanel", { name: label, exact: true }).first();
  }

  param(name: string): string | null {
    return new URL(this.page.url()).searchParams.get(name);
  }

  async expectParam(name: string, value: string | null): Promise<void> {
    await expect.poll(() => this.param(name), `URL param ${name}`).toBe(value);
  }

  /** A grid, picked by a column only it has: mode switches keep both modes' grids mounted. */
  grid(tabLabel: string, column: string): DataGrid {
    return new DataGrid(
      this.panel(tabLabel)
        .locator("table")
        .filter({
          has: this.page.locator("thead").getByRole("button", { name: column, exact: true }),
        }),
    );
  }

  /** Modes are radios styled as a segmented control; the label is what takes the click. */
  async chooseMode(panel: Locator, label: string): Promise<void> {
    await panel
      .locator("label")
      .filter({ hasText: new RegExp(`^${label}$`) })
      .click();
  }

  /** A tile in the Overview metrics bar: its name is the label followed by the value. */
  metricTile(label: string): Locator {
    return this.panel("Overview").getByRole("button", { name: new RegExp(`^${label} [\\d$—]`) });
  }

  async metric(label: string): Promise<number | null> {
    const lines = (await this.metricTile(label).innerText()).split("\n");
    return parseValue(lines[1] ?? "");
  }

  /** Waits out the loading placeholder. */
  async loadedMetric(label: string): Promise<number> {
    await expect.poll(() => this.metric(label), `${label} tile to load`).not.toBeNull();
    return (await this.metric(label)) as number;
  }

  /** A filter that matches nothing isn't a narrowed total, so zero doesn't count. */
  async expectMetricNarrowedFrom(label: string, total: number): Promise<void> {
    await expect
      .poll(async () => {
        const value = await this.metric(label);
        return value !== null && value > 0 && value < total;
      }, `${label} between 0 and ${total}`)
      .toBe(true);
  }

  /** `name` is a regex source for the label, or a plain label. */
  filterChip(name: string | RegExp): Locator {
    const source = typeof name === "string" ? RegExp.escape(name) : name.source;
    return this.page.getByRole("button", { name: new RegExp(`^Remove filter: ${source}$`) });
  }

  get dateInputs(): Locator {
    return this.page.getByPlaceholder("mm/dd/yyyy");
  }

  async dateRange(): Promise<{ start: number; end: number }> {
    await expect(this.dateInputs.first()).not.toHaveValue("");
    return {
      start: parsePickerDate(await this.dateInputs.nth(0).inputValue()),
      end: parsePickerDate(await this.dateInputs.nth(1).inputValue()),
    };
  }

  async choosePreset(label: string): Promise<void> {
    await this.dateInputs.first().click();
    await this.page
      .getByRole("listbox", { name: "Date range presets" })
      .getByRole("option", {
        name: label,
      })
      .click();
    await expect(this.page.getByRole("listbox", { name: "Date range presets" })).toBeHidden();
  }
}

export class DataGrid {
  constructor(readonly root: Locator) {}

  get rows(): Locator {
    return this.root.locator("tbody > tr");
  }

  sortButton(column: string): Locator {
    return this.root.locator("thead").getByRole("button", { name: column, exact: true });
  }

  sortState(column: string): Locator {
    return this.sortButton(column).locator("xpath=ancestor::th");
  }

  /** Waits for the first row, which any tab with data in the last 30 days has. */
  async expectRows(): Promise<void> {
    await expectDataRows(this.rows);
  }

  async columnText(column: string): Promise<string[]> {
    const index = await this.root
      .locator("thead tr")
      .first()
      .locator("th")
      .evaluateAll(
        (cells, name) =>
          cells.findIndex(
            (cell) => cell.querySelector("button span")?.textContent?.trim() === name,
          ),
        column,
      );
    expect(index, `column ${column}`).toBeGreaterThanOrEqual(0);
    return this.root.locator(`tbody > tr > td:nth-child(${index + 1})`).allInnerTexts();
  }

  /** Superset windows and narrower filters move this, whichever rows are on the page. */
  async columnSum(column: string): Promise<number> {
    return (await this.columnValues(column)).reduce((sum, value) => sum + value, 0);
  }

  async columnValues(column: string): Promise<number[]> {
    return (await this.columnText(column))
      .map(parseValue)
      .filter((value): value is number => value !== null);
  }

  /** Clicks the header until it reports `direction`; the first click depends on the current sort. */
  async sortBy(column: string, direction: SortDirection): Promise<void> {
    await expect(async () => {
      if ((await this.sortState(column).getAttribute("aria-sort")) !== direction) {
        await this.sortButton(column).click();
      }
      await expect(this.sortState(column)).toHaveAttribute("aria-sort", direction, {
        timeout: 1_000,
      });
    }).toPass();
  }

  /** Sorting refetches, so this retries until the rows settle into the expected order. */
  async expectSorted(column: string, direction: SortDirection): Promise<void> {
    await expect(async () => {
      const values = await this.columnValues(column);
      expect(values.length, `${column} values`).toBeGreaterThan(1);
      const ordered = values.toSorted((a, b) => (direction === "ascending" ? a - b : b - a));
      expect(values).toEqual(ordered);
    }).toPass();
  }
}
