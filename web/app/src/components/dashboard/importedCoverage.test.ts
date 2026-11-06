import { describe, expect, it } from "vitest";
import type { ImportedCoverage } from "../../lib/importedCoverageApi";
import { importedBucketSpans, importedDataLeftOut } from "./importedCoverage";

const coverage: ImportedCoverage = {
  include_in_reports: true,
  has_data: true,
  has_events: true,
  ranges: [],
};

describe("importedDataLeftOut", () => {
  it("is true for tabs that can't use imported data", () => {
    expect(importedDataLeftOut(coverage, "funnels", false)).toBe(true);
    expect(importedDataLeftOut(coverage, "revenue", false)).toBe(true);
    expect(importedDataLeftOut(coverage, "visitors", false)).toBe(true);
  });

  it("is true for a tab that adds it only once a filter narrows the view", () => {
    expect(importedDataLeftOut(coverage, "overview", false)).toBe(false);
    expect(importedDataLeftOut(coverage, "overview", true)).toBe(true);
    expect(importedDataLeftOut(coverage, "pages", true)).toBe(true);
  });

  it("stays quiet when nothing imported is being left out", () => {
    expect(importedDataLeftOut({ ...coverage, has_data: false }, "funnels", true)).toBe(false);
    expect(importedDataLeftOut(undefined, "funnels", true)).toBe(false);
    expect(importedDataLeftOut(coverage, "search_terms", true)).toBe(false);
  });

  it("says nothing of events a property never sent", () => {
    expect(importedDataLeftOut(coverage, "events", true)).toBe(true);
    expect(importedDataLeftOut({ ...coverage, has_events: false }, "events", true)).toBe(false);
  });

  it("stays quiet when the project chose to keep imported data out of reports", () => {
    expect(importedDataLeftOut({ ...coverage, include_in_reports: false }, "funnels", true)).toBe(
      false,
    );
  });
});

describe("importedBucketSpans", () => {
  const day = (date: string) => Date.parse(`${date}T00:00:00Z`) / 1000;
  const range = { start_date: "2026-03-02", end_date: "2026-03-03", source_name: "Acme" };

  it("finds the buckets inside an import", () => {
    const starts = ["2026-03-01", "2026-03-02", "2026-03-03", "2026-03-04"].map(day);
    expect(importedBucketSpans(starts, [range], "UTC")).toEqual([{ first: 1, last: 2 }]);
  });

  it("counts a week that holds the import's first day", () => {
    const starts = ["2026-02-23", "2026-03-02", "2026-03-09"].map(day);
    const wide = { ...range, start_date: "2026-02-25", end_date: "2026-03-03" };
    expect(importedBucketSpans(starts, [wide], "UTC")).toEqual([{ first: 0, last: 1 }]);
  });

  it("covers every hour of an imported day", () => {
    const hour = (date: string, time: string) => Date.parse(`${date}T${time}:00Z`) / 1000;
    const starts = [
      hour("2026-03-01", "23:00"),
      hour("2026-03-02", "00:00"),
      hour("2026-03-02", "12:00"),
      hour("2026-03-03", "23:00"),
      hour("2026-03-04", "00:00"),
    ];
    expect(importedBucketSpans(starts, [range], "UTC")).toEqual([{ first: 1, last: 3 }]);
  });

  it("skips an import outside the chart", () => {
    const starts = ["2026-04-01", "2026-04-02"].map(day);
    expect(importedBucketSpans(starts, [range], "UTC")).toEqual([]);
  });

  it("reads bucket dates in the project's timezone", () => {
    const tokyoMidnight = Date.parse("2026-03-01T15:00:00Z") / 1000;
    expect(importedBucketSpans([tokyoMidnight], [range], "Asia/Tokyo")).toEqual([
      { first: 0, last: 0 },
    ]);
  });
});
