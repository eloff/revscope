import type { BreakdownRow, OverviewBreakdowns } from "../../lib/dashboardApi";
import {
  attributeFilterKey,
  type LandingAttribute,
  landingPageFilterKey,
  pageFilterKey,
  type SegmentFilters,
  type SourceFilter,
  sourceFilterKey,
  withAttributeFilter,
  withCountryFilter,
  withLandingPageFilter,
  withoutAttributeFilter,
  withoutCountryFilter,
  withoutLandingPageFilter,
  withoutPageFilter,
  withoutSourceFilter,
  withPageFilter,
  withSourceFilter,
} from "../../lib/segmentFilters";
import type { ColumnSort } from "./columnSort";
import { campaignDisplayLabel } from "./revenueSourcesLogic";

export type BreakdownKey = keyof OverviewBreakdowns;

export type BreakdownColumn = "label" | "visitors" | "revenue";

export type BreakdownSort = ColumnSort<BreakdownColumn>;

export const DEFAULT_BREAKDOWN_SORT: BreakdownSort = { column: "visitors", direction: "desc" };

export type BreakdownTab = { key: BreakdownKey; label: string };

export type BreakdownPanelConfig = { title: string; tabs: BreakdownTab[] };

export const BREAKDOWN_PANELS: BreakdownPanelConfig[] = [
  {
    title: "Sources",
    tabs: [
      { key: "sources", label: "Source" },
      { key: "channels", label: "Channel" },
      { key: "campaigns", label: "Campaign" },
    ],
  },
  {
    title: "Locations",
    tabs: [
      { key: "countries", label: "Country" },
      { key: "cities", label: "City" },
    ],
  },
  {
    title: "Pages",
    tabs: [
      { key: "pages", label: "Page" },
      { key: "landing_pages", label: "Landing page" },
    ],
  },
  {
    title: "Technology",
    tabs: [
      { key: "browsers", label: "Browser" },
      { key: "os", label: "OS" },
      { key: "devices", label: "Device" },
    ],
  },
];

const LANDING_ATTRIBUTE_FOR: Partial<Record<BreakdownKey, LandingAttribute>> = {
  channels: "channel",
  cities: "city",
  browsers: "browser",
  os: "os",
  devices: "device",
};

/** The country code region and city values start with (`US` in `US-NY`). */
export function breakdownRowCountry(key: BreakdownKey, row: BreakdownRow): string | undefined {
  if (key === "countries") {
    return row.value;
  }
  if (key === "cities") {
    return row.value.split("-", 1)[0];
  }
  return undefined;
}

export function breakdownRowLabel(row: BreakdownRow): string {
  return row.campaign ? campaignDisplayLabel(row.campaign) : row.label;
}

function sourceFilterForRow(row: BreakdownRow): SourceFilter | null {
  const name = breakdownRowLabel(row);
  if (row.value === "direct") {
    return { kind: "direct", name };
  }
  const [kind, id] = row.value.split("-", 2);
  const numericId = Number(id);
  if (!Number.isInteger(numericId)) {
    return null;
  }
  if (kind === "campaign" || kind === "referrer") {
    return { kind, id: numericId, name };
  }
  return null;
}

/** Whether the row's value is among the active filters. */
export function isBreakdownRowFiltered(
  segments: SegmentFilters,
  key: BreakdownKey,
  row: BreakdownRow,
): boolean {
  const attribute = LANDING_ATTRIBUTE_FOR[key];
  if (attribute) {
    const rowKey = attributeFilterKey({ attribute, value: row.value });
    return segments.attributes.some((filter) => attributeFilterKey(filter) === rowKey);
  }
  switch (key) {
    case "sources":
    case "campaigns":
      return segments.sources.some((source) => sourceFilterKey(source) === row.value);
    case "countries":
      return segments.countries.some((country) => country.code === row.value);
    case "pages":
      return segments.pages.some((page) => pageFilterKey(page) === row.value);
    case "landing_pages":
      return segments.landingPages.some((page) => landingPageFilterKey(page) === row.value);
    default:
      return false;
  }
}

/** Adds the row's value to the filters, or removes it when it is already there. */
export function toggleBreakdownFilter(
  segments: SegmentFilters,
  key: BreakdownKey,
  row: BreakdownRow,
): SegmentFilters {
  const active = isBreakdownRowFiltered(segments, key, row);
  const name = breakdownRowLabel(row);
  const attribute = LANDING_ATTRIBUTE_FOR[key];
  if (attribute) {
    return active
      ? withoutAttributeFilter(segments, attributeFilterKey({ attribute, value: row.value }))
      : withAttributeFilter(segments, { attribute, value: row.value, name });
  }
  switch (key) {
    case "sources":
    case "campaigns": {
      if (active) {
        return withoutSourceFilter(segments, row.value);
      }
      const source = sourceFilterForRow(row);
      return source ? withSourceFilter(segments, source) : segments;
    }
    case "countries":
      return active
        ? withoutCountryFilter(segments, row.value)
        : withCountryFilter(segments, row.value, name);
    case "pages":
    case "landing_pages": {
      const id = Number(row.value);
      if (!Number.isInteger(id)) {
        return segments;
      }
      if (key === "pages") {
        return active
          ? withoutPageFilter(segments, row.value)
          : withPageFilter(segments, { id, name });
      }
      return active
        ? withoutLandingPageFilter(segments, row.value)
        : withLandingPageFilter(segments, { id, name });
    }
    default:
      return segments;
  }
}

/**
 * The rows in `sort`'s order. Ties break on the other measure, or on visitors
 * when sorting by name; ascending is descending reversed, tiebreak included.
 */
export function sortedBreakdownRows(rows: BreakdownRow[], sort: BreakdownSort): BreakdownRow[] {
  const column = sort.column;
  const descending =
    column === "label"
      ? (a: BreakdownRow, b: BreakdownRow) =>
          breakdownRowLabel(b).localeCompare(breakdownRowLabel(a)) || b.visitors - a.visitors
      : (a: BreakdownRow, b: BreakdownRow) => {
          const tiebreak = column === "visitors" ? "revenue" : "visitors";
          return b[column] - a[column] || b[tiebreak] - a[tiebreak];
        };
  const sign = sort.direction === "desc" ? 1 : -1;
  return rows.slice().sort((a, b) => sign * descending(a, b));
}

const compactCount = new Intl.NumberFormat("en", {
  notation: "compact",
  maximumFractionDigits: 1,
});

export function formatBreakdownCount(count: number): string {
  return compactCount.format(count);
}
