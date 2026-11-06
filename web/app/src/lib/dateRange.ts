import {
  addZonedDays,
  type CivilDate,
  endOfZonedDayExclusive,
  startOfZonedDay,
  startOfZonedDayFrom,
  startOfZonedMonth,
  startOfZonedWeek,
  zonedDate,
} from "./timezone";

export type DateRangeSeconds = {
  start: number;
  end: number;
};

const DAY_SECONDS = 86_400;

/**
 * The same calendar days in another zone. A range is picked on the selected
 * project's day boundaries; another project's days start at other instants.
 * A range that isn't whole days, such as the sliding last 24 hours, names the
 * same instants everywhere, so it is returned as is.
 */
export function rangeInTimeZone(
  range: DateRangeSeconds,
  fromTimeZone: string,
  toTimeZone: string,
): DateRangeSeconds {
  const wholeDays =
    startOfZonedDay(fromTimeZone, range.start) === range.start &&
    startOfZonedDay(fromTimeZone, range.end) === range.end;
  if (fromTimeZone === toTimeZone || !wholeDays) {
    return range;
  }
  return {
    start: startOfZonedDayFrom(toTimeZone, zonedDate(fromTimeZone, range.start)),
    end: startOfZonedDayFrom(toTimeZone, zonedDate(fromTimeZone, range.end)),
  };
}

export const MAX_RANGE_DAYS = 366;

/** Stable SSR placeholder — real ranges are filled in onMount from `Date.now()`. */
export const EMPTY_DATE_RANGE: DateRangeSeconds = { start: 0, end: 0 };

export function isDateRangeReady(range: DateRangeSeconds): boolean {
  return range.end > range.start;
}

/** Calendar days in an exclusive-end range (`end` is start of the day after the last included day). */
export function rangeDayCount(range: DateRangeSeconds): number {
  return Math.max(1, Math.round((range.end - range.start) / DAY_SECONDS));
}

/** Keep the end fixed and move the start forward when the range exceeds {@link MAX_RANGE_DAYS}. */
export function clampRangeToMaxDays(range: DateRangeSeconds): DateRangeSeconds {
  const maxDuration = MAX_RANGE_DAYS * DAY_SECONDS;
  if (range.end - range.start <= maxDuration) {
    return range;
  }
  return {
    start: range.end - maxDuration,
    end: range.end,
  };
}

/**
 * The adjacent period of the same length, before (`-1`) or after (`1`) `range`.
 * Ranges on day boundaries move by calendar days, so a period spanning a DST
 * change still starts and ends at midnight; sliding windows (Last 24 Hours)
 * move by their exact length.
 */
export function shiftRange(
  timeZone: string,
  range: DateRangeSeconds,
  direction: -1 | 1,
): DateRangeSeconds {
  const dayAligned =
    startOfZonedDay(timeZone, range.start) === range.start &&
    startOfZonedDay(timeZone, range.end) === range.end;
  if (dayAligned) {
    const days = direction * rangeDayCount(range);
    return {
      start: addZonedDays(timeZone, range.start, days),
      end: addZonedDays(timeZone, range.end, days),
    };
  }
  const duration = Math.max(DAY_SECONDS, range.end - range.start);
  return {
    start: range.start + direction * duration,
    end: range.end + direction * duration,
  };
}

export type DateShortcut =
  | "today"
  | "yesterday"
  | "last_24_hours"
  | "last_7_days"
  | "last_30_days"
  | "last_60_days"
  | "last_90_days"
  | "week_to_date"
  | "month_to_date"
  | "custom";

export const DATE_SHORTCUT_OPTIONS: { value: DateShortcut; label: string }[] = [
  { value: "today", label: "Today" },
  { value: "yesterday", label: "Yesterday" },
  { value: "last_24_hours", label: "Last 24 Hours" },
  { value: "last_7_days", label: "Last 7 Days" },
  { value: "last_30_days", label: "Last 30 Days" },
  { value: "last_60_days", label: "Last 60 Days" },
  { value: "last_90_days", label: "Last 90 Days" },
  { value: "week_to_date", label: "Week to Date" },
  { value: "month_to_date", label: "Month to Date" },
];

export const DEFAULT_DATE_SHORTCUT: DateShortcut = "last_30_days";

/**
 * The period another project should show for the one selected here. A preset
 * ("Today", "Last 30 days") means that preset on the other project's own clock,
 * so it is worked out afresh there; a hand-picked range keeps its calendar days.
 */
export function periodInTimeZone(
  range: DateRangeSeconds,
  fromTimeZone: string,
  toTimeZone: string,
): DateRangeSeconds {
  const preset = shortcutForRange(range, fromTimeZone);
  return preset === "custom"
    ? rangeInTimeZone(range, fromTimeZone, toTimeZone)
    : shortcutToRange(preset, toTimeZone);
}

/** Slack for a sliding preset computed a moment before the comparison. */
const SLIDING_RANGE_TOLERANCE_SECONDS = 60;

/**
 * The preset that produces `range` today in `timeZone`, or `custom`. Lets a
 * picker opened on a range it didn't pick, such as one carried over from
 * another project, label it correctly.
 */
export function shortcutForRange(
  range: DateRangeSeconds,
  timeZone: string,
): DateShortcut | "custom" {
  for (const { value } of DATE_SHORTCUT_OPTIONS) {
    const candidate = shortcutToRange(value, timeZone);
    if (value === "last_24_hours") {
      if (
        range.end - range.start === candidate.end - candidate.start &&
        Math.abs(range.end - candidate.end) <= SLIDING_RANGE_TOLERANCE_SECONDS
      ) {
        return value;
      }
    } else if (candidate.start === range.start && candidate.end === range.end) {
      return value;
    }
  }
  return "custom";
}

export function nowUnixSeconds(): number {
  return Math.floor(Date.now() / 1000);
}

/** Unix second at the start of the current calendar day in `timeZone`. */
export function currentDayStart(timeZone: string): number {
  return startOfZonedDay(timeZone, nowUnixSeconds());
}

/** True when `unixSeconds` is the start of the current calendar day in `timeZone`. */
export function isCurrentDay(timeZone: string, unixSeconds: number): boolean {
  return unixSeconds === currentDayStart(timeZone);
}

/** True when the exclusive range end includes all or part of today in `timeZone`. */
export function rangeEndIncludesCurrentDay(timeZone: string, endExclusive: number): boolean {
  return endExclusive > currentDayStart(timeZone);
}

/**
 * The instants a shortcut selects, with every boundary on a calendar boundary in
 * `timeZone` so the range lines up with the buckets the server draws for it.
 *
 * "Last N days" counts back N calendar days rather than N × 86,400 seconds, so a
 * DST transition inside the window cannot land the start mid-day.
 */
export function shortcutToRange(shortcut: DateShortcut, timeZone: string): DateRangeSeconds {
  const now = nowUnixSeconds();
  const todayStart = startOfZonedDay(timeZone, now);
  const endOfToday = endOfZonedDayExclusive(timeZone, now);

  switch (shortcut) {
    case "today":
      return { start: todayStart, end: endOfToday };
    case "yesterday":
      return { start: addZonedDays(timeZone, now, -1), end: todayStart };
    case "last_24_hours":
      return { start: now - DAY_SECONDS, end: now };
    case "last_7_days":
      return { start: addZonedDays(timeZone, now, -7), end: endOfToday };
    case "last_30_days":
      return { start: addZonedDays(timeZone, now, -30), end: endOfToday };
    case "last_60_days":
      return { start: addZonedDays(timeZone, now, -60), end: endOfToday };
    case "last_90_days":
      return { start: addZonedDays(timeZone, now, -90), end: endOfToday };
    case "week_to_date":
      return { start: startOfZonedWeek(timeZone, now), end: endOfToday };
    case "month_to_date":
      return { start: startOfZonedMonth(timeZone, now), end: endOfToday };
    case "custom":
      return shortcutToRange("last_30_days", timeZone);
  }
}

/** The date-picker's value shape, which is a civil date with no zone. */
export type DateValueLike = CivilDate;

export function dateValueToUnixStart(timeZone: string, value: DateValueLike): number {
  return startOfZonedDayFrom(timeZone, value);
}

export function dateValueToUnixEndExclusive(timeZone: string, value: DateValueLike): number {
  return addZonedDays(timeZone, dateValueToUnixStart(timeZone, value), 1);
}

export function unixStartToDateValue(timeZone: string, unixSeconds: number): DateValueLike {
  return zonedDate(timeZone, unixSeconds);
}

export function formatBucketLabel(
  unixSeconds: number,
  granularity: "hour" | "day" | "week",
  timeZone: string,
): string {
  const date = new Date(unixSeconds * 1000);
  if (granularity === "hour") {
    return date.toLocaleString(undefined, {
      month: "short",
      day: "numeric",
      hour: "numeric",
      timeZone,
    });
  }
  // Day and week buckets are both labelled by the bucket's start date.
  return date.toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    timeZone,
  });
}
