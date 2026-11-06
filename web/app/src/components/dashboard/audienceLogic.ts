/// <reference types="solid-js" />

import type {
  AudienceBrowsers,
  AudienceCities,
  AudienceCountries,
  AudienceDevices,
  AudienceOs,
  AudienceScreenSizes,
  AudienceViewports,
  CityRow,
  ConversionInterval,
  CountryRow,
  RowMetrics,
} from "../../lib/audienceApi";
import { formatPercentRatio, formatRevenueAmount } from "../../lib/dashboardApi";
import type { ColumnSort } from "./columnSort";
import { formatWilsonScoreInterval } from "./conversionConfidence";
import {
  dashboardIconBrowserBaidu,
  dashboardIconBrowserBrave,
  dashboardIconBrowserChrome,
  dashboardIconBrowserDuckduckgo,
  dashboardIconBrowserEdge,
  dashboardIconBrowserFirefox,
  dashboardIconBrowserIe,
  dashboardIconBrowserKai,
  dashboardIconBrowserOpera,
  dashboardIconBrowserQq,
  dashboardIconBrowserSafari,
  dashboardIconBrowserSamsung,
  dashboardIconBrowserUc,
  dashboardIconBrowserYandex,
  dashboardIconDeviceDesktop,
  dashboardIconDeviceDesktopTouch,
  dashboardIconDeviceMobile,
  dashboardIconDeviceTablet,
  dashboardIconOsAndroid,
  dashboardIconOsIos,
  dashboardIconOsKaios,
  dashboardIconOsLinux,
  dashboardIconOsMacos,
  dashboardIconOsWindows,
  dashboardIconUnknown,
} from "./icons.css";
import type { MetricHoverContent } from "./metricHoverDetails";
import {
  buildHoverHighlights,
  type HighlightMetric,
  type HoverHighlight,
} from "./metricHoverDetails";

/** Number of choropleth buckets the map is shaded into. */
const REVENUE_BUCKET_COUNT = 5;

/**
 * Sequential blue scale from the lowest revenue-share bucket (light) to the
 * highest (dark). Index 0 is bucket 1; index 4 is bucket {@link REVENUE_BUCKET_COUNT}.
 */
export const REVENUE_BUCKET_COLORS: readonly string[] = [
  "#dbeafe",
  "#93c5fd",
  "#60a5fa",
  "#3b82f6",
  "#1d4ed8",
];

/** Fill applied to countries with no visitors in the period. */
export const NO_DATA_REGION_FILL = "#e5e7eb";

/**
 * Display-ready, comparison-aware metrics for a single hover-card subject. All
 * values are computed server-side; the frontend only renders them.
 */
type AudienceHoverMetrics = {
  visitors: number;
  convertingVisitors: number;
  /** Revenue in thousandths of a dollar. */
  revenue: number;
  /** Revenue per visitor (thousandths of a dollar). */
  revenuePerVisitor: number;
  revenueChangeRatio: number | null;
  visitorsChangeRatio: number | null;
  conversionsChangeRatio: number | null;
};

/** Country hover metrics include conversion rate (country-only on the audience page). */
type CountryHoverMetrics = AudienceHoverMetrics & {
  /** Converting visitors ÷ tracked visitors (0..1). */
  conversionRate: number;
  /** 95% Wilson interval for the conversion rate, when defined. */
  conversionCi?: ConversionInterval;
};

/** Derived metrics for a single country in the period. */
export type CountryMetrics = CountryHoverMetrics & {
  /** ISO 3166-1 alpha-2 code (uppercase). */
  code: string;
  /** Human-readable country name. */
  name: string;
  /** Share of the period's total revenue (0..1). */
  revenueShare: number;
};

/** Derived metrics for a single city in the period. */
export type CityMetrics = AudienceHoverMetrics & {
  /** Stable row key (`country:city`). */
  key: string;
  /** City name from GeoIP. */
  city: string;
  /** ISO 3166-1 alpha-2 code when resolved. */
  countryCode?: string;
  /** Human-readable country name when a code is present. */
  countryName?: string;
};

/** Derived metrics for a single tech-dimension row in the period. */
export type TechMetrics = AudienceHoverMetrics & {
  key: string;
  name: string;
};

export type AudienceBreakdownColumn =
  | "name"
  | "visitors"
  | "revenue"
  | "revenuePerVisitor"
  | "conversion";

export type AudienceBreakdownSortState = ColumnSort<AudienceBreakdownColumn>;

export const DEFAULT_AUDIENCE_BREAKDOWN_SORT: AudienceBreakdownSortState = {
  column: "revenue",
  direction: "desc",
};

export type AudienceBreakdownRow = {
  key: string;
  name: string;
  /** When set, show this country's flag beside the name (city rows). */
  flagCountryCode?: string;
  visitors: number;
  convertingVisitors: number;
  /** Revenue in thousandths of a dollar. */
  revenue: number;
  /** Revenue per visitor (thousandths of a dollar). */
  revenuePerVisitor: number;
  /** Converting visitors ÷ tracked visitors (0..1). Present on country rows only. */
  conversionRate?: number;
  /** Lower bound of the 95% Wilson interval. Present on country rows only. */
  conversionLowerBound?: number;
  /** 95% Wilson interval for the conversion rate. Present on country rows only. */
  conversionCi?: ConversionInterval;
};

let regionNameFormatter: Intl.DisplayNames | undefined | null;

export function isIsoCountryCode(code: string): boolean {
  return /^[A-Z]{2}$/.test(code);
}

/** Resolve a country display name from an alpha-2 code, falling back to the code. */
export function countryDisplayName(code: string): string {
  if (regionNameFormatter === undefined) {
    try {
      regionNameFormatter = new Intl.DisplayNames(["en"], { type: "region" });
    } catch {
      regionNameFormatter = null;
    }
  }
  if (regionNameFormatter) {
    try {
      return regionNameFormatter.of(code) ?? code;
    } catch {
      return code;
    }
  }
  return code;
}

/** Build the shared hover-metric view from a row's server-computed metrics. */
function hoverMetricsFrom(metrics: RowMetrics): AudienceHoverMetrics {
  return {
    visitors: metrics.visitors,
    convertingVisitors: metrics.converting_visitors,
    revenue: metrics.revenue,
    revenuePerVisitor: metrics.revenue_per_visitor,
    revenueChangeRatio: metrics.revenue_change.ratio,
    visitorsChangeRatio: metrics.visitors_change.ratio,
    conversionsChangeRatio: metrics.conversions_change.ratio,
  };
}

function countryHoverMetricsFrom(row: CountryRow): CountryHoverMetrics {
  return {
    ...hoverMetricsFrom(row),
    conversionRate: row.conversion_rate,
    conversionCi: row.conversion_ci,
  };
}

/** Build the shared breakdown-row view from a row's server-computed metrics. */
function breakdownRowFrom(
  key: string,
  name: string,
  metrics: RowMetrics,
  flagCountryCode?: string,
): AudienceBreakdownRow {
  return {
    key,
    name,
    flagCountryCode,
    visitors: metrics.visitors,
    convertingVisitors: metrics.converting_visitors,
    revenue: metrics.revenue,
    revenuePerVisitor: metrics.revenue_per_visitor,
  };
}

function countryBreakdownRowFrom(key: string, name: string, row: CountryRow): AudienceBreakdownRow {
  return {
    ...breakdownRowFrom(key, name, row),
    conversionRate: row.conversion_rate,
    conversionLowerBound: row.conversion_lower_bound,
    conversionCi: row.conversion_ci,
  };
}

/** Sum of revenue across every current row, including landings with no country. */
function totalPeriodRevenue(countries: AudienceCountries): number {
  return countries.current.reduce((sum, row) => sum + row.revenue, 0);
}

/**
 * Build metrics for every country with a code in the current period, keyed by
 * alpha-2 code. Landings with no country are excluded from the map (they cannot
 * be placed) but still count toward total revenue, so the revenue-share
 * denominator stays honest.
 */
export function buildCountryMetrics(countries: AudienceCountries): Map<string, CountryMetrics> {
  const totalRevenue = totalPeriodRevenue(countries);
  const metrics = new Map<string, CountryMetrics>();
  for (const row of countries.current) {
    if (!row.country) {
      continue;
    }
    metrics.set(row.country, {
      ...countryHoverMetricsFrom(row),
      code: row.country,
      name: row.country_name || countryDisplayName(row.country),
      revenueShare: totalRevenue > 0 ? row.revenue / totalRevenue : 0,
    });
  }
  return metrics;
}

function countryRowName(row: CountryRow): string {
  if (row.country_name) {
    return row.country_name;
  }
  return row.country ? countryDisplayName(row.country) : "Unknown";
}

export function buildBrowserBreakdownRows(browsers: AudienceBrowsers): AudienceBreakdownRow[] {
  return browsers.current.map((row) =>
    breakdownRowFrom(row.browser_family, row.browser_family_name || row.browser_family, row),
  );
}

const browserFamilyIconClasses: Record<string, string> = {
  baidu: dashboardIconBrowserBaidu,
  brave: dashboardIconBrowserBrave,
  chrome: dashboardIconBrowserChrome,
  duckduckgo: dashboardIconBrowserDuckduckgo,
  edge: dashboardIconBrowserEdge,
  firefox: dashboardIconBrowserFirefox,
  ie: dashboardIconBrowserIe,
  kai: dashboardIconBrowserKai,
  opera: dashboardIconBrowserOpera,
  qq: dashboardIconBrowserQq,
  safari: dashboardIconBrowserSafari,
  samsung: dashboardIconBrowserSamsung,
  uc: dashboardIconBrowserUc,
  yandex: dashboardIconBrowserYandex,
};

const osIconClasses: Record<string, string> = {
  android: dashboardIconOsAndroid,
  ios: dashboardIconOsIos,
  kaios: dashboardIconOsKaios,
  linux: dashboardIconOsLinux,
  macos: dashboardIconOsMacos,
  windows: dashboardIconOsWindows,
};

const deviceIconClasses: Record<string, string> = {
  desktop: dashboardIconDeviceDesktop,
  desktop_touch: dashboardIconDeviceDesktopTouch,
  mobile: dashboardIconDeviceMobile,
  tablet: dashboardIconDeviceTablet,
};

/** CSS class for a browser-family icon. */
export function browserFamilyIconClass(browserFamily: string): string {
  if (browserFamily === "unknown") {
    return dashboardIconUnknown;
  }
  return browserFamilyIconClasses[browserFamily] ?? dashboardIconUnknown;
}

/** CSS class for an OS icon. */
export function osIconClass(os: string): string {
  if (os === "unknown") {
    return dashboardIconUnknown;
  }
  if (os === "chromeos") {
    return dashboardIconBrowserChrome;
  }
  return osIconClasses[os] ?? dashboardIconUnknown;
}

/** CSS class for a device icon. */
export function deviceIconClass(device: string): string {
  return deviceIconClasses[device] ?? dashboardIconUnknown;
}

export function buildOsBreakdownRows(os: AudienceOs): AudienceBreakdownRow[] {
  return os.current.map((row) => breakdownRowFrom(row.os, row.os_name || row.os, row));
}

/** Human-readable label for a width bucket slug from the API. */
export function widthBucketLabel(bucket: string): string {
  switch (bucket) {
    case "small_mobile":
      return "Mobile < 360";
    case "mobile":
      return "Mobile [360, 768)";
    case "tablet":
      return "Tablet [768, 1024)";
    case "desktop":
      return "Desktop [1024, 1440)";
    case "large_desktop":
      return "Desktop [1440, 1920)";
    case "x_large_desktop":
      return "Desktop ≥ 1920";
    default:
      return bucket;
  }
}

export function buildScreenSizeBreakdownRows(
  screenSizes: AudienceScreenSizes,
): AudienceBreakdownRow[] {
  return screenSizes.current.map((row) =>
    breakdownRowFrom(row.screen_size, widthBucketLabel(row.screen_size), row),
  );
}

export function buildViewportBreakdownRows(viewports: AudienceViewports): AudienceBreakdownRow[] {
  return viewports.current.map((row) =>
    breakdownRowFrom(row.viewport, widthBucketLabel(row.viewport), row),
  );
}

export function buildDeviceBreakdownRows(devices: AudienceDevices): AudienceBreakdownRow[] {
  return devices.current.map((row) =>
    breakdownRowFrom(row.device, row.device_name || row.device, row),
  );
}

function cityRowKey(country: string | undefined, city: string): string {
  return `${country ?? "unknown"}:${city}`;
}

/** Build metrics for every city in the current period. */
export function buildCityMetrics(cities: AudienceCities): Map<string, CityMetrics> {
  const metrics = new Map<string, CityMetrics>();
  for (const row of cities.current) {
    const key = cityRowKey(row.country, row.city);
    metrics.set(key, {
      ...hoverMetricsFrom(row),
      key,
      city: row.city,
      countryCode: row.country,
      countryName: row.country ? countryDisplayName(row.country) : undefined,
    });
  }
  return metrics;
}

function cityRowDisplayName(row: CityRow): string {
  if (row.country) {
    return `${row.city}, ${countryDisplayName(row.country)}`;
  }
  return row.city;
}

export function buildCityBreakdownRows(cities: AudienceCities): AudienceBreakdownRow[] {
  return cities.current.map((row) =>
    breakdownRowFrom(cityRowKey(row.country, row.city), cityRowDisplayName(row), row, row.country),
  );
}

export function buildCountryBreakdownRows(countries: AudienceCountries): AudienceBreakdownRow[] {
  return countries.current.map((row) =>
    countryBreakdownRowFrom(row.country ?? "unknown", countryRowName(row), row),
  );
}

function audienceBreakdownSortValue(
  row: AudienceBreakdownRow,
  column: AudienceBreakdownColumn,
): number | string {
  switch (column) {
    case "name":
      return row.name.toLowerCase();
    case "visitors":
      return row.visitors;
    case "revenue":
      return row.revenue;
    case "revenuePerVisitor":
      return row.revenuePerVisitor;
    case "conversion":
      return row.conversionLowerBound ?? 0;
  }
}

function compareAudienceBreakdownValues(a: number | string, b: number | string): number {
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

export type AudienceBreakdownTreemapLeaf = {
  label: string;
  tooltip: string;
  datumKey: string;
  revenue: number;
  visitors: number;
  isOther: boolean;
};

export const AUDIENCE_BREAKDOWN_TREEMAP_TOP = 50;
const AUDIENCE_BREAKDOWN_TREEMAP_OTHER_KEY = "__audience_breakdown_other__";

function compareAudienceBreakdownRowsByRevenue(
  left: AudienceBreakdownRow,
  right: AudienceBreakdownRow,
): number {
  if (right.revenue !== left.revenue) {
    return right.revenue - left.revenue;
  }
  if (right.visitors !== left.visitors) {
    return right.visitors - left.visitors;
  }
  return left.name.localeCompare(right.name);
}

/** Ranked treemap leaves for a breakdown dimension, with long tails rolled into Other. */
export function buildAudienceBreakdownTreemapLeaves(
  rows: AudienceBreakdownRow[],
  otherTooltipNoun: string,
): AudienceBreakdownTreemapLeaf[] {
  const ranked = rows.filter((row) => row.revenue > 0).sort(compareAudienceBreakdownRowsByRevenue);
  const topRows = ranked.slice(0, AUDIENCE_BREAKDOWN_TREEMAP_TOP);
  const otherRows = ranked.slice(AUDIENCE_BREAKDOWN_TREEMAP_TOP);
  // Zero-revenue tiles have no area; omitting them lets the chart collapse
  // when a dimension has visitors but no revenue.
  const leaves: AudienceBreakdownTreemapLeaf[] = topRows.map((row) => ({
    label: row.name,
    tooltip: row.name,
    datumKey: row.key,
    revenue: row.revenue,
    visitors: row.visitors,
    isOther: false,
  }));
  const otherRevenue = otherRows.reduce((total, row) => total + row.revenue, 0);
  if (otherRevenue > 0) {
    leaves.push({
      label: "Other",
      tooltip: `Revenue from ${otherRows.length.toLocaleString()} ${otherTooltipNoun} outside the top ${AUDIENCE_BREAKDOWN_TREEMAP_TOP}`,
      datumKey: AUDIENCE_BREAKDOWN_TREEMAP_OTHER_KEY,
      revenue: otherRevenue,
      visitors: otherRows.reduce((total, row) => total + row.visitors, 0),
      isOther: true,
    });
  }
  return leaves;
}

export function sortAudienceBreakdownRows(
  rows: AudienceBreakdownRow[],
  sort: AudienceBreakdownSortState,
): AudienceBreakdownRow[] {
  const direction = sort.direction === "asc" ? 1 : -1;
  return [...rows].sort((a, b) => {
    const comparison = compareAudienceBreakdownValues(
      audienceBreakdownSortValue(a, sort.column),
      audienceBreakdownSortValue(b, sort.column),
    );
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
    return a.name.localeCompare(b.name);
  });
}

export type AudienceBreakdownHighlightMetric = "revenue" | "conversion";

export type AudienceBreakdownHighlights = Partial<Record<AudienceBreakdownHighlightMetric, string>>;

function audienceHighlightValue(
  row: AudienceBreakdownRow,
  metric: AudienceBreakdownHighlightMetric,
): number {
  switch (metric) {
    case "revenue":
      return row.revenue;
    case "conversion":
      return row.conversionLowerBound ?? 0;
  }
}

function compareAudienceHighlightRows(
  a: AudienceBreakdownRow,
  b: AudienceBreakdownRow,
  metric: AudienceBreakdownHighlightMetric,
): number {
  const av = audienceHighlightValue(a, metric);
  const bv = audienceHighlightValue(b, metric);
  if (av !== bv) {
    return bv - av;
  }
  if (a.visitors !== b.visitors) {
    return b.visitors - a.visitors;
  }
  if (a.revenue !== b.revenue) {
    return b.revenue - a.revenue;
  }
  return a.name.localeCompare(b.name);
}

export function findAudienceBreakdownHighlights(
  rows: AudienceBreakdownRow[],
  metrics: readonly AudienceBreakdownHighlightMetric[] = ["revenue", "conversion"],
): AudienceBreakdownHighlights {
  const highlights: AudienceBreakdownHighlights = {};
  for (const metric of metrics) {
    const best = [...rows].sort((a, b) => compareAudienceHighlightRows(a, b, metric))[0];
    if (best) {
      highlights[metric] = best.key;
    }
  }
  return highlights;
}

function audienceBreakdownRowHighlights(
  rowKey: string,
  highlights: AudienceBreakdownHighlights,
): HoverHighlight[] {
  const metrics: HighlightMetric[] = [];
  if (highlights.revenue === rowKey) {
    metrics.push("revenue");
  }
  if (highlights.conversion === rowKey) {
    metrics.push("conversion");
  }
  return buildHoverHighlights(metrics);
}

export function withAudienceBreakdownHighlightDescription(
  content: MetricHoverContent,
  rowKey: string,
  highlights: AudienceBreakdownHighlights | undefined,
): MetricHoverContent {
  if (!highlights) {
    return content;
  }
  const highlightDescriptions = audienceBreakdownRowHighlights(rowKey, highlights);
  if (highlightDescriptions.length === 0) {
    return content;
  }
  return { ...content, highlightDescriptions };
}

/**
 * Assign each country a 1-based bucket from most to least revenue. Countries
 * are ranked by revenue (descending) and split into {@link REVENUE_BUCKET_COUNT}
 * roughly equal-count groups, so the top group is bucket
 * {@link REVENUE_BUCKET_COUNT} (darkest) and the bottom is bucket 1 (lightest).
 */
export function bucketByRevenue(metrics: Map<string, CountryMetrics>): Map<string, number> {
  const sorted = [...metrics.values()].sort((a, b) => {
    if (b.revenue !== a.revenue) {
      return b.revenue - a.revenue;
    }
    return b.visitors - a.visitors;
  });
  const buckets = new Map<string, number>();
  const count = sorted.length;
  if (count === 0) {
    return buckets;
  }
  for (let index = 0; index < count; index += 1) {
    const fromTop = Math.floor((index * REVENUE_BUCKET_COUNT) / count);
    const bucket = Math.min(REVENUE_BUCKET_COUNT, Math.max(1, REVENUE_BUCKET_COUNT - fromTop));
    buckets.set(sorted[index].code, bucket);
  }
  return buckets;
}

function buildAudienceHoverStats(metrics: AudienceHoverMetrics): MetricHoverContent["stats"] {
  return [
    {
      label: "Revenue",
      value: formatRevenueAmount(metrics.revenue),
      changeRatio: metrics.revenueChangeRatio,
    },
    {
      label: "Visitors",
      value: metrics.visitors.toLocaleString(),
      changeRatio: metrics.visitorsChangeRatio,
    },
    {
      label: "Conversions",
      value: metrics.convertingVisitors.toLocaleString(),
      changeRatio: metrics.conversionsChangeRatio,
    },
    {
      label: "Revenue / visitor",
      value: formatRevenueAmount(metrics.revenuePerVisitor),
    },
  ];
}

/**
 * Hover-card content for a country: revenue and visitors with period-over-period
 * change, plus conversions, revenue per visitor, and conversion rate.
 */
export function buildCountryHoverContent(metrics: CountryMetrics): MetricHoverContent {
  return {
    stats: [
      ...buildAudienceHoverStats(metrics),
      {
        label: "Conversion rate",
        value: formatPercentRatio(metrics.conversionRate, 2),
      },
      {
        label: "95% CI",
        value: formatWilsonScoreInterval(metrics.conversionCi ?? null, 2),
      },
    ],
  };
}

/** Hover-card content for a city row (no conversion rate — too noisy at city grain). */
export function buildCityHoverContent(metrics: CityMetrics): MetricHoverContent {
  return { stats: buildAudienceHoverStats(metrics) };
}

function buildKeyedTechMetrics<T extends RowMetrics>(
  data: { current: T[] },
  keyFn: (row: T) => string,
  nameFn: (row: T) => string,
): Map<string, TechMetrics> {
  const metrics = new Map<string, TechMetrics>();
  for (const row of data.current) {
    const key = keyFn(row);
    metrics.set(key, {
      ...hoverMetricsFrom(row),
      key,
      name: nameFn(row),
    });
  }
  return metrics;
}

export function buildBrowserMetrics(browsers: AudienceBrowsers): Map<string, TechMetrics> {
  return buildKeyedTechMetrics(
    browsers,
    (row) => row.browser_family,
    (row) => row.browser_family_name || row.browser_family,
  );
}

export function buildOsMetrics(os: AudienceOs): Map<string, TechMetrics> {
  return buildKeyedTechMetrics(
    os,
    (row) => row.os,
    (row) => row.os_name || row.os,
  );
}

export function buildDeviceMetrics(devices: AudienceDevices): Map<string, TechMetrics> {
  return buildKeyedTechMetrics(
    devices,
    (row) => row.device,
    (row) => row.device_name || row.device,
  );
}

export function buildScreenSizeMetrics(screenSizes: AudienceScreenSizes): Map<string, TechMetrics> {
  return buildKeyedTechMetrics(
    screenSizes,
    (row) => row.screen_size,
    (row) => widthBucketLabel(row.screen_size),
  );
}

export function buildViewportMetrics(viewports: AudienceViewports): Map<string, TechMetrics> {
  return buildKeyedTechMetrics(
    viewports,
    (row) => row.viewport,
    (row) => widthBucketLabel(row.viewport),
  );
}

/** Hover-card content for browser, OS, device, and size breakdown rows. */
export function buildTechHoverContent(metrics: TechMetrics): MetricHoverContent {
  return { stats: buildAudienceHoverStats(metrics) };
}
