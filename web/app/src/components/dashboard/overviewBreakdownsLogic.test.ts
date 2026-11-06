import { describe, expect, it } from "vitest";
import { testBreakdownRow } from "../../../test/dashboardFixtures";
import { EMPTY_SEGMENT_FILTERS, segmentFiltersKey } from "../../lib/segmentFilters";
import {
  type BreakdownSort,
  breakdownRowCountry,
  DEFAULT_BREAKDOWN_SORT,
  isBreakdownRowFiltered,
  sortedBreakdownRows,
  toggleBreakdownFilter,
} from "./overviewBreakdownsLogic";

describe("toggleBreakdownFilter", () => {
  it("adds a row's value to its dimension's filter and removes it on a second toggle", () => {
    const row = testBreakdownRow({ value: "US", label: "United States" });
    const on = toggleBreakdownFilter(EMPTY_SEGMENT_FILTERS, "countries", row);
    expect(on.countries).toEqual([{ code: "US", name: "United States" }]);
    expect(isBreakdownRowFiltered(on, "countries", row)).toBe(true);
    const off = toggleBreakdownFilter(on, "countries", row);
    expect(segmentFiltersKey(off)).toBe(segmentFiltersKey(EMPTY_SEGMENT_FILTERS));
  });

  it("maps source rows to campaign, referrer and direct filters", () => {
    const campaign = testBreakdownRow({
      value: "campaign-7",
      label: "launch",
      campaign: { source: "newsletter", medium: "email", campaign: "launch" },
    });
    const referrer = testBreakdownRow({ value: "referrer-3", label: "google.com" });
    const direct = testBreakdownRow({ value: "direct", label: "Direct" });
    let segments = toggleBreakdownFilter(EMPTY_SEGMENT_FILTERS, "sources", campaign);
    segments = toggleBreakdownFilter(segments, "sources", referrer);
    segments = toggleBreakdownFilter(segments, "sources", direct);
    expect(segments.sources).toEqual([
      { kind: "campaign", id: 7, name: "source=newsletter, medium=email, campaign=launch" },
      { kind: "referrer", id: 3, name: "google.com" },
      { kind: "direct", name: "Direct" },
    ]);
    expect(isBreakdownRowFiltered(segments, "sources", referrer)).toBe(true);
    expect(toggleBreakdownFilter(segments, "sources", referrer).sources).toHaveLength(2);
  });

  it("keeps pages and landing pages apart", () => {
    const row = testBreakdownRow({ value: "12", label: "/pricing" });
    const page = toggleBreakdownFilter(EMPTY_SEGMENT_FILTERS, "pages", row);
    expect(page.pages).toEqual([{ id: 12, name: "/pricing" }]);
    expect(isBreakdownRowFiltered(page, "landing_pages", row)).toBe(false);
    const landing = toggleBreakdownFilter(EMPTY_SEGMENT_FILTERS, "landing_pages", row);
    expect(landing.landingPages).toEqual([{ id: 12, name: "/pricing" }]);
  });

  it("turns landing attribute rows into attribute filters keyed by the row value", () => {
    const city = testBreakdownRow({ value: "US-New York", label: "New York" });
    const segments = toggleBreakdownFilter(EMPTY_SEGMENT_FILTERS, "cities", city);
    expect(segments.attributes).toEqual([
      { attribute: "city", value: "US-New York", name: "New York" },
    ]);
    expect(isBreakdownRowFiltered(segments, "countries", city)).toBe(false);
    expect(toggleBreakdownFilter(segments, "cities", city).attributes).toEqual([]);
  });
});

describe("breakdownRowCountry", () => {
  it("reads the country off country and city rows only", () => {
    expect(breakdownRowCountry("countries", testBreakdownRow({ value: "FR" }))).toBe("FR");
    expect(breakdownRowCountry("cities", testBreakdownRow({ value: "US-Winston-Salem" }))).toBe(
      "US",
    );
    expect(breakdownRowCountry("os", testBreakdownRow({ value: "US-x" }))).toBeUndefined();
  });
});

describe("sortedBreakdownRows", () => {
  const rows = [
    testBreakdownRow({ value: "busy", label: "Busy", visitors: 90, revenue: 0 }),
    testBreakdownRow({ value: "rich", label: "Alpha", visitors: 2, revenue: 5_000 }),
    testBreakdownRow({ value: "tied", label: "Zulu", visitors: 90, revenue: 100 }),
  ];
  const values = (sort: BreakdownSort) => sortedBreakdownRows(rows, sort).map((row) => row.value);

  it("keeps every row, ordered by the sorted measure with the other breaking ties", () => {
    expect(values(DEFAULT_BREAKDOWN_SORT)).toEqual(["tied", "busy", "rich"]);
    expect(values({ column: "revenue", direction: "desc" })).toEqual(["rich", "tied", "busy"]);
  });

  it("reverses the order when ascending", () => {
    expect(values({ column: "visitors", direction: "asc" })).toEqual(["rich", "busy", "tied"]);
  });

  it("orders by name", () => {
    expect(values({ column: "label", direction: "asc" })).toEqual(["rich", "busy", "tied"]);
    expect(values({ column: "label", direction: "desc" })).toEqual(["tied", "busy", "rich"]);
  });
});
