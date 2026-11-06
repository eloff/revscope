import { describe, expect, it } from "vitest";
import {
  addCivilDays,
  addZonedDays,
  civilDateKey,
  civilToUnix,
  civilWeekday,
  formatTimeZoneLabel,
  formatUtcOffset,
  startOfZonedDay,
  startOfZonedDayFrom,
  startOfZonedMonth,
  startOfZonedWeek,
  zonedDate,
  zonedDateTime,
} from "./timezone";

/** Unix seconds for a UTC wall-clock time. */
function utc(iso: string): number {
  return Date.parse(`${iso}Z`) / 1000;
}

describe("zonedDateTime", () => {
  it("reads the wall clock in the zone", () => {
    expect(zonedDateTime("America/New_York", utc("2026-01-15T03:30:00"))).toEqual({
      year: 2026,
      month: 1,
      day: 14,
      hour: 22,
      minute: 30,
      second: 0,
    });
    expect(zonedDate("Asia/Tokyo", utc("2026-01-15T20:00:00"))).toEqual({
      year: 2026,
      month: 1,
      day: 16,
    });
  });

  it("falls back to UTC for a zone it cannot resolve", () => {
    expect(zonedDate("Not/AZone", utc("2026-01-15T23:00:00"))).toEqual({
      year: 2026,
      month: 1,
      day: 15,
    });
  });
});

describe("civilToUnix", () => {
  it("round-trips through zonedDateTime on ordinary days", () => {
    const civil = { year: 2026, month: 7, day: 4, hour: 9, minute: 15, second: 30 };
    for (const zone of ["UTC", "America/Los_Angeles", "Asia/Kolkata", "Australia/Adelaide"]) {
      expect(zonedDateTime(zone, civilToUnix(zone, civil))).toEqual(civil);
    }
  });

  it("uses the offset in force on either side of a DST change", () => {
    // New York springs forward on 2026-03-08 at 02:00 local.
    expect(
      civilToUnix("America/New_York", {
        year: 2026,
        month: 3,
        day: 8,
        hour: 1,
        minute: 0,
        second: 0,
      }),
    ).toBe(utc("2026-03-08T06:00:00"));
    expect(
      civilToUnix("America/New_York", {
        year: 2026,
        month: 3,
        day: 8,
        hour: 3,
        minute: 0,
        second: 0,
      }),
    ).toBe(utc("2026-03-08T07:00:00"));
  });
});

describe("day boundaries", () => {
  it("starts a day at local midnight", () => {
    expect(startOfZonedDay("America/New_York", utc("2026-01-15T03:30:00"))).toBe(
      utc("2026-01-14T05:00:00"),
    );
    expect(startOfZonedDay("UTC", utc("2026-01-15T03:30:00"))).toBe(utc("2026-01-15T00:00:00"));
  });

  it("steps by calendar days, so a 23-hour DST day does not shift later boundaries", () => {
    const zone = "America/New_York";
    const before = startOfZonedDayFrom(zone, { year: 2026, month: 3, day: 7 });
    const after = addZonedDays(zone, before, 2);
    expect(after).toBe(utc("2026-03-09T04:00:00"));
    expect(after - before).toBe(2 * 86_400 - 3_600);
  });

  it("starts a day that has no midnight at its first real hour", () => {
    // Santiago springs forward at 00:00 on 2026-09-06, so the day starts at 01:00.
    const start = startOfZonedDayFrom("America/Santiago", { year: 2026, month: 9, day: 6 });
    expect(zonedDateTime("America/Santiago", start)).toEqual({
      year: 2026,
      month: 9,
      day: 6,
      hour: 1,
      minute: 0,
      second: 0,
    });
  });

  it("starts weeks on Monday and months on the 1st", () => {
    // 2026-01-15 is a Thursday.
    expect(startOfZonedWeek("UTC", utc("2026-01-15T12:00:00"))).toBe(utc("2026-01-12T00:00:00"));
    // A Sunday belongs to the week that began six days earlier.
    expect(startOfZonedWeek("UTC", utc("2026-01-18T12:00:00"))).toBe(utc("2026-01-12T00:00:00"));
    expect(startOfZonedMonth("Europe/Berlin", utc("2026-01-15T12:00:00"))).toBe(
      utc("2025-12-31T23:00:00"),
    );
  });
});

describe("civil date arithmetic", () => {
  it("crosses month and year ends and leap days", () => {
    expect(addCivilDays({ year: 2026, month: 12, day: 31 }, 1)).toEqual({
      year: 2027,
      month: 1,
      day: 1,
    });
    expect(addCivilDays({ year: 2028, month: 3, day: 1 }, -1)).toEqual({
      year: 2028,
      month: 2,
      day: 29,
    });
    expect(civilDateKey({ year: 2026, month: 3, day: 7 })).toBe("2026-03-07");
    expect(civilWeekday({ year: 2026, month: 1, day: 18 })).toBe(0);
  });
});

describe("offset labels", () => {
  const winter = new Date("2026-01-15T12:00:00Z");
  const summer = new Date("2026-07-15T12:00:00Z");

  it("formats whole, half and quarter hour offsets", () => {
    expect(formatUtcOffset("America/Los_Angeles", winter)).toBe("UTC-8");
    expect(formatUtcOffset("America/Los_Angeles", summer)).toBe("UTC-7");
    expect(formatUtcOffset("Asia/Kolkata", winter)).toBe("UTC+5.5");
    expect(formatUtcOffset("Asia/Kathmandu", winter)).toBe("UTC+5.75");
    expect(formatUtcOffset("America/St_Johns", winter)).toBe("UTC-3.5");
  });

  it("omits a zero offset from the label", () => {
    expect(formatTimeZoneLabel("UTC", winter)).toBe("UTC");
    expect(formatTimeZoneLabel("Europe/London", winter)).toBe("Europe/London");
    expect(formatTimeZoneLabel("Europe/London", summer)).toBe("Europe/London (UTC+1)");
  });
});
