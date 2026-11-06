import { describe, expect, it } from "vitest";
import {
  COMPARISON_BASELINE_SECONDS,
  formatCrawlersViewNote,
  formatInclusiveDateRange,
  formatMrrComparisonNote,
  formatPeriodComparisonNote,
  periodComparisonFromRange,
} from "./periodComparison";

const utc = (iso: string) => Date.parse(`${iso}T00:00:00Z`) / 1000;
const DAY = 86_400;

describe("periodComparisonFromRange", () => {
  it("compares against the 30 days before the range", () => {
    const range = { start: utc("2026-03-01"), end: utc("2026-03-08") };
    expect(periodComparisonFromRange(range)).toEqual({
      current: range,
      previous: { start: range.start - COMPARISON_BASELINE_SECONDS, end: range.start },
      dayCount: 7,
    });
  });
});

describe("formatInclusiveDateRange", () => {
  it("shows a single day once", () => {
    expect(
      formatInclusiveDateRange({ start: utc("2026-03-05"), end: utc("2026-03-06") }, "UTC"),
    ).toBe("Mar 5, 2026");
  });

  it("treats the end as exclusive within one month", () => {
    expect(
      formatInclusiveDateRange({ start: utc("2026-03-01"), end: utc("2026-03-08") }, "UTC"),
    ).toBe("Mar 1–7, 2026");
  });

  it("names both months within one year", () => {
    expect(
      formatInclusiveDateRange({ start: utc("2026-02-20"), end: utc("2026-03-08") }, "UTC"),
    ).toBe("Feb 20–Mar 7, 2026");
  });

  it("names both years across a year boundary", () => {
    expect(
      formatInclusiveDateRange({ start: utc("2025-12-20"), end: utc("2026-01-08") }, "UTC"),
    ).toBe("Dec 20, 2025–Jan 7, 2026");
  });

  it("reads dates in the project's zone", () => {
    // Midnight UTC on Mar 1 is still Feb 28 in New York.
    const range = { start: utc("2026-03-01") + 5 * 3600, end: utc("2026-03-08") + 5 * 3600 };
    expect(formatInclusiveDateRange(range, "America/New_York")).toBe("Mar 1–7, 2026");
    expect(formatInclusiveDateRange(range, "UTC")).toBe("Mar 1–8, 2026");
  });
});

describe("comparison notes", () => {
  const range = { start: utc("2026-03-01"), end: utc("2026-03-08") };

  it("describes the prior-30-day baseline", () => {
    expect(formatPeriodComparisonNote(periodComparisonFromRange(range), "UTC")).toBe(
      "Percent change vs the prior 30 days (Jan 30–Feb 28, 2026), scaled to the same duration as the selected period.",
    );
  });

  it("describes MRR as start vs end of the range", () => {
    expect(formatMrrComparisonNote(range, "UTC")).toContain("(Mar 1–7, 2026)");
  });

  it("names the inclusive first and last day", () => {
    expect(
      formatCrawlersViewNote({ start: utc("2026-03-01"), end: utc("2026-03-01") + DAY }, "UTC"),
    ).toBe("Displaying crawler views between Mar 1, 2026 and Mar 1, 2026.");
  });
});
