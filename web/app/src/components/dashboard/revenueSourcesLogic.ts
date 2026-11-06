/// <reference types="solid-js" />

import type { PaymentsFilter } from "../../lib/paymentsApi";
import type {
  CampaignReferrerCount,
  ConversionInterval,
  GroupSummaries,
  LandingPathCount,
  ReferrerType,
  RevenueSource,
  RevenueSourceCampaign,
  RevenueSourcePeriod,
  SourceGroup,
  SourceMetrics,
} from "../../lib/revenueSourcesApi";
import type { SourceFilter } from "../../lib/segmentFilters";
import type { ColumnSort } from "./columnSort";
import {
  buildHoverHighlights,
  type HighlightMetric,
  type HoverHighlight,
} from "./metricHoverDetails";

export type { SourceGroup };

export const SOURCE_GROUP_ORDER: SourceGroup[] = [
  "search",
  "ai",
  "social",
  "campaigns",
  "referrers",
  "direct",
];

export const SOURCE_GROUP_LABELS: Record<SourceGroup, string> = {
  search: "Search",
  ai: "AI",
  social: "Social",
  campaigns: "Campaigns",
  referrers: "Referrers",
  direct: "Direct",
};

export const SOURCE_GROUP_COLORS: Record<SourceGroup, string> = {
  search: "#2563eb",
  ai: "#9333ea",
  social: "#16a34a",
  campaigns: "#f59e0b",
  referrers: "#0ea5e9",
  direct: "#6b7280",
};

/** Sentinel label used in records and the grid for the "direct" pseudo-referrer. */
const DIRECT_REFERRER_LABEL = "Direct";

/** Hover cards show at most this many referrers for a campaign. */
const MAX_CAMPAIGN_REFERRERS = 5;

/** Synthetic row label for long-tail traffic within a channel group. */
export const GROUP_OTHER_LABEL = "Other";

/** Fallback label for a campaign row whose source/medium/campaign tuple is entirely empty. */
const UNNAMED_CAMPAIGN_LABEL = "(unnamed campaign)";
export const DIRECT_SOURCE_KEY = "d";

export type RevenueSourceLinkTarget =
  | { kind: "campaign"; id: number }
  | { kind: "referrer"; id: number }
  | { kind: "direct" };

export function campaignSourceKey(id: number): string {
  return `c:${id}`;
}

export function referrerSourceKey(id: number): string {
  return `r:${id}`;
}

/**
 * One grid/treemap row per revenue source: a referrer, the direct bucket, or a
 * campaign. Campaigns are top-level rows now, not nested under a referrer.
 */
export type RevenueSourceRecord = {
  /** Stable identity for matching current-vs-previous rows. */
  key: string;
  group: SourceGroup;
  /** Display label: referrer URL, "Direct", campaign summary, or "Other". */
  label: string;
  /** Referrer URL for referrer rows; empty string otherwise. */
  referrer: string;
  isDirect: boolean;
  isCampaign: boolean;
  /** `campaigns.id` for campaign rows; null otherwise. */
  campaignId: number | null;
  /** `referrers.id` for referrer rows; null otherwise. */
  referrerId: number | null;
  /** `landings.referrer_type` for referrer rows; null otherwise. */
  referrerType: ReferrerType | null;
  visitors: number;
  /** First visits with attributed payment revenue. */
  convertingVisitors: number;
  /** Revenue (thousandths of a dollar) attributed to this source. */
  revenue: number;
  /** Top landing page paths for visits in this source, ordered by visit count. */
  landingPaths: LandingPathCount[];
  /** Top referrers for campaign rows, ordered by visit count. Empty otherwise. */
  referrers: { label: string; visitors: number }[];
  /** Long-tail bucket synthesized from API group totals minus displayed rows. */
  isGroupOther?: boolean;
  /** Server-computed conversion rate (`convertingVisitors / visitors`). */
  conversionRate: number;
  /** Server-computed revenue per visitor (thousandths). */
  revenuePerVisitor: number;
  /** Lower bound of the 95% Wilson interval, used for ranking. */
  conversionLowerBound: number;
  /** 95% Wilson confidence interval for the conversion rate. */
  conversionCi?: ConversionInterval;
  /** Normalized revenue change vs the prior 30 days (hover-card convention). */
  revenueChangeRatio: number | null;
  /** Normalized visitor change vs the prior 30 days. */
  visitorsChangeRatio: number | null;
  /** Normalized conversion-count change vs the prior 30 days. */
  conversionsChangeRatio: number | null;
  /** Revenue-per-visitor change vs the prior 30 days (rate, not normalized). */
  revenuePerVisitorChangeRatio: number | null;
  /** Conversion-rate change vs the prior 30 days (rate, not normalized). */
  conversionRateChangeRatio: number | null;
};

/** Maps a grid row to conversions API filters; null for non-drill-down rows. */
export function paymentFiltersForRecord(record: RevenueSourceRecord): PaymentsFilter | null {
  if (record.isGroupOther) {
    return null;
  }
  if (record.isCampaign && record.campaignId !== null) {
    return { campaignId: record.campaignId };
  }
  if (record.isDirect) {
    return { direct: true };
  }
  if (record.referrerId !== null) {
    return { referrerId: record.referrerId };
  }
  return null;
}

export function recordSupportsConversionsDrillDown(record: RevenueSourceRecord): boolean {
  return record.convertingVisitors > 0 && paymentFiltersForRecord(record) !== null;
}

/** Maps a drill-down grid row to the dashboard `source` URL param. */
export function revenueSourceLinkTargetForRecord(
  record: RevenueSourceRecord,
): RevenueSourceLinkTarget | null {
  if (record.isGroupOther) {
    return null;
  }
  if (record.isCampaign && record.campaignId !== null) {
    return { kind: "campaign", id: record.campaignId };
  }
  if (record.isDirect) {
    return { kind: "direct" };
  }
  if (record.referrerId !== null) {
    return { kind: "referrer", id: record.referrerId };
  }
  return null;
}

/**
 * Maps a grid row to a global source segment filter, carrying the row label as
 * the chip name. Returns null for the synthetic "Other" bucket and rows without
 * a resolvable identity.
 */
export function sourceFilterFromRecord(record: RevenueSourceRecord): SourceFilter | null {
  if (record.isGroupOther) {
    return null;
  }
  if (record.isCampaign && record.campaignId !== null) {
    return { kind: "campaign", id: record.campaignId, name: record.label };
  }
  if (record.isDirect) {
    return { kind: "direct", name: record.label };
  }
  if (record.referrerId !== null) {
    return { kind: "referrer", id: record.referrerId, name: record.label };
  }
  return null;
}

/** Maps a flattened backend {@link SourceMetrics} payload onto record fields. */
function recordMetricsFrom(
  metrics: SourceMetrics,
): Pick<
  RevenueSourceRecord,
  | "visitors"
  | "convertingVisitors"
  | "revenue"
  | "conversionRate"
  | "revenuePerVisitor"
  | "conversionLowerBound"
  | "conversionCi"
  | "revenueChangeRatio"
  | "visitorsChangeRatio"
  | "conversionsChangeRatio"
  | "revenuePerVisitorChangeRatio"
  | "conversionRateChangeRatio"
> {
  return {
    visitors: metrics.visitors,
    convertingVisitors: metrics.converting_visitors,
    revenue: metrics.revenue,
    conversionRate: metrics.conversion_rate,
    revenuePerVisitor: metrics.revenue_per_visitor,
    conversionLowerBound: metrics.conversion_lower_bound,
    conversionCi: metrics.conversion_ci,
    revenueChangeRatio: metrics.revenue_change.ratio,
    visitorsChangeRatio: metrics.visitors_change.ratio,
    conversionsChangeRatio: metrics.conversions_change.ratio,
    revenuePerVisitorChangeRatio: metrics.revenue_per_visitor_change.ratio,
    conversionRateChangeRatio: metrics.conversion_rate_change.ratio,
  };
}

export function landingPathsTooltip(landingPaths: LandingPathCount[]): string | undefined {
  if (landingPaths.length <= 2) {
    return undefined;
  }
  return landingPaths.map((entry) => entry.path).join("\n");
}

function landingPathsSearchText(landingPaths: LandingPathCount[]): string {
  return landingPaths
    .map((entry) => `${entry.path} ${entry.count}`)
    .join(" ")
    .toLowerCase();
}

function campaignTripleLabel(campaign: RevenueSourceCampaign): string {
  const parts: string[] = [];
  if (campaign.network && campaign.network !== "" && campaign.network !== "generic") {
    parts.push(`ads=${campaign.network}`);
  }
  if (campaign.source !== "") {
    parts.push(`source=${campaign.source}`);
  }
  if (campaign.medium !== "") {
    parts.push(`medium=${campaign.medium}`);
  }
  if (campaign.campaign !== "") {
    parts.push(`campaign=${campaign.campaign}`);
  }
  if (parts.length === 0) {
    return UNNAMED_CAMPAIGN_LABEL;
  }
  return parts.join(", ");
}

type CampaignLabelFields = Pick<
  RevenueSourceCampaign,
  "source" | "medium" | "campaign" | "network" | "display_name"
>;

/**
 * Single-line label for a campaign row.
 *
 * Prefers the user-set `display_name`; otherwise joins non-empty attribution
 * fields as `key=value` pairs (e.g.
 * `ads=google_ads, source=newsletter, medium=cpc, campaign=launch`).
 * Omits `ads=` when network is absent or `generic`. Falls back to
 * `UNNAMED_CAMPAIGN_LABEL` when all fields are empty.
 */
export function campaignDisplayLabel(campaign: CampaignLabelFields): string {
  if (campaign.display_name && campaign.display_name !== "") {
    return campaign.display_name;
  }
  return campaignTripleLabel(campaign as RevenueSourceCampaign);
}

function summarizeCampaign(campaign: RevenueSourceCampaign): string {
  return campaignDisplayLabel(campaign);
}

function campaignReferrerLabel(entry: CampaignReferrerCount): string {
  if (entry.referrer && entry.referrer !== "") {
    return entry.referrer;
  }
  return DIRECT_REFERRER_LABEL;
}

function campaignReferrers(source: RevenueSourceCampaign): RevenueSourceRecord["referrers"] {
  return (source.referrers ?? []).slice(0, MAX_CAMPAIGN_REFERRERS).map((entry) => ({
    label: campaignReferrerLabel(entry),
    visitors: entry.count,
  }));
}

function entryToRecord(source: RevenueSource): RevenueSourceRecord {
  if (source.kind === "campaign") {
    return {
      key: campaignSourceKey(source.id),
      group: source.group,
      label: summarizeCampaign(source),
      referrer: "",
      isDirect: false,
      isCampaign: true,
      campaignId: source.id,
      referrerId: null,
      referrerType: null,
      landingPaths: source.landing_paths ?? [],
      referrers: campaignReferrers(source),
      ...recordMetricsFrom(source),
    };
  }
  const referrer = source.referrer;
  const isDirect = referrer === undefined;
  const referrerId = isDirect ? null : (source.id ?? null);
  return {
    key: isDirect
      ? DIRECT_SOURCE_KEY
      : referrerId === null
        ? `r:${referrer}`
        : referrerSourceKey(referrerId),
    group: source.group,
    label: isDirect ? DIRECT_REFERRER_LABEL : referrer,
    referrer: isDirect ? "" : referrer,
    isDirect,
    isCampaign: false,
    campaignId: null,
    referrerId,
    referrerType: source.referrer_type ?? null,
    landingPaths: source.landing_paths ?? [],
    referrers: [],
    ...recordMetricsFrom(source),
  };
}

/**
 * Build one row per revenue source from a single-period payload, sorted by
 * revenue descending (visitors break ties).
 */
export function rowsFromResponse(period: RevenueSourcePeriod): RevenueSourceRecord[] {
  const records = period.entries.map(entryToRecord);
  records.sort((a, b) => {
    if (b.revenue !== a.revenue) {
      return b.revenue - a.revenue;
    }
    return b.visitors - a.visitors;
  });
  return records;
}

type GroupTotals = Record<SourceGroup, { visitors: number; revenue: number }>;

/** Per-channel totals from a revenue-sources period (full period, not top-N). */
export function groupTotalsFromResponse(period: RevenueSourcePeriod): GroupTotals {
  return {
    search: {
      visitors: period.search_visitors,
      revenue: period.search_revenue,
    },
    ai: {
      visitors: period.ai_visitors,
      revenue: period.ai_revenue,
    },
    social: {
      visitors: period.social_visitors,
      revenue: period.social_revenue,
    },
    campaigns: {
      visitors: period.campaigns_visitors,
      revenue: period.campaigns_revenue,
    },
    referrers: {
      visitors: period.referrers_group_visitors,
      revenue: period.referrers_group_revenue,
    },
    direct: {
      visitors: period.direct_visitors,
      revenue: period.direct_revenue,
    },
  };
}

export function isSourceGroupEnabled(
  group: SourceGroup,
  disabledGroups: ReadonlySet<SourceGroup>,
): boolean {
  return !disabledGroups.has(group);
}

export function toggleDisabledSourceGroup(
  group: SourceGroup,
  disabledGroups: ReadonlySet<SourceGroup>,
): Set<SourceGroup> {
  const next = new Set(disabledGroups);
  if (next.has(group)) {
    next.delete(group);
  } else {
    next.add(group);
  }
  return next;
}

/** True when `group` is the only enabled source group. */
export function isSourceGroupSoloed(
  group: SourceGroup,
  disabledGroups: ReadonlySet<SourceGroup>,
): boolean {
  return disabledGroups.size === SOURCE_GROUP_ORDER.length - 1 && !disabledGroups.has(group);
}

/** Solo `group`, or select all groups if it is already the only one enabled. */
export function soloSourceGroup(
  group: SourceGroup,
  disabledGroups: ReadonlySet<SourceGroup> = new Set(),
): Set<SourceGroup> {
  if (isSourceGroupSoloed(group, disabledGroups)) {
    return new Set();
  }
  return new Set(SOURCE_GROUP_ORDER.filter((candidate) => candidate !== group));
}

export function filterRecordsByGroups(
  records: RevenueSourceRecord[],
  disabledGroups: ReadonlySet<SourceGroup>,
): RevenueSourceRecord[] {
  if (disabledGroups.size === 0) {
    return records;
  }
  return records.filter((record) => isSourceGroupEnabled(record.group, disabledGroups));
}

function sumRecordsInGroup(
  records: RevenueSourceRecord[],
  group: SourceGroup,
): { visitors: number; revenue: number } {
  let visitors = 0;
  let revenue = 0;
  for (const record of records) {
    if (record.group === group && !record.isGroupOther) {
      visitors += record.visitors;
      revenue += record.revenue;
    }
  }
  return { visitors, revenue };
}

function groupOtherKey(group: SourceGroup): string {
  return `\0other:${group}`;
}

function buildGroupOtherRecord(
  group: SourceGroup,
  visitors: number,
  revenue: number,
): RevenueSourceRecord {
  return {
    key: groupOtherKey(group),
    group,
    label: GROUP_OTHER_LABEL,
    referrer: GROUP_OTHER_LABEL,
    isDirect: false,
    isCampaign: false,
    campaignId: null,
    referrerId: null,
    referrerType: null,
    visitors: Math.max(0, visitors),
    convertingVisitors: 0,
    revenue: Math.max(0, revenue),
    landingPaths: [],
    referrers: [],
    isGroupOther: true,
    conversionRate: 0,
    revenuePerVisitor: revenuePerVisitor(Math.max(0, revenue), Math.max(0, visitors)),
    conversionLowerBound: 0,
    conversionCi: undefined,
    revenueChangeRatio: null,
    visitorsChangeRatio: null,
    conversionsChangeRatio: null,
    revenuePerVisitorChangeRatio: null,
    conversionRateChangeRatio: null,
  };
}

/** Synthetic "Other" rows for groups whose API totals exceed displayed sums. */
export function buildGroupOtherRecords(
  displayed: RevenueSourceRecord[],
  groupTotals: GroupTotals,
  disabledGroups: ReadonlySet<SourceGroup> = new Set(),
): RevenueSourceRecord[] {
  const out: RevenueSourceRecord[] = [];
  for (const group of SOURCE_GROUP_ORDER) {
    if (group === "direct") {
      continue;
    }
    if (!isSourceGroupEnabled(group, disabledGroups)) {
      continue;
    }
    const totals = groupTotals[group];
    const shown = sumRecordsInGroup(displayed, group);
    const otherVisitors = totals.visitors - shown.visitors;
    const otherRevenue = totals.revenue - shown.revenue;
    if (otherVisitors > 0 || otherRevenue > 0) {
      out.push(buildGroupOtherRecord(group, otherVisitors, otherRevenue));
    }
  }
  return out;
}

function appendGroupOtherRecords(
  records: RevenueSourceRecord[],
  groupTotals: GroupTotals,
  disabledGroups: ReadonlySet<SourceGroup> = new Set(),
): RevenueSourceRecord[] {
  return [...records, ...buildGroupOtherRecords(records, groupTotals, disabledGroups)];
}

export type TreemapLeaf = {
  group: SourceGroup;
  /** Short label shown on the tile (hostname for referrers). */
  label: string;
  /** Full source label shown in the chart tooltip. */
  tooltip: string;
  /** Stable unique key for the ApexCharts datum (`x`). */
  datumKey: string;
  revenue: number;
  visitors: number;
};

/** Host portion of a normalized referrer (`host/path` → `host`). */
function referrerHostname(referrer: string): string {
  const slash = referrer.indexOf("/");
  return slash === -1 ? referrer : referrer.slice(0, slash);
}

/** Keep the highest-revenue sources (visitors break ties). */
export function takeTopSourcesByRevenue(
  records: RevenueSourceRecord[],
  limit: number,
): RevenueSourceRecord[] {
  return [...records]
    .sort((a, b) => {
      if (b.revenue !== a.revenue) {
        return b.revenue - a.revenue;
      }
      return b.visitors - a.visitors;
    })
    .slice(0, limit);
}

function recordToTreemapLeaf(record: RevenueSourceRecord): TreemapLeaf {
  return {
    group: record.group,
    label: treemapLabelForRecord(record),
    tooltip: treemapTooltipForRecord(record),
    datumKey: record.key,
    revenue: record.revenue,
    visitors: record.visitors,
  };
}

/**
 * Build treemap leaves grouped by `SourceGroup`, keeping at most `topSources`
 * leaves overall (by revenue), then optional per-group "Other" tiles.
 * Sources with no revenue are omitted so the chart can hide when empty.
 */
export function buildTreemapLeaves(
  records: RevenueSourceRecord[],
  groupTotals: GroupTotals,
  topSources: number,
  disabledGroups: ReadonlySet<SourceGroup> = new Set(),
): Map<SourceGroup, TreemapLeaf[]> {
  const top = takeTopSourcesByRevenue(records, topSources);
  const withOther = appendGroupOtherRecords(top, groupTotals, disabledGroups);
  const grouped = new Map<SourceGroup, TreemapLeaf[]>();
  for (const group of SOURCE_GROUP_ORDER) {
    grouped.set(group, []);
  }
  for (const record of withOther) {
    // Zero-revenue tiles have no area; omitting them lets the chart collapse
    // when a period has visitors but no attributed revenue.
    if (record.revenue <= 0) {
      continue;
    }
    const leaves = grouped.get(record.group);
    if (leaves) {
      leaves.push(recordToTreemapLeaf(record));
    }
  }
  for (const [group, leaves] of grouped) {
    leaves.sort((a, b) => b.revenue - a.revenue);
    grouped.set(group, leaves);
  }
  return grouped;
}

function treemapLabelForRecord(record: RevenueSourceRecord): string {
  if (record.isGroupOther) {
    return GROUP_OTHER_LABEL;
  }
  if (record.isDirect) {
    return DIRECT_REFERRER_LABEL;
  }
  if (record.isCampaign) {
    return record.label;
  }
  return referrerHostname(record.referrer);
}

function treemapTooltipForRecord(record: RevenueSourceRecord): string {
  if (record.isGroupOther) {
    return `${SOURCE_GROUP_LABELS[record.group]} · ${GROUP_OTHER_LABEL}`;
  }
  if (record.isDirect) {
    return DIRECT_REFERRER_LABEL;
  }
  if (record.isCampaign) {
    return record.label;
  }
  return record.referrer;
}

/** Revenue per visitor (thousandths of a dollar per visit). */
function revenuePerVisitor(revenue: number, visitors: number): number {
  return visitors > 0 ? revenue / visitors : 0;
}

/**
 * Integer count of conversions for display. Uses the API count when present;
 * otherwise estimates from visitors × unrounded conversion rate.
 */
export function conversionCount(
  visitors: number,
  convertingVisitors: number | undefined,
  unroundedConversionRate?: number,
): number {
  if (convertingVisitors !== undefined) {
    return convertingVisitors;
  }
  if (visitors > 0 && unroundedConversionRate !== undefined) {
    return Math.round(visitors * unroundedConversionRate);
  }
  return 0;
}

/**
 * Unsigned percent magnitude, one decimal below 10% and none above, with a
 * trailing ".0" dropped (`5%`, `1.2%`, `12%`).
 */
function formatChangePercentMagnitude(ratio: number): string {
  const absPct = Math.abs(ratio * 100);
  // Number() drops the ".0" that toFixed leaves, including when 9.96 rounds up to "10.0".
  return `${Number(absPct < 10 ? absPct.toFixed(1) : absPct.toFixed(0))}%`;
}

/** A compared change too small to show an arrow for. */
export const FLAT_CHANGE_TEXT = "0%";

/**
 * Period-over-period change as ▲/▼ plus unsigned percent (e.g. `▲12%`).
 * Returns empty string when no comparison is available or the change rounds to
 * 0%; callers that must still show a flat change use FLAT_CHANGE_TEXT.
 */
export function formatTileChangeRatio(ratio: number | null): string {
  if (ratio === null || !Number.isFinite(ratio)) {
    return "";
  }
  const magnitude = formatChangePercentMagnitude(ratio);
  if (magnitude === "0%") {
    return "";
  }
  return `${ratio > 0 ? "▲" : "▼"}${magnitude}`;
}

/** Sortable grid columns, keyed to the displayed table headers. */
export type SortColumn =
  | "group"
  | "source"
  | "landingPaths"
  | "visitors"
  | "revenue"
  | "conversion";

export type SortState = ColumnSort<SortColumn>;

/** The grid initially mirrors the API ordering: revenue, highest first. */
export const DEFAULT_SORT: SortState = {
  column: "revenue",
  direction: "desc",
};

function sortValue(record: RevenueSourceRecord, column: SortColumn): number | string {
  switch (column) {
    case "group":
      return SOURCE_GROUP_ORDER.indexOf(record.group);
    case "source":
      return record.label.toLowerCase();
    case "landingPaths":
      return landingPathsSearchText(record.landingPaths);
    case "visitors":
      return record.visitors;
    case "revenue":
      return record.revenue;
    case "conversion":
      return record.conversionLowerBound;
  }
}

function compareSortValues(a: number | string, b: number | string): number {
  if (typeof a === "string" && typeof b === "string") {
    return a.localeCompare(b);
  }
  const an = a as number;
  const bn = b as number;
  if (an < bn) {
    return -1;
  }
  if (an > bn) {
    return 1;
  }
  return 0;
}

/**
 * Return a new array of records ordered by the requested column/direction.
 *
 * Ties fall back to visitors desc (revenue desc when sorting on visitors),
 * then key, so the sort is stable regardless of the input order.
 */
export function sortRecords(
  records: RevenueSourceRecord[],
  sort: SortState,
): RevenueSourceRecord[] {
  const direction = sort.direction === "asc" ? 1 : -1;
  return [...records].sort((a, b) => {
    const comparison = compareSortValues(sortValue(a, sort.column), sortValue(b, sort.column));
    if (comparison !== 0) {
      return comparison * direction;
    }
    if (sort.column === "visitors") {
      if (a.revenue !== b.revenue) {
        return b.revenue - a.revenue;
      }
    } else if (a.visitors !== b.visitors) {
      return b.visitors - a.visitors;
    }
    return a.key < b.key ? -1 : a.key > b.key ? 1 : 0;
  });
}

/** Channel groups that get per-group "best source" highlights in the grid. */
const BEST_SOURCE_GROUPS: SourceGroup[] = ["campaigns", "social", "referrers"];

/** Groups that get a conversion-leak warning (below-average rate × visitors). */
const CONVERSION_LEAK_GROUPS: SourceGroup[] = ["campaigns", "social"];

export type BestSourceHighlights = {
  revenue: Set<string>;
  revenuePerVisitor: Set<string>;
  conversion: Set<string>;
  conversionLeak: Set<string>;
};

type BestSourceHighlightMetric = "revenue" | "revenuePerVisitor" | "conversion";

function bestSourceHighlightValue(
  record: RevenueSourceRecord,
  metric: BestSourceHighlightMetric,
): number {
  switch (metric) {
    case "revenue":
      return record.revenue;
    case "revenuePerVisitor":
      return record.revenuePerVisitor;
    case "conversion":
      return record.conversionLowerBound;
  }
}

function findPerGroupBestSourceKeys(
  records: RevenueSourceRecord[],
  metric: BestSourceHighlightMetric,
): Set<string> {
  const keys = new Set<string>();
  for (const group of BEST_SOURCE_GROUPS) {
    const candidates = records.filter((record) => {
      if (record.group !== group || record.isGroupOther) {
        return false;
      }
      // A 0% conversion rate is not a best-conversion highlight.
      if (metric === "conversion" && record.conversionRate <= 0) {
        return false;
      }
      return true;
    });
    if (candidates.length === 0) {
      continue;
    }
    let best = Number.NEGATIVE_INFINITY;
    for (const record of candidates) {
      const value = bestSourceHighlightValue(record, metric);
      if (value > best) {
        best = value;
      }
    }
    if (!Number.isFinite(best) || best <= 0) {
      continue;
    }
    for (const record of candidates) {
      if (bestSourceHighlightValue(record, metric) === best) {
        keys.add(record.key);
      }
    }
  }
  return keys;
}

/** Extra converters this source would have at the group's conversion rate. */
export function conversionLeak(
  visitors: number,
  conversionRate: number,
  groupConversionRate: number,
): number {
  if (visitors <= 0) {
    return 0;
  }
  const gap = groupConversionRate - conversionRate;
  return gap > 0 ? visitors * gap : 0;
}

function findConversionLeakKeys(
  records: RevenueSourceRecord[],
  groups: GroupSummaries | undefined,
): Set<string> {
  const keys = new Set<string>();
  if (!groups) {
    return keys;
  }
  for (const group of CONVERSION_LEAK_GROUPS) {
    const groupRate = groups[group]?.conversion_rate;
    if (groupRate == null || groupRate <= 0) {
      continue;
    }
    const candidates = records.filter(
      (record) => record.group === group && !record.isGroupOther && record.visitors > 0,
    );
    let best = Number.NEGATIVE_INFINITY;
    for (const record of candidates) {
      const leak = conversionLeak(record.visitors, record.conversionRate, groupRate);
      if (leak > best) {
        best = leak;
      }
    }
    if (!Number.isFinite(best) || best <= 0) {
      continue;
    }
    for (const record of candidates) {
      // A 0% row is not the leak warning, and a lesser leak is not promoted.
      if (record.conversionRate <= 0) {
        continue;
      }
      if (conversionLeak(record.visitors, record.conversionRate, groupRate) === best) {
        keys.add(record.key);
      }
    }
  }
  return keys;
}

/**
 * Keys of rows with the highest revenue, revenue / visitor, or conversion rate
 * (Wilson lower bound) within each campaigns, social, and referrers group
 * (ties all receive a highlight). A 0% conversion rate is not starred or
 * warned. Conversion-leak warnings use the group-tile conversion rate and
 * apply only to campaigns and social.
 */
export function findBestSourceHighlights(
  records: RevenueSourceRecord[],
  groups?: GroupSummaries,
): BestSourceHighlights {
  return {
    revenue: findPerGroupBestSourceKeys(records, "revenue"),
    revenuePerVisitor: findPerGroupBestSourceKeys(records, "revenuePerVisitor"),
    conversion: findPerGroupBestSourceKeys(records, "conversion"),
    conversionLeak: findConversionLeakKeys(records, groups),
  };
}

export function sourceRowHasBestHighlight(key: string, highlights: BestSourceHighlights): boolean {
  return (
    highlights.revenue.has(key) ||
    highlights.revenuePerVisitor.has(key) ||
    highlights.conversion.has(key) ||
    highlights.conversionLeak.has(key)
  );
}

/** Hover card highlight lines for a row flagged as a per-group best or worst source. */
export function bestSourceRowHighlights(
  record: RevenueSourceRecord,
  highlights: BestSourceHighlights,
): HoverHighlight[] {
  const metrics: HighlightMetric[] = [];
  if (highlights.revenue.has(record.key)) {
    metrics.push("revenue");
  }
  if (highlights.revenuePerVisitor.has(record.key)) {
    metrics.push("revenuePerVisitor");
  }
  if (highlights.conversion.has(record.key)) {
    metrics.push("conversion");
  }
  if (highlights.conversionLeak.has(record.key)) {
    metrics.push("conversionLeak");
  }
  return buildHoverHighlights(metrics, record.group);
}
