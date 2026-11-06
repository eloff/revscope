import { describe, expect, it } from "vitest";
import {
  emptySpendRow,
  formatRoas,
  formatSpendDate,
  type SpendDraftRow,
  spendRowsFromEntries,
  validateSpendRows,
} from "./campaignSpendLogic";

function once(startDate: string, amount: string): SpendDraftRow {
  return { kind: "once", startDate, endDate: "", amount };
}

function daily(startDate: string, endDate: string, amount: string): SpendDraftRow {
  return { kind: "daily", startDate, endDate, amount };
}

describe("validateSpendRows", () => {
  it("converts dollars to thousandths and drops blank rows", () => {
    expect(
      validateSpendRows([
        once("2026-01-01", "$1,250.5"),
        emptySpendRow(),
        daily("2026-02-01", "2026-02-10", ".125"),
      ]),
    ).toEqual({
      ok: true,
      entries: [
        { kind: "once", start_date: "2026-01-01", amount: 1_250_500 },
        { kind: "daily", start_date: "2026-02-01", end_date: "2026-02-10", amount: 125 },
      ],
    });
  });

  it.each([
    [[once("", "5")], 0, "Spend 1: choose a date."],
    [[emptySpendRow(), daily("", "2026-01-02", "5")], 1, "Spend 2: choose a start date."],
    [[once("2026-01-01", "0")], 0, "Spend 1: enter an amount greater than 0."],
    [[once("2026-01-01", "1.2345")], 0, "Spend 1: enter an amount greater than 0."],
    [[once("2026-01-01", "abc")], 0, "Spend 1: enter an amount greater than 0."],
    [[daily("2026-01-01", "", "5")], 0, "Spend 1: choose an end date."],
    [
      [daily("2026-01-05", "2026-01-01", "5")],
      0,
      "Spend 1: the end date is before the start date.",
    ],
  ])("rejects %j at row %d", (rows, row, error) => {
    expect(validateSpendRows(rows)).toEqual({ ok: false, row, error });
  });

  it("rejects daily ranges that share a day, naming both rows", () => {
    expect(
      validateSpendRows([
        daily("2026-03-10", "2026-03-20", "5"),
        daily("2026-03-01", "2026-03-10", "5"),
      ]),
    ).toEqual({ ok: false, row: 0, error: "Spend 1 and spend 2 cover some of the same days." });
  });

  it("allows adjacent daily ranges and one-off spend inside a range", () => {
    expect(
      validateSpendRows([
        daily("2026-03-01", "2026-03-09", "5"),
        daily("2026-03-10", "2026-03-20", "5"),
        once("2026-03-05", "5"),
      ]).ok,
    ).toBe(true);
  });
});

describe("spend formatting", () => {
  it("round-trips saved entries into editable rows", () => {
    expect(
      spendRowsFromEntries([
        {
          id: 1,
          kind: "once",
          start_date: "2026-01-01",
          end_date: "2026-01-01",
          amount: 2500,
          spent_to_date: 0,
        },
        {
          id: 2,
          kind: "daily",
          start_date: "2026-01-01",
          end_date: "2026-01-31",
          amount: 10_000,
          spent_to_date: 0,
        },
      ]),
    ).toEqual([once("2026-01-01", "2.5"), daily("2026-01-01", "2026-01-31", "10")]);
  });

  it("formats a calendar date without shifting it by timezone", () => {
    expect(formatSpendDate("2026-03-01")).toBe(
      new Date(Date.UTC(2026, 2, 1)).toLocaleDateString(undefined, {
        month: "short",
        day: "numeric",
        year: "numeric",
        timeZone: "UTC",
      }),
    );
  });

  it("formats ROAS", () => {
    expect(formatRoas(2.456)).toBe("2.46×");
    expect(formatRoas(null)).toBe("—");
  });
});
