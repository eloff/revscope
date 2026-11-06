/**
 * Calendar arithmetic in a project's IANA timezone.
 *
 * The dashboard sends date ranges to the API as absolute instants, so the
 * timezone matters at exactly one point on this side: turning a calendar
 * intention ("today", "the 1st of this month", the day the user clicked in the
 * picker) into the instant that day begins in the project's zone. Get that wrong
 * and the range no longer lines up with the buckets the server draws from it.
 *
 * There is no native way to convert a civil date to an instant in a named zone,
 * so offsets are read back out of `Intl.DateTimeFormat`, which is the one place
 * the browser exposes the IANA database. Formatters are cached because
 * constructing them is much more expensive than using them.
 */

/** A calendar date with no timezone attached. */
export type CivilDate = {
  year: number;
  /** 1-12. */
  month: number;
  day: number;
};

/** A calendar date and wall-clock time with no timezone attached. */
export type CivilDateTime = CivilDate & {
  hour: number;
  minute: number;
  second: number;
};

const HOUR_SECONDS = 3_600;
export const UTC_TIME_ZONE = "UTC";

const partsFormatters = new Map<string, Intl.DateTimeFormat>();

function partsFormatter(timeZone: string): Intl.DateTimeFormat {
  const cached = partsFormatters.get(timeZone);
  if (cached) {
    return cached;
  }
  let formatter: Intl.DateTimeFormat;
  try {
    formatter = newPartsFormatter(timeZone);
  } catch {
    // A zone this browser cannot resolve would otherwise throw on every date the
    // page renders. UTC is always available, and matches the server's own
    // fallback for a zone it cannot load.
    formatter = newPartsFormatter(UTC_TIME_ZONE);
  }
  partsFormatters.set(timeZone, formatter);
  return formatter;
}

function newPartsFormatter(timeZone: string): Intl.DateTimeFormat {
  return new Intl.DateTimeFormat("en-US", {
    timeZone,
    hourCycle: "h23",
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

/** The wall-clock date and time `unixSeconds` reads as in `timeZone`. */
export function zonedDateTime(timeZone: string, unixSeconds: number): CivilDateTime {
  const parts = partsFormatter(timeZone).formatToParts(new Date(unixSeconds * 1000));
  const values = new Map<string, number>();
  for (const part of parts) {
    if (part.type !== "literal") {
      values.set(part.type, Number(part.value));
    }
  }
  return {
    year: values.get("year") ?? 1970,
    month: values.get("month") ?? 1,
    day: values.get("day") ?? 1,
    // Some engines render midnight as hour 24 rather than 0.
    hour: (values.get("hour") ?? 0) % 24,
    minute: values.get("minute") ?? 0,
    second: values.get("second") ?? 0,
  };
}

/** The calendar date `unixSeconds` falls on in `timeZone`. */
export function zonedDate(timeZone: string, unixSeconds: number): CivilDate {
  const { year, month, day } = zonedDateTime(timeZone, unixSeconds);
  return { year, month, day };
}

/** Reads a civil date-time as if it were UTC, giving a naive instant. */
function civilAsUtcSeconds(civil: CivilDateTime): number {
  return Math.floor(
    Date.UTC(civil.year, civil.month - 1, civil.day, civil.hour, civil.minute, civil.second) / 1000,
  );
}

/** How far ahead of UTC `timeZone` is at `unixSeconds`, in seconds. */
function offsetSeconds(timeZone: string, unixSeconds: number): number {
  return civilAsUtcSeconds(zonedDateTime(timeZone, unixSeconds)) - unixSeconds;
}

function offsetMinutes(timeZone: string, at: Date): number {
  return Math.round(offsetSeconds(timeZone, Math.floor(at.getTime() / 1000)) / 60);
}

/**
 * `UTC-7`, `UTC+5.5`, or `UTC+5.75`: the offset `timeZone` observes at `at`.
 * Whole hours have no decimal part, and fractional hours drop trailing zeros.
 */
export function formatUtcOffset(timeZone: string, at: Date = new Date()): string {
  return formatOffsetMinutes(offsetMinutes(timeZone, at));
}

function formatOffsetMinutes(minutes: number): string {
  const sign = minutes < 0 ? "-" : "+";
  const absolute = Math.abs(minutes);
  const wholeHours = Math.floor(absolute / 60);
  const remainder = absolute % 60;
  if (remainder === 0) {
    return `UTC${sign}${wholeHours}`;
  }
  // Quarter hours land on exact hundredths: 15 → 25, 30 → 50, 45 → 75.
  const hundredths = Math.round((remainder * 100) / 60);
  const fraction = String(hundredths).padStart(2, "0").replace(/0+$/, "");
  return `UTC${sign}${wholeHours}.${fraction}`;
}

/**
 * IANA name plus the offset in effect at `at`, e.g. `America/Los_Angeles (UTC-7)`.
 * A zero offset is omitted, so `UTC` stays `UTC`.
 */
export function formatTimeZoneLabel(timeZone: string, at: Date = new Date()): string {
  const minutes = offsetMinutes(timeZone, at);
  if (minutes === 0) {
    return timeZone;
  }
  return `${timeZone} (${formatOffsetMinutes(minutes)})`;
}

/**
 * The instant at which `civil` occurs in `timeZone`.
 *
 * The offset to apply depends on the instant being solved for, so the naive
 * instant supplies a first guess and the offset actually in force there
 * supplies the answer. Two passes settle every real transition.
 */
export function civilToUnix(timeZone: string, civil: CivilDateTime): number {
  const naive = civilAsUtcSeconds(civil);
  const guess = naive - offsetSeconds(timeZone, naive);
  return naive - offsetSeconds(timeZone, guess);
}

export function compareCivilDates(left: CivilDate, right: CivilDate): number {
  return left.year - right.year || left.month - right.month || left.day - right.day;
}

/** `YYYY-MM-DD`, the form the API uses for calendar-day keys. */
export function civilDateKey(date: CivilDate): string {
  const month = String(date.month).padStart(2, "0");
  const day = String(date.day).padStart(2, "0");
  return `${date.year}-${month}-${day}`;
}

/** Pure calendar arithmetic on a civil date; no zone involved. */
export function addCivilDays(date: CivilDate, days: number): CivilDate {
  const shifted = new Date(Date.UTC(date.year, date.month - 1, date.day));
  shifted.setUTCDate(shifted.getUTCDate() + days);
  return {
    year: shifted.getUTCFullYear(),
    month: shifted.getUTCMonth() + 1,
    day: shifted.getUTCDate(),
  };
}

/** Day of the week for a civil date, 0 = Sunday. */
export function civilWeekday(date: CivilDate): number {
  return new Date(Date.UTC(date.year, date.month - 1, date.day)).getUTCDay();
}

/** The instant `date` begins in `timeZone`. */
export function startOfZonedDayFrom(timeZone: string, date: CivilDate): number {
  let instant = civilToUnix(timeZone, { ...date, hour: 0, minute: 0, second: 0 });
  // A zone that springs forward at midnight has no 00:00 on that date, so the
  // naive answer lands on the day before. Step up to the first hour that does
  // exist, which is the boundary the server buckets on.
  for (
    let attempt = 0;
    attempt < 4 && compareCivilDates(zonedDate(timeZone, instant), date) < 0;
    attempt += 1
  ) {
    instant += HOUR_SECONDS;
  }
  return instant;
}

/** The instant the calendar day containing `unixSeconds` begins in `timeZone`. */
export function startOfZonedDay(timeZone: string, unixSeconds: number): number {
  return startOfZonedDayFrom(timeZone, zonedDate(timeZone, unixSeconds));
}

/**
 * The start of the calendar day `days` after the one containing `unixSeconds`.
 *
 * Stepping by calendar days rather than by 86,400 seconds is what keeps a
 * 23-hour DST day from shifting every later boundary by an hour.
 */
export function addZonedDays(timeZone: string, unixSeconds: number, days: number): number {
  return startOfZonedDayFrom(timeZone, addCivilDays(zonedDate(timeZone, unixSeconds), days));
}

/** Exclusive end of the calendar day containing `unixSeconds`. */
export function endOfZonedDayExclusive(timeZone: string, unixSeconds: number): number {
  return addZonedDays(timeZone, unixSeconds, 1);
}

/** The instant the Monday-based week containing `unixSeconds` begins. */
export function startOfZonedWeek(timeZone: string, unixSeconds: number): number {
  const date = zonedDate(timeZone, unixSeconds);
  const daysSinceMonday = (civilWeekday(date) + 6) % 7;
  return startOfZonedDayFrom(timeZone, addCivilDays(date, -daysSinceMonday));
}

/** The instant the calendar month containing `unixSeconds` begins. */
export function startOfZonedMonth(timeZone: string, unixSeconds: number): number {
  const { year, month } = zonedDate(timeZone, unixSeconds);
  return startOfZonedDayFrom(timeZone, { year, month, day: 1 });
}
