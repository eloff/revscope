import { describe, expect, it, vi } from "vitest";
// Transforms the module graph (vanilla-extract styles included) once up front;
// on a busy run the first fresh import below could otherwise outlast the timeout.
import "./dashboardPreferences";

const PREFIX = "revscope.dashboard";

/** Preferences deserialize from localStorage when the module loads, so each test loads a fresh copy. */
async function loadWith(stored: Record<string, unknown>) {
  localStorage.clear();
  for (const [key, value] of Object.entries(stored)) {
    localStorage.setItem(
      `${PREFIX}.${key}`,
      typeof value === "string" ? value : JSON.stringify(value),
    );
  }
  vi.resetModules();
  return import("./dashboardPreferences");
}

describe("dashboard preferences", () => {
  it("uses defaults when nothing is stored", async () => {
    const prefs = await loadWith({});
    expect(prefs.chartVisibility().lineMetric).toBe("visitors");
    expect(prefs.revenueSourceSort()).toEqual({ column: "revenue", direction: "desc" });
    expect(prefs.disabledSourceGroups()).toEqual([]);
    expect(prefs.audienceBreakdownSorts().cities).toEqual({ column: "revenue", direction: "desc" });
  });

  it("restores valid stored sorts", async () => {
    const prefs = await loadWith({
      revenueSourceSort: { column: "visitors", direction: "asc" },
      conversionsSort: { column: "amount", direction: "asc" },
      pagesSort: { column: "visitors", direction: "asc" },
      eventsSort: { column: "revenue", direction: "asc" },
      crawlersSort: { column: "views", direction: "asc" },
      visitorsSort: { orderBy: "source", direction: "asc" },
      searchTermsSort: { orderBy: "clicks", direction: "asc" },
    });
    expect(prefs.revenueSourceSort()).toEqual({ column: "visitors", direction: "asc" });
    expect(prefs.conversionsSort()).toEqual({ column: "amount", direction: "asc" });
    expect(prefs.pagesSort()).toEqual({ column: "visitors", direction: "asc" });
    expect(prefs.eventsSort()).toEqual({ column: "revenue", direction: "asc" });
    expect(prefs.crawlersSort()).toEqual({ column: "views", direction: "asc" });
    expect(prefs.visitorsSort()).toEqual({ orderBy: "source", direction: "asc" });
    expect(prefs.searchTermsSort()).toEqual({ orderBy: "clicks", direction: "asc" });
  });

  it.each<[string, unknown, string]>([
    ["revenueSourceSort", { column: "revenueChange", direction: "asc" }, "legacy column"],
    ["revenueSourceSort", { column: "revenue", direction: "sideways" }, "bad direction"],
    ["conversionsSort", { column: "nope", direction: "asc" }, "unknown column"],
    ["pagesSort", "not json", "unparseable JSON"],
    ["eventsSort", null, "null"],
    ["crawlersSort", { direction: "asc" }, "missing column"],
    ["visitorsSort", { column: "source", direction: "asc" }, "column instead of orderBy"],
  ])("falls back to the default for %s (%j, %s)", async (key, value) => {
    const prefs = await loadWith({ [key]: value });
    const fresh = await loadWith({});
    const read = (module: typeof prefs) =>
      (module as unknown as Record<string, () => unknown>)[key]?.();
    expect(read(prefs)).toEqual(read(fresh));
  });

  it("accepts the legacy `column` key for search terms", async () => {
    const prefs = await loadWith({ searchTermsSort: { column: "ctr", direction: "asc" } });
    expect(prefs.searchTermsSort()).toEqual({ orderBy: "ctr", direction: "asc" });
  });

  it("drops unknown source groups and crawler categories", async () => {
    const prefs = await loadWith({
      disabledSourceGroups: ["search", "martian", 4],
      disabledCrawlerCategories: ["archive", "telepathy"],
    });
    expect(prefs.disabledSourceGroups()).toEqual(["search"]);
    expect(prefs.disabledCrawlerCategories()).toEqual(["archive"]);
  });

  it("merges stored bar series over the defaults and validates the line metric", async () => {
    const prefs = await loadWith({
      chartVisibility: { lineMetric: "made_up", barEnabled: false, barSeries: { churn_mrr: true } },
    });
    expect(prefs.chartVisibility()).toEqual({
      lineMetric: "visitors",
      barEnabled: false,
      barSeries: {
        mrr: true,
        new_subscriptions_mrr: true,
        new_revenue: true,
        churn_mrr: true,
        refunds_and_chargebacks: false,
      },
    });
  });

  it("keeps a deliberately hidden line metric", async () => {
    const prefs = await loadWith({ chartVisibility: { lineMetric: null } });
    expect(prefs.chartVisibility().lineMetric).toBeNull();
  });

  it("only allows the conversion column on the country breakdown", async () => {
    const prefs = await loadWith({
      audienceBreakdownSorts: {
        country: { column: "conversion", direction: "asc" },
        browser: { column: "conversion", direction: "asc" },
        os: { column: "name", direction: "asc" },
      },
    });
    const sorts = prefs.audienceBreakdownSorts();
    expect(sorts.country).toEqual({ column: "conversion", direction: "asc" });
    expect(sorts.browser).toEqual({ column: "revenue", direction: "desc" });
    expect(sorts.os).toEqual({ column: "name", direction: "asc" });
  });

  it("persists updates made through the helpers", async () => {
    const prefs = await loadWith({});
    prefs.updateDisabledSourceGroups((groups) => new Set([...groups, "ai"]));
    prefs.updateDisabledCrawlerCategories((categories) => new Set([...categories, "headless"]));
    prefs.setAudienceBreakdownSort("device", { column: "visitors", direction: "asc" });

    expect(prefs.disabledSourceGroupsSet()).toEqual(new Set(["ai"]));
    expect(JSON.parse(localStorage.getItem(`${PREFIX}.disabledSourceGroups`) ?? "")).toEqual([
      "ai",
    ]);
    expect(prefs.disabledCrawlerCategoriesSet()).toEqual(new Set(["headless"]));
    expect(
      JSON.parse(localStorage.getItem(`${PREFIX}.audienceBreakdownSorts`) ?? "{}").device,
    ).toEqual({ column: "visitors", direction: "asc" });
  });
});
