import { type DemoDashboard, expect, type Locator, test } from "./support/demo";
import { parseValue } from "./support/values";

test.describe("Overview segment filters", () => {
  const countryRows = (demo: DemoDashboard) =>
    demo
      .panel("Overview")
      .getByRole("tabpanel", { name: "Country", exact: true })
      .locator("button[aria-pressed]");

  const rowLabel = async (row: Locator) => (await row.getAttribute("title"))?.split("\n")[0] ?? "";

  test("clicking a row narrows the totals, and removing the chip restores them", async ({
    demo,
  }) => {
    await demo.goto();
    const all = await demo.loadedMetric("Visitors");
    await expect(countryRows(demo).nth(1)).toBeVisible();

    const row = countryRows(demo).first();
    const country = await rowLabel(row);
    // The row's title has the exact count, and the cell an abbreviation.
    const promised = /^([\d,]+) visitors/m.exec((await row.getAttribute("title")) ?? "")?.[1];
    await row.click();

    await expect(demo.filterChip(`Country: ${country}`)).toBeVisible();
    await expect(
      countryRows(demo).filter({ has: demo.page.getByText(country, { exact: true }) }),
    ).toHaveAttribute("aria-pressed", "true");
    await demo.expectMetricNarrowedFrom("Visitors", all);
    expect(await demo.metric("Visitors")).toBe(parseValue(promised ?? "?"));

    await demo.filterChip(`Country: ${country}`).click();
    await expect(demo.filterChip(/.*/)).toHaveCount(0);
    await expect.poll(() => demo.metric("Visitors")).toBe(all);
  });

  test("clicking the filtered row again removes the filter", async ({ demo }) => {
    await demo.goto();
    const row = countryRows(demo).first();
    await expect(row).toBeVisible();
    const country = await rowLabel(row);
    await row.click();
    await expect(demo.filterChip(`Country: ${country}`)).toBeVisible();
    await countryRows(demo)
      .filter({ has: demo.page.getByText(country, { exact: true }) })
      .click();
    await expect(demo.filterChip(/.*/)).toHaveCount(0);
  });

  test("filters on different dimensions narrow further", async ({ demo }) => {
    await demo.goto();
    await expect(countryRows(demo).first()).toBeVisible();
    await countryRows(demo).first().click();
    await expect(demo.filterChip(/Country: .*/)).toBeVisible();
    await expect.poll(() => demo.metric("Visitors")).toBeGreaterThan(0);
    const oneCountry = await demo.loadedMetric("Visitors");

    const browsers = demo
      .panel("Overview")
      .getByRole("tabpanel", { name: "Browser", exact: true })
      .locator("button[aria-pressed]");
    await browsers.first().click();
    await expect(demo.filterChip(/Browser: .*/)).toBeVisible();
    await demo.expectMetricNarrowedFrom("Visitors", oneCountry);
  });

  test("a filter narrows the data on other tabs too", async ({ demo }) => {
    await demo.goto("?tab=pages");
    const grid = demo.grid("Pages", "Exit rate");
    await grid.expectRows();
    await grid.sortBy("Visitors", "descending");
    await grid.expectSorted("Visitors", "descending");
    const all = await grid.columnSum("Visitors");

    await demo.tab("Overview").click();
    await expect(countryRows(demo).first()).toBeVisible();
    await countryRows(demo).first().click();
    await expect(demo.filterChip(/Country: .*/)).toBeVisible();

    await demo.tab("Pages").click();
    // The grid empties while it reloads, so zero isn't the narrowed total.
    await expect
      .poll(async () => {
        const narrowed = await grid.columnSum("Visitors");
        return narrowed > 0 && narrowed < all;
      })
      .toBe(true);

    await demo.filterChip(/Country: .*/).click();
    await expect(demo.filterChip(/.*/)).toHaveCount(0);
    await expect.poll(() => grid.columnSum("Visitors")).toBe(all);
  });
});

test.describe("filtering from a grid", () => {
  // Grids within a tab load independently, so each is picked out by a column of its own.
  const CASES = [
    { tab: "Sources", column: "Landing paths", chip: /Source: .*/ },
    { tab: "Audience", column: "Country", chip: /Country: .*/ },
    { tab: "Pages", column: "Exit rate", chip: /Page: .*/ },
    { tab: "Events", column: "Conversions", chip: /Event: .*/ },
  ];
  for (const { tab, column, chip } of CASES) {
    test(`a ${tab} row filters the dashboard and opens Overview`, async ({ demo }) => {
      await demo.goto();
      const all = await demo.loadedMetric("Visitors");
      await demo.tab(tab).click();
      const grid = demo.grid(tab, column).root;
      const filter = grid.getByRole("button", { name: /^Filter by / }).first();
      await expect(filter).toBeVisible();

      await filter.click();

      await expect(demo.filterChip(chip)).toBeVisible();
      await expect(demo.tab("Overview")).toHaveAttribute("aria-selected", "true");
      await demo.expectParam("tab", "overview");
      await demo.expectMetricNarrowedFrom("Visitors", all);

      // Back on the tab, the row shows that it's the one being filtered on.
      await demo.tab(tab).click();
      const active = grid.getByRole("button", { name: /^Filtering by / });
      await expect(active).toHaveAttribute("aria-pressed", "true");

      await demo.filterChip(chip).click();
      await expect(demo.filterChip(/.*/)).toHaveCount(0);
      await expect(active).toHaveCount(0);
    });
  }
});

test.describe("show and hide toggles", () => {
  test("Sources hides a group from the grid and brings it back", async ({ demo }) => {
    await demo.goto("?tab=revenue_sources");
    const grid = demo.grid("Sources", "Landing paths");
    await grid.expectRows();
    const [group] = await grid.columnText("Group");
    const card = demo.panel("Sources").getByRole("button", { name: new RegExp(`^${group} \\$`) });
    await expect(card).toHaveAttribute("aria-pressed", "true");

    await card.click();
    await expect(card).toHaveAttribute("aria-pressed", "false");
    // Other rows must remain: a blank grid doesn't contain the hidden one either.
    await expect
      .poll(async () => {
        const remaining = await grid.columnText("Group");
        return remaining.length > 0 && !remaining.includes(group);
      })
      .toBe(true);

    await card.click();
    await expect(card).toHaveAttribute("aria-pressed", "true");
    await expect.poll(() => grid.columnText("Group")).toContain(group);
  });

  test("Crawlers hides a category from the grid and brings it back", async ({ demo }) => {
    await demo.goto("?tab=crawlers");
    const grid = demo.grid("Crawlers", "Views");
    await grid.expectRows();
    const [category] = await grid.columnText("Category");
    const card = demo
      .panel("Crawlers")
      .getByRole("button", { name: new RegExp(`^${category} \\d`) });
    await expect(card).toHaveAttribute("aria-pressed", "true");

    await card.click();
    await expect(card).toHaveAttribute("aria-pressed", "false");
    // Other rows must remain: a blank grid doesn't contain the hidden one either.
    await expect
      .poll(async () => {
        const remaining = await grid.columnText("Category");
        return remaining.length > 0 && !remaining.includes(category);
      })
      .toBe(true);

    await card.click();
    await expect.poll(() => grid.columnText("Category")).toContain(category);
  });

  test("an Overview tile toggles its series on the chart, and the choice survives a reload", async ({
    demo,
    page,
  }) => {
    await demo.goto();
    const tile = demo.metricTile("Visitors");
    await expect(tile).toHaveAttribute("aria-pressed", "true");
    await demo.loadedMetric("Visitors");
    // ApexCharts doesn't update the chart's aria-label when a series is hidden.
    const series = demo.panel("Overview").getByRole("application").locator(".apexcharts-series");
    const seriesCount = () => series.count();
    await expect.poll(seriesCount).toBeGreaterThan(1);
    const before = await seriesCount();

    await tile.click();
    await expect(tile).toHaveAttribute("aria-pressed", "false");
    await expect.poll(seriesCount).toBe(before - 1);

    await page.reload();
    await expect(demo.metricTile("Visitors")).toHaveAttribute("aria-pressed", "false");
    await demo.loadedMetric("Revenue");
    await expect.poll(seriesCount).toBe(before - 1);
  });
});
