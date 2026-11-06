import { type DateRangeSeconds, rangeDayCount, unixStartToDateValue } from "./dateRange";

export type PeriodComparison = {
  current: DateRangeSeconds;
  previous: DateRangeSeconds;
  dayCount: number;
};

/** Length of the fixed comparison baseline, in seconds (30 days). */
export const COMPARISON_BASELINE_SECONDS = 30 * 24 * 60 * 60;

/**
 * Derive the comparison baseline: the 30 days immediately preceding the
 * selected range. Flow metrics (visitors, revenue, conversions, …) are scaled
 * by the backend to the current period's length so the two windows are
 * comparable; rates are already normalized and compared directly.
 */
export function periodComparisonFromRange(range: DateRangeSeconds): PeriodComparison {
  return {
    current: range,
    previous: {
      start: range.start - COMPARISON_BASELINE_SECONDS,
      end: range.start,
    },
    dayCount: rangeDayCount(range),
  };
}

function formatDate(unixSeconds: number, timeZone: string): string {
  const date = new Date(unixSeconds * 1000);
  return date.toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
    timeZone,
  });
}

function formatMonthDay(unixSeconds: number, timeZone: string): string {
  const date = new Date(unixSeconds * 1000);
  return date.toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    timeZone,
  });
}

/**
 * The range's first and last calendar dates, already formatted. `end` is
 * exclusive, so the last included instant is the second before it.
 */
function inclusiveBounds(range: DateRangeSeconds, timeZone: string): [string, string] {
  return [
    formatDate(range.start, timeZone),
    formatDate(Math.max(range.start, range.end - 1), timeZone),
  ];
}

/** Inclusive calendar dates for an exclusive-end range, read in `timeZone`. */
export function formatInclusiveDateRange(range: DateRangeSeconds, timeZone: string): string {
  const start = range.start;
  const endInclusive = Math.max(range.start, range.end - 1);
  if (rangeDayCount(range) === 1) {
    return formatDate(start, timeZone);
  }

  const startDate = unixStartToDateValue(timeZone, start);
  const endDate = unixStartToDateValue(timeZone, endInclusive);
  const sameYear = startDate.year === endDate.year;
  const sameMonth = sameYear && startDate.month === endDate.month;

  if (sameMonth) {
    return `${formatMonthDay(start, timeZone)}–${endDate.day}, ${endDate.year}`;
  }
  if (sameYear) {
    return `${formatMonthDay(start, timeZone)}–${formatMonthDay(endInclusive, timeZone)}, ${endDate.year}`;
  }
  return `${formatDate(start, timeZone)}–${formatDate(endInclusive, timeZone)}`;
}

export function formatPeriodComparisonNote(comparison: PeriodComparison, timeZone: string): string {
  const dates = formatInclusiveDateRange(comparison.previous, timeZone);
  return `Percent change vs the prior 30 days (${dates}), scaled to the same duration as the selected period.`;
}

/** MRR is a stock metric: the tile compares start vs end of the selected range. */
export function formatMrrComparisonNote(range: DateRangeSeconds, timeZone: string): string {
  const dates = formatInclusiveDateRange(range, timeZone);
  return `Percent change compares MRR at period end to MRR at period start (${dates}), not the prior 30 days.`;
}

/** Clarifies that Sources is keyed by first-visit time, not paid_at. */
export function formatRevenueSourcesSessionNote(range: DateRangeSeconds, timeZone: string): string {
  const [start, end] = inclusiveBounds(range, timeZone);
  return `Displaying sources for sessions that began between ${start} and ${end}.`;
}

/** Clarifies that Crawlers is keyed by crawler-view hour, not session start. */
export function formatCrawlersViewNote(range: DateRangeSeconds, timeZone: string): string {
  const [start, end] = inclusiveBounds(range, timeZone);
  return `Displaying crawler views between ${start} and ${end}.`;
}

/** Clarifies that Pages is keyed by pageview time in the selected range. */
export function formatPagesViewedNote(range: DateRangeSeconds, timeZone: string): string {
  const [start, end] = inclusiveBounds(range, timeZone);
  return `Displaying pages viewed between ${start} and ${end}.`;
}

/** Clarifies that Landing Pages is keyed by session start, not paid_at. */
export function formatLandingPagesSessionNote(range: DateRangeSeconds, timeZone: string): string {
  const [start, end] = inclusiveBounds(range, timeZone);
  return `Displaying landing pages for sessions that began between ${start} and ${end}.`;
}

/** Clarifies that the Visitors tab is keyed by session start, not last activity. */
export function formatVisitorsSessionNote(range: DateRangeSeconds, timeZone: string): string {
  const [start, end] = inclusiveBounds(range, timeZone);
  return `Displaying visitors whose sessions began between ${start} and ${end}.`;
}

/** Clarifies pageview/event visitor drill-ins are keyed by viewing the page in-range. */
export function formatVisitorsPageviewNote(range: DateRangeSeconds, timeZone: string): string {
  const [start, end] = inclusiveBounds(range, timeZone);
  return `Displaying visitors who viewed this page between ${start} and ${end}.`;
}

/** Clarifies that New Customers is keyed by purchase time (paid_at). */
export function formatCustomersPurchaseNote(range: DateRangeSeconds, timeZone: string): string {
  const [start, end] = inclusiveBounds(range, timeZone);
  return `Displaying customers who purchased between ${start} and ${end}.`;
}
