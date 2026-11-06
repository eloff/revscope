import { DEMO_TABS, type DemoDashboard, expect, expectDataRows, test } from "./support/demo";
import { parseValue } from "./support/values";

test.describe("tab navigation", () => {
  test("the demo opens on Overview with real data", async ({ demo }) => {
    await demo.goto();
    await expect(demo.tab("Overview")).toHaveAttribute("aria-selected", "true");
    expect(
      await demo.loadedMetric("Visitors"),
      "demo visitors in the last 30 days",
    ).toBeGreaterThan(0);
  });

  test("offers every demo tab, but not the signed-in Projects tab", async ({ demo }) => {
    await demo.goto();
    for (const { label } of DEMO_TABS) {
      await expect(demo.tab(label)).toBeVisible();
    }
    await expect(demo.tab("Projects")).toHaveCount(0);
  });

  test("clicking through the tabs updates the URL, and Back retraces them", async ({
    demo,
    page,
  }) => {
    await demo.goto();
    for (const { label, param } of DEMO_TABS.filter((tab) => tab.param !== "overview")) {
      await demo.tab(label).click();
      await expect(demo.tab(label)).toHaveAttribute("aria-selected", "true");
      await expect(demo.panel(label)).toBeVisible();
      await demo.expectParam("tab", param);
    }

    await page.goBack();
    await expect(demo.tab("Events")).toHaveAttribute("aria-selected", "true");
    await page.goBack();
    await expect(demo.tab("Crawlers")).toHaveAttribute("aria-selected", "true");
    await page.goForward();
    await expect(demo.tab("Events")).toHaveAttribute("aria-selected", "true");
    await demo.expectParam("tab", "events");
  });

  for (const { label, param } of DEMO_TABS) {
    test(`a link to ${label} opens it with data`, async ({ demo }) => {
      await demo.goto(`?tab=${param}`);
      await expect(demo.tab(label)).toHaveAttribute("aria-selected", "true");
      const panel = demo.panel(label);
      await expect(panel).toBeVisible();

      if (param === "overview") {
        await expect(panel.getByRole("application")).toBeVisible();
        expect(await demo.loadedMetric("Visitors")).toBeGreaterThan(0);
      } else if (param === "revenue") {
        await expect(
          panel.getByText(/^[1-9][\d,]* purchases? in the last 12 months$/),
        ).toBeVisible();
      } else if (param === "funnels") {
        // Steps are labelled on the chart with their visitor counts. More than one must have
        // some, so a query that loses everyone after the first step doesn't read as data.
        await expect
          .poll(
            async () =>
              ((await panel.getByRole("application").textContent()) ?? "").match(/: [1-9]/g)
                ?.length,
          )
          .toBeGreaterThan(1);
      } else {
        await expectDataRows(panel.locator("tbody > tr"));
      }
      await expect(
        panel.getByRole("heading", { name: /^No .* (date range|12 months)$/ }),
      ).toHaveCount(0);
    });
  }

  test("an unknown tab, or one the demo doesn't have, falls back to Overview", async ({ demo }) => {
    for (const query of ["?tab=nonsense", "?tab=projects"]) {
      await demo.goto(query);
      await expect(demo.tab("Overview")).toHaveAttribute("aria-selected", "true");
    }
  });

  test("old bookmarks land on the tabs that replaced them", async ({ demo }) => {
    await demo.goto("?tab=conversions");
    await expect(demo.tab("Visitors")).toHaveAttribute("aria-selected", "true");
    await demo.expectParam("tab", "visitors");
    await demo.expectParam("people", "customers");
    await expect(
      demo.panel("Visitors").getByRole("radio", { name: "New Customers" }),
    ).toBeChecked();

    await demo.goto("?tab=landing_pages");
    await expect(demo.tab("Pages")).toHaveAttribute("aria-selected", "true");
    await demo.expectParam("tab", "pages");
    await demo.expectParam("pages", "landing");
    await expect(demo.panel("Pages").getByRole("radio", { name: "Landing Pages" })).toBeChecked();
  });
});

test.describe("modes within a tab", () => {
  test("Pages switches to Landing Pages and back", async ({ demo, page }) => {
    await demo.goto("?tab=pages");
    const panel = demo.panel("Pages");
    await expect(panel.getByRole("radio", { name: "Pages", exact: true })).toBeChecked();
    await demo.grid("Pages", "Exit rate").expectRows();

    await demo.chooseMode(panel, "Landing Pages");
    await demo.expectParam("pages", "landing");
    const landing = demo.grid("Pages", "Bounce rate");
    await landing.expectRows();
    // Both modes stay mounted and slide, so "visible" can't tell which one is showing.
    await expect(landing.rows.first()).toBeInViewport();

    await page.goBack();
    await demo.expectParam("pages", null);
    await expect(panel.getByRole("radio", { name: "Pages", exact: true })).toBeChecked();
    await expect(demo.grid("Pages", "Exit rate").rows.first()).toBeInViewport();
  });

  test("Visitors switches to New Customers", async ({ demo }) => {
    await demo.goto("?tab=visitors");
    const panel = demo.panel("Visitors");
    await demo.grid("Visitors", "Pages Viewed").expectRows();
    await demo.chooseMode(panel, "New Customers");
    await demo.expectParam("people", "customers");
    await expect(panel.getByRole("radio", { name: "New Customers" })).toBeChecked();
    const customers = demo.grid("Visitors", "Time to purchase");
    await customers.expectRows();
    await expect(customers.rows.first()).toBeInViewport();
  });

  test("Audience switches between countries and cities", async ({ demo }) => {
    await demo.goto("?tab=audience");
    const panel = demo.panel("Audience");
    await expect(panel.getByRole("tab", { name: "Country" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await panel.getByRole("tab", { name: "Cities" }).click();
    await expectDataRows(panel.getByRole("tabpanel", { name: "Cities" }).locator("tbody > tr"));
  });

  test("Overview's breakdown panels switch dimension", async ({ demo }) => {
    await demo.goto();
    const panel = demo.panel("Overview");
    for (const tab of ["Channel", "City", "Landing page", "OS"]) {
      await panel.getByRole("tab", { name: tab, exact: true }).click();
      await expect(
        panel
          .getByRole("tabpanel", { name: tab, exact: true })
          .locator("button[aria-pressed]")
          .first(),
      ).toBeVisible();
    }
  });
});

test.describe("links between tabs", () => {
  /** Clicks the first matching link, returning the name it says it leads to. */
  const followLink = async (demo: DemoDashboard, tab: string, pattern: RegExp) => {
    const link = demo.panel(tab).getByRole("button", { name: pattern }).first();
    await expect(link).toBeVisible();
    const name = (await link.getAttribute("title"))?.match(pattern)?.[1] ?? "";
    expect(name, "link target").not.toBe("");
    await link.click();
    return name;
  };

  test("a visitor's location scrolls to that country in Audience", async ({ demo }) => {
    await demo.goto("?tab=visitors");
    const country = await followLink(demo, "Visitors", /^View details for (.*) in Audience$/);
    await expect(demo.tab("Audience")).toHaveAttribute("aria-selected", "true");
    await expect.poll(() => demo.param("cc")).toMatch(/^[A-Z]{2}$/);
    const row = demo.panel("Audience").locator(`tr[data-record-key="${demo.param("cc")}"]`);
    await expect(row).toContainText(country);
    await expect(row).toBeInViewport();
  });

  test("a visitor's referrer scrolls to it in Sources", async ({ demo }) => {
    await demo.goto("?tab=visitors");
    // A referrer's hostname, since campaign links and rows name a campaign differently.
    const source = await followLink(
      demo,
      "Visitors",
      /^View details for ([\w-]+\.[\w.-]+) in Sources$/,
    );
    await expect(demo.tab("Sources")).toHaveAttribute("aria-selected", "true");
    await expect.poll(() => demo.param("source")).toMatch(/^referrer:\d+$/);
    // Rows without conversions have nothing to expand, so aren't buttons.
    const row = demo
      .panel("Sources")
      .locator("tr[data-record-key]")
      .filter({ has: demo.page.getByRole("cell", { name: source, exact: true }) });
    await expect(row).toBeInViewport();
  });

  test("an event's page opens that page expanded in Pages", async ({ demo }) => {
    await demo.goto("?tab=events");
    const path = await followLink(demo, "Events", /^View details for (.*) in Pages$/);
    await expect(demo.tab("Pages")).toHaveAttribute("aria-selected", "true");
    await expect.poll(() => demo.param("path_id")).toMatch(/^\d+$/);
    const expanded = demo.panel("Pages").locator('tr[aria-expanded="true"]');
    await expect(expanded).toHaveAttribute("aria-label", `Hide transitions for ${path}`);
  });

  test("a search term's landing path opens it expanded in Landing Pages", async ({ demo }) => {
    await demo.goto("?tab=search_terms");
    const path = await followLink(demo, "Search terms", /^View details for (.*) in Landing Pages$/);
    await expect(demo.tab("Pages")).toHaveAttribute("aria-selected", "true");
    await demo.expectParam("pages", "landing");
    await expect.poll(() => demo.param("landing_path_id")).toMatch(/^\d+$/);
    const expanded = demo.panel("Pages").locator('tr[aria-expanded="true"]');
    await expect(expanded).toHaveAttribute(
      "aria-label",
      new RegExp(`^Hide transitions for [^/]*${RegExp.escape(path)}$`),
    );
  });

  test("an event's visitor count lists just those visitors, until the filter is removed", async ({
    demo,
  }) => {
    await demo.goto("?tab=events");
    const events = demo.grid("Events", "Conversions");
    await events.expectRows();
    // The rarest event, so its visitors can't also be the whole page of latest visitors.
    await events.sortBy("Visitors", "ascending");
    await events.expectSorted("Visitors", "ascending");
    const link = events.root
      .getByRole("button", { name: /^View visitors for / })
      .filter({ hasNotText: /^0$/ })
      .first();
    const count = parseValue(await link.innerText()) as number;
    await link.click();
    await expect(demo.tab("Visitors")).toHaveAttribute("aria-selected", "true");
    await demo.expectParam("filter", "event");

    const grid = demo.grid("Visitors", "Pages Viewed");
    await grid.expectRows();
    const journeys = grid.root.locator('tr[aria-label^="View journey for"]');
    const listed = () =>
      journeys.evaluateAll((rows) => rows.map((row) => row.getAttribute("aria-label")));
    await expect(journeys.first()).toBeVisible();
    const filtered = await listed();
    expect(filtered.length).toBeLessThanOrEqual(count);

    const panel = demo.panel("Visitors");
    await panel.getByRole("button", { name: /^Remove filter: / }).click();
    await demo.expectParam("filter", null);
    await expect(panel.getByRole("button", { name: /^Remove filter: / })).toHaveCount(0);
    await expect
      .poll(async () => {
        const all = await listed();
        return all.length > 0 && all.join() !== filtered.join();
      })
      .toBe(true);
  });
});

test.describe("drilling into a row", () => {
  const expandFirstPage = async (demo: DemoDashboard) => {
    await demo.goto("?tab=pages");
    const grid = demo.grid("Pages", "Exit rate");
    await grid.expectRows();
    const row = grid.root.locator('tr[aria-label^="Show transitions for"]').first();
    const name = (await row.getAttribute("aria-label"))?.replace("Show transitions for ", "") ?? "";
    await row.click();
    const expanded = grid.root.locator(
      `tr[aria-label=${JSON.stringify(`Hide transitions for ${name}`)}]`,
    );
    await expect(expanded).toHaveAttribute("aria-expanded", "true");
    await expect.poll(() => demo.param("path_id")).toMatch(/^\d+$/);
    return { grid, expanded };
  };

  test("a page row expands its transitions", async ({ demo }) => {
    await expandFirstPage(demo);
  });

  test("a second click collapses a page row", async ({ demo }) => {
    const { grid, expanded } = await expandFirstPage(demo);
    await expanded.click();
    await expect(grid.root.locator('tr[aria-expanded="true"]')).toHaveCount(0);
    await demo.expectParam("path_id", null);
  });

  test("a visitor row opens that visitor's journey", async ({ demo }) => {
    await demo.goto("?tab=visitors");
    const grid = demo.grid("Visitors", "Pages Viewed");
    await grid.expectRows();
    const row = grid.root.getByRole("button", { name: /^View journey for visitor / }).first();
    const visitor = (await row.getAttribute("aria-label"))?.replace(
      "View journey for visitor ",
      "",
    );
    await row.click();
    await expect(
      demo.panel("Visitors").getByRole("heading", { name: `Visitor ${visitor}`, exact: true }),
    ).toBeInViewport();
    // New Customers keeps a journey view mounted too, so go by what's on screen.
    const close = demo
      .panel("Visitors")
      .getByRole("button", { name: "Close", exact: true })
      .first();
    await expect(close).toBeInViewport();
    await close.click();
    await expect(grid.rows.first()).toBeInViewport();
  });

  test("a campaign row expands its details", async ({ demo }) => {
    await demo.goto("?tab=campaigns");
    const show = demo
      .panel("Campaigns")
      .getByRole("button", { name: /^Show details for / })
      .first();
    const name = (await show.getAttribute("aria-label"))?.replace("Show details for ", "") ?? "";
    await show.click();
    await expect(
      demo
        .panel("Campaigns")
        .getByRole("button", { name: `Hide details for ${name}`, exact: true }),
    ).toBeVisible();
  });
});

test("the embedded overview links into the full demo", async ({ page, demo }) => {
  await page.goto("embed/");
  const views = page.getByRole("navigation", { name: "Dashboard views" });
  await expect(views.locator('[aria-current="page"]')).toHaveText("Overview");
  await views.getByRole("link", { name: "Pages", exact: true }).click();
  await expect(demo.tab("Pages")).toHaveAttribute("aria-selected", "true");
  await demo.expectParam("tab", "pages");
});

test("Ask AI, where it is configured, starts with nothing to send", async ({ demo, page }) => {
  // The tab needs an AI gateway, which a local or CI backend may not have, and it
  // only appears once this answers.
  const status = page.waitForResponse((response) => response.url().includes("/public/chat/status"));
  await demo.goto();
  const { available } = (await (await status).json()) as { available: boolean };
  test.skip(!available, "Ask AI isn't configured on this backend");

  await demo.tab("Ask AI").click();
  await expect(
    demo.panel("Ask AI").getByRole("button", { name: "Ask", exact: true }),
  ).toBeDisabled();
});
