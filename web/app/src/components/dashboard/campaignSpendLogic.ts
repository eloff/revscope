import type {
  CampaignSpendEntry,
  CampaignSpendInput,
  CampaignSpendKind,
} from "../../lib/campaignsApi";

/** API amounts are thousandths of the project currency. */
const AMOUNT_SCALE = 1000;
export const MAX_SPEND_ENTRIES = 50;

export type SpendDraftRow = {
  kind: CampaignSpendKind;
  startDate: string;
  endDate: string;
  /** Dollars as typed. */
  amount: string;
};

export type SpendValidation =
  | { ok: true; entries: CampaignSpendInput[] }
  | { ok: false; error: string; row?: number };

export function emptySpendRow(kind: CampaignSpendKind = "once"): SpendDraftRow {
  return { kind, startDate: "", endDate: "", amount: "" };
}

export function spendRowsFromEntries(entries: CampaignSpendEntry[]): SpendDraftRow[] {
  return entries.map((entry) => ({
    kind: entry.kind,
    startDate: entry.start_date,
    endDate: entry.kind === "daily" ? entry.end_date : "",
    amount: String(entry.amount / AMOUNT_SCALE),
  }));
}

function isBlank(row: SpendDraftRow): boolean {
  return !row.startDate && !row.endDate && !row.amount.trim();
}

function parseAmount(value: string): number | null {
  const cleaned = value.replace(/[$,\s]/g, "");
  if (!/^\d+(\.\d{0,3})?$|^\.\d{1,3}$/.test(cleaned)) {
    return null;
  }
  const amount = Math.round(Number(cleaned) * AMOUNT_SCALE);
  return amount > 0 ? amount : null;
}

/** Blank rows are dropped; daily ranges may not share a day. */
export function validateSpendRows(rows: SpendDraftRow[]): SpendValidation {
  const entries: CampaignSpendInput[] = [];
  const ranges: { start: string; end: string; row: number }[] = [];
  for (const [index, row] of rows.entries()) {
    if (isBlank(row)) {
      continue;
    }
    const label = `Spend ${index + 1}`;
    if (!row.startDate) {
      return {
        ok: false,
        row: index,
        error: `${label}: choose ${row.kind === "daily" ? "a start date" : "a date"}.`,
      };
    }
    const amount = parseAmount(row.amount);
    if (amount === null) {
      return { ok: false, row: index, error: `${label}: enter an amount greater than 0.` };
    }
    if (row.kind === "once") {
      entries.push({ kind: "once", start_date: row.startDate, amount });
      continue;
    }
    if (!row.endDate) {
      return { ok: false, row: index, error: `${label}: choose an end date.` };
    }
    if (row.endDate < row.startDate) {
      return { ok: false, row: index, error: `${label}: the end date is before the start date.` };
    }
    ranges.push({ start: row.startDate, end: row.endDate, row: index });
    entries.push({ kind: "daily", start_date: row.startDate, end_date: row.endDate, amount });
  }
  ranges.sort((left, right) => left.start.localeCompare(right.start));
  for (let index = 1; index < ranges.length; index += 1) {
    const previous = ranges[index - 1];
    const current = ranges[index];
    if (current.start <= previous.end) {
      const [first, second] = [previous.row, current.row].sort((a, b) => a - b);
      return {
        ok: false,
        row: current.row,
        error: `Spend ${first + 1} and spend ${second + 1} cover some of the same days.`,
      };
    }
  }
  return { ok: true, entries };
}

/** Formats a `YYYY-MM-DD` date without shifting it through the browser's timezone. */
export function formatSpendDate(date: string): string {
  const [year, month, day] = date.split("-").map(Number);
  return new Date(Date.UTC(year, month - 1, day)).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
    timeZone: "UTC",
  });
}

export function formatRoas(roas: number | null): string {
  return roas === null ? "—" : `${roas.toFixed(2)}×`;
}
