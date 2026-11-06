import type { ImportedCoverage, ImportedRange } from "../../lib/importedCoverageApi";
import { civilDateKey, zonedDate } from "../../lib/timezone";
import type { DashboardTab } from "./dashboardTabs";

/** Tabs built from individual sessions or payments, which imported data doesn't have. */
const TABS_WITHOUT_IMPORTED_DATA = new Set<DashboardTab>([
  "revenue",
  "revenue_sources",
  "campaigns",
  "visitors",
  "funnels",
]);

/** Tabs that add imported data, except when a filter narrows them. */
const TABS_WITH_IMPORTED_DATA = new Set<DashboardTab>(["overview", "audience", "pages", "events"]);

/** Whether this view's range has imported data that it isn't counting. */
export function importedDataLeftOut(
  coverage: ImportedCoverage | undefined,
  tab: DashboardTab,
  filtersActive: boolean,
): boolean {
  if (!coverage?.has_data || !coverage.include_in_reports) {
    return false;
  }
  if (tab === "events" && !coverage.has_events) {
    return false;
  }
  return TABS_WITHOUT_IMPORTED_DATA.has(tab) || (filtersActive && TABS_WITH_IMPORTED_DATA.has(tab));
}

/** The chart's buckets each import overlaps, as first and last index. */
export function importedBucketSpans(
  bucketStarts: number[],
  ranges: ImportedRange[],
  timeZone: string,
): { first: number; last: number }[] {
  const dates = bucketStarts.map((start) => civilDateKey(zonedDate(timeZone, start)));
  const spans: { first: number; last: number }[] = [];
  for (const range of ranges) {
    // A bucket runs up to the next one's start, so a week can hold the range's first
    // day; the last bucket's end isn't known, but ranges never start past the chart.
    const inRange = (index: number) =>
      dates[index] <= range.end_date &&
      (dates[index] >= range.start_date ||
        index === dates.length - 1 ||
        dates[index + 1] > range.start_date);
    const first = dates.findIndex((_, index) => inRange(index));
    if (first < 0) {
      continue;
    }
    let last = first;
    while (last + 1 < dates.length && inRange(last + 1)) {
      last += 1;
    }
    spans.push({ first, last });
  }
  return spans;
}
