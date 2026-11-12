import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  clampRangeToMaxDays,
  clampRangeToRetention,
  dateValueToUnixEndExclusive,
  dateValueToUnixStart,
  isDateRangeReady,
  MAX_RANGE_DAYS,
  oldestRetainedStart,
  periodInTimeZone,
  rangeDayCount,
  rangeEndIncludesCurrentDay,
  rangeInTimeZone,
  shiftRange,
  shortcutForRange,
  shortcutToRange,
} from "./dateRange";

function utc(iso: string): number {
  return Date.parse(`${iso}Z`) / 1000;
}

const DAY = 86_400;

describe("shortcutToRange", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    // A Thursday, 22:30 in New York and already Friday in UTC.
    vi.setSystemTime(new Date("2026-01-16T03:30:00Z"));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("anchors days on the project's calendar, not UTC's", () => {
    expect(shortcutToRange("today", "America/New_York")).toEqual({
      start: utc("2026-01-15T05:00:00"),
      end: utc("2026-01-16T05:00:00"),
    });
    expect(shortcutToRange("today", "UTC")).toEqual({
      start: utc("2026-01-16T00:00:00"),
      end: utc("2026-01-17T00:00:00"),
    });
  });

  it("covers whole days ending with today", () => {
    const zone = "America/New_York";
    expect(shortcutToRange("yesterday", zone)).toEqual({
      start: utc("2026-01-14T05:00:00"),
      end: utc("2026-01-15T05:00:00"),
    });
    expect(shortcutToRange("last_7_days", zone)).toEqual({
      start: utc("2026-01-08T05:00:00"),
      end: utc("2026-01-16T05:00:00"),
    });
    expect(shortcutToRange("week_to_date", zone).start).toBe(utc("2026-01-12T05:00:00"));
    expect(shortcutToRange("month_to_date", zone).start).toBe(utc("2026-01-01T05:00:00"));
  });

  it("makes the last 24 hours a sliding window", () => {
    const now = utc("2026-01-16T03:30:00");
    expect(shortcutToRange("last_24_hours", "Asia/Tokyo")).toEqual({ start: now - DAY, end: now });
  });

  it("counts back calendar days across a DST change", () => {
    vi.setSystemTime(new Date("2026-03-20T16:00:00Z"));
    const range = shortcutToRange("last_30_days", "America/New_York");
    // Thirty-one calendar days, one of them 23 hours long.
    expect(range.start).toBe(utc("2026-02-18T05:00:00"));
    expect(range.end).toBe(utc("2026-03-21T04:00:00"));
    expect(rangeDayCount(range)).toBe(31);
  });

  it("treats custom as the default range", () => {
    expect(shortcutToRange("custom", "UTC")).toEqual(shortcutToRange("last_30_days", "UTC"));
  });

  it("knows whether a range reaches into today", () => {
    const today = shortcutToRange("today", "America/New_York");
    expect(rangeEndIncludesCurrentDay("America/New_York", today.end)).toBe(true);
    expect(rangeEndIncludesCurrentDay("America/New_York", today.start)).toBe(false);
  });
});

describe("range helpers", () => {
  it("clamps an over-long range by moving its start", () => {
    const end = utc("2026-01-01T00:00:00");
    const long = { start: end - 400 * DAY, end };
    expect(clampRangeToMaxDays(long)).toEqual({ start: end - MAX_RANGE_DAYS * DAY, end });
    const ok = { start: end - 30 * DAY, end };
    expect(clampRangeToMaxDays(ok)).toBe(ok);
  });

  it("is ready only once the end is after the start", () => {
    expect(isDateRangeReady({ start: 0, end: 0 })).toBe(false);
    expect(isDateRangeReady({ start: 1, end: 2 })).toBe(true);
    expect(rangeDayCount({ start: 0, end: 0 })).toBe(1);
  });

  it("turns a picked date into that whole day in the zone", () => {
    const picked = { year: 2026, month: 3, day: 8 };
    expect(dateValueToUnixStart("America/New_York", picked)).toBe(utc("2026-03-08T05:00:00"));
    expect(dateValueToUnixEndExclusive("America/New_York", picked)).toBe(
      utc("2026-03-09T04:00:00"),
    );
  });
});

describe("shiftRange", () => {
  it("moves a day-aligned range by whole days, keeping midnight across DST", () => {
    // New York midnights: 1 Mar is UTC-5, 1 Apr is UTC-4 after spring forward.
    const march = { start: utc("2026-03-01T05:00:00"), end: utc("2026-04-01T04:00:00") };
    expect(shiftRange("America/New_York", march, -1)).toEqual({
      start: utc("2026-01-29T05:00:00"),
      end: utc("2026-03-01T05:00:00"),
    });
    expect(shiftRange("America/New_York", march, 1)).toEqual({
      start: utc("2026-04-01T04:00:00"),
      end: utc("2026-05-02T04:00:00"),
    });
  });

  it("slides a window that isn't on day boundaries by its length", () => {
    const window = { start: utc("2026-03-01T13:30:00"), end: utc("2026-03-02T13:30:00") };
    expect(shiftRange("UTC", window, -1)).toEqual({
      start: utc("2026-02-28T13:30:00"),
      end: utc("2026-03-01T13:30:00"),
    });
  });
});

describe("rangeInTimeZone", () => {
  it("keeps the calendar days and moves to the other zone's midnights", () => {
    const range = { start: utc("2026-03-01T00:00:00"), end: utc("2026-03-08T00:00:00") };
    expect(rangeInTimeZone(range, "UTC", "Asia/Tokyo")).toEqual({
      start: utc("2026-02-28T15:00:00"),
      end: utc("2026-03-07T15:00:00"),
    });
    expect(rangeInTimeZone(range, "UTC", "UTC")).toBe(range);
  });

  it("leaves a sliding window on the same instants", () => {
    const lastDay = { start: utc("2026-10-31T12:00:00"), end: utc("2026-11-01T12:00:00") };
    expect(rangeInTimeZone(lastDay, "UTC", "Asia/Tokyo")).toBe(lastDay);
  });

  it("follows a daylight saving change inside the range", () => {
    // New York springs forward on 2026-03-08.
    const range = { start: utc("2026-03-07T00:00:00"), end: utc("2026-03-10T00:00:00") };
    expect(rangeInTimeZone(range, "UTC", "America/New_York")).toEqual({
      start: utc("2026-03-07T05:00:00"),
      end: utc("2026-03-10T04:00:00"),
    });
  });
});

describe("shortcutForRange", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-01-16T03:30:00Z"));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("names the preset a range came from in that zone", () => {
    const zone = "America/New_York";
    expect(shortcutForRange(shortcutToRange("yesterday", zone), zone)).toBe("yesterday");
    expect(shortcutForRange(shortcutToRange("last_30_days", zone), zone)).toBe("last_30_days");
    const window = shortcutToRange("last_24_hours", zone);
    vi.setSystemTime(new Date("2026-01-16T03:30:05Z"));
    expect(shortcutForRange(window, zone)).toBe("last_24_hours");
  });

  it("calls anything else custom", () => {
    expect(
      shortcutForRange(
        { start: utc("2026-01-02T00:00:00"), end: utc("2026-01-05T00:00:00") },
        "UTC",
      ),
    ).toBe("custom");
    // Yesterday on Tokyo's days is not yesterday in New York.
    expect(shortcutForRange(shortcutToRange("yesterday", "Asia/Tokyo"), "America/New_York")).toBe(
      "custom",
    );
  });
});

describe("periodInTimeZone", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    // 10:00 on Nov 1 in Tokyo, still Oct 31 in Los Angeles.
    vi.setSystemTime(new Date("2026-11-01T01:00:00Z"));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("works a preset out again on the other project's clock", () => {
    const today = shortcutToRange("today", "Asia/Tokyo");
    expect(periodInTimeZone(today, "Asia/Tokyo", "America/Los_Angeles")).toEqual(
      shortcutToRange("today", "America/Los_Angeles"),
    );
    const month = shortcutToRange("last_30_days", "Asia/Tokyo");
    expect(periodInTimeZone(month, "Asia/Tokyo", "America/Los_Angeles")).toEqual(
      shortcutToRange("last_30_days", "America/Los_Angeles"),
    );
  });

  it("keeps a hand-picked range on its calendar days", () => {
    const picked = { start: utc("2026-10-01T00:00:00"), end: utc("2026-10-04T00:00:00") };
    expect(periodInTimeZone(picked, "UTC", "Asia/Tokyo")).toEqual(
      rangeInTimeZone(picked, "UTC", "Asia/Tokyo"),
    );
  });
});

describe("retention", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    // Already 11 November in UTC, still the 10th in New York.
    vi.setSystemTime(new Date("2026-11-11T03:00:00Z"));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("starts on the same day N months back, in the project's zone", () => {
    expect(oldestRetainedStart("UTC", 3)).toBe(utc("2026-08-11T00:00:00"));
    expect(oldestRetainedStart("America/New_York", 3)).toBe(utc("2026-08-10T04:00:00"));
    expect(oldestRetainedStart("UTC", null)).toBeNull();
  });

  it("moves a range's start up to the oldest day kept", () => {
    const oldest = utc("2026-08-11T00:00:00");
    const range = { start: utc("2026-01-01T00:00:00"), end: utc("2026-11-12T00:00:00") };
    expect(clampRangeToRetention("UTC", range, oldest)).toEqual({ start: oldest, end: range.end });
    const inside = { start: utc("2026-10-01T00:00:00"), end: range.end };
    expect(clampRangeToRetention("UTC", inside, oldest)).toBe(inside);
    expect(clampRangeToRetention("UTC", range, null)).toBe(range);
  });

  it("turns a range wholly before retention into its oldest day", () => {
    const oldest = utc("2026-08-11T00:00:00");
    const range = { start: utc("2026-01-01T00:00:00"), end: utc("2026-02-01T00:00:00") };
    expect(clampRangeToRetention("UTC", range, oldest)).toEqual({
      start: oldest,
      end: oldest + DAY,
    });
  });
});
