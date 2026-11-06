import { describe, expect, it } from "vitest";
import type { PageRow } from "../../lib/pagesApi";
import type { SearchTermRow, SearchTermsPeriod } from "../../lib/searchTermsApi";
import { nextColumnSort } from "./columnSort";
import { DASHBOARD_TAB_ITEMS, DASHBOARD_TABS, demoTabHref, isDashboardTab } from "./dashboardTabs";
import {
  niceAxisBoundsFromExtent,
  niceLineAxisBounds,
  niceRevenueAxisBounds,
  niceStepCeil,
} from "./overviewChartAxis";
import {
  DEFAULT_PAGES_SORT,
  isPagesTreemapSort,
  pageRowHasConversions,
  pageRowHighlights,
} from "./pagesLogic";
import {
  DEFAULT_SEARCH_TERMS_SORT,
  hasSearchTermsPeriodData,
  isSearchTermsTreemapSort,
  nextSearchTermsSortState,
  searchTermRowHighlights,
  searchTermRowKey,
} from "./searchTermsLogic";

describe("dashboard tabs", () => {
  it("lists every tab once, in the tab set", () => {
    expect(DASHBOARD_TAB_ITEMS.map((item) => item.value)).toEqual([...DASHBOARD_TABS]);
  });

  it("recognises tab names", () => {
    expect(isDashboardTab("pages")).toBe(true);
    expect(isDashboardTab("Pages")).toBe(false);
    expect(isDashboardTab(null)).toBe(false);
  });

  it("links the demo, keeping overview at the bare path", () => {
    expect(demoTabHref("overview")).toBe("/demo");
    expect(demoTabHref("search_terms")).toBe("/demo?tab=search_terms");
  });
});

describe("overview chart axis", () => {
  it.each([
    [0, 1],
    [-3, 1],
    [Number.NaN, 1],
    [0.7, 1],
    [1, 1],
    [1.5, 2],
    [3, 5],
    [7, 10],
    [120, 200],
  ])("rounds a rough step of %d up to %d", (rough, expected) => {
    expect(niceStepCeil(rough)).toBe(expected);
  });

  it("expands positive data to tick multiples", () => {
    const bounds = niceAxisBoundsFromExtent(0, 73);
    // min comes out as -0, which the axis formatters print as 0.
    expect(bounds.min).toBeCloseTo(0);
    expect(bounds).toEqual({
      min: bounds.min,
      max: 80,
      step: 10,
      ticksBelowZero: 0,
      ticksAboveZero: 8,
    });
  });

  it("adds ticks below zero for negative data", () => {
    const bounds = niceAxisBoundsFromExtent(-12, 70);
    expect(bounds.step).toBe(20);
    expect(bounds.min).toBe(-20);
    expect(bounds.max).toBe(80);
  });

  it("keeps one tick above zero for empty data", () => {
    expect(niceLineAxisBounds(0)).toMatchObject({ max: 1, ticksAboveZero: 1 });
    expect(niceLineAxisBounds(0).min).toBeCloseTo(0);
    expect(niceLineAxisBounds(-5).max).toBe(1);
  });

  it("works in dollars for revenue held in thousandths", () => {
    const bounds = niceRevenueAxisBounds(0, 730_000);
    expect(bounds.stepDollars).toBe(100);
    expect(bounds.step).toBe(100_000);
    expect(bounds.max).toBe(800_000);
  });
});

describe("pages logic", () => {
  it("flips direction on the same column and starts a new column descending", () => {
    const sort = { column: "visitors", direction: "desc" } as const;
    expect(nextColumnSort(sort, "visitors")).toEqual({ column: "visitors", direction: "asc" });
    expect(nextColumnSort({ ...sort, direction: "asc" }, "visitors").direction).toBe("desc");
    expect(nextColumnSort(sort, "exit")).toEqual({
      column: "exit",
      direction: "desc",
    });
  });

  it("matches the treemap only on the default ranking", () => {
    expect(isPagesTreemapSort(DEFAULT_PAGES_SORT)).toBe(true);
    expect(isPagesTreemapSort({ ...DEFAULT_PAGES_SORT, direction: "asc" })).toBe(false);
  });

  it("checks for conversions", () => {
    expect(pageRowHasConversions({ converting_visitors: 0 })).toBe(false);
    expect(pageRowHasConversions({ converting_visitors: 1 })).toBe(true);
  });

  it("highlights exit flags on pages and bounce flags on landing pages", () => {
    const row = {
      best_visitors_uplift: true,
      best_conversion_uplift: false,
      best_exit_uplift: true,
      highest_exit_rate: true,
      best_bounce_uplift: true,
      highest_bounce_rate: true,
      best_session_uplift: true,
    } as PageRow;
    expect(pageRowHighlights(row, "pages").map((h) => "metric" in h && h.metric)).toEqual([
      "visitorsUplift",
      "exitUplift",
      "highestExitRate",
      "sessionUplift",
    ]);
    expect(pageRowHighlights(row, "landingPages").map((h) => "metric" in h && h.metric)).toEqual([
      "visitorsUplift",
      "bounceUplift",
      "highestBounceRate",
      "sessionUplift",
    ]);
  });
});

describe("search terms logic", () => {
  it("keys rows by query", () => {
    expect(searchTermRowKey({ query: "crm" } as SearchTermRow)).toBe("term:crm");
  });

  it("counts a period with only summary clicks or impressions as data", () => {
    const empty = {
      terms: [],
      summary: { clicks: 0, impressions: 0 },
    } as unknown as SearchTermsPeriod;
    expect(hasSearchTermsPeriodData(empty)).toBe(false);
    expect(
      hasSearchTermsPeriodData({ ...empty, summary: { ...empty.summary, impressions: 3 } }),
    ).toBe(true);
  });

  it("toggles sort direction like the other tables", () => {
    expect(nextSearchTermsSortState(DEFAULT_SEARCH_TERMS_SORT, "estimated_revenue").direction).toBe(
      "asc",
    );
    expect(nextSearchTermsSortState(DEFAULT_SEARCH_TERMS_SORT, "clicks")).toEqual({
      orderBy: "clicks",
      direction: "desc",
    });
    expect(isSearchTermsTreemapSort(DEFAULT_SEARCH_TERMS_SORT)).toBe(true);
  });

  it("highlights cannibalization and lost clicks", () => {
    const row = { highest_cannibalization: true, highest_lost_clicks: true } as SearchTermRow;
    expect(searchTermRowHighlights(row)).toEqual([
      { metric: "highestCannibalization" },
      { metric: "highestLostClicks" },
    ]);
  });
});
