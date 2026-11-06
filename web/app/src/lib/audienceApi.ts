import { getApiBaseUrl } from "@revscope/web-shared/apiBaseUrl";
import { projectApiPath } from "@revscope/web-shared/apiPaths";
import { authRequestInit, fetchJson } from "@revscope/web-shared/apiRequest";
import type { AuthSession } from "@revscope/web-shared/authState";
import type { MetricChange } from "./metricChange";
import { appendSegmentParams, EMPTY_SEGMENT_FILTERS, type SegmentFilters } from "./segmentFilters";

export const CITIES_PAGE_SIZE = 200;

export type AudienceCitiesOrderBy = "name" | "visitors" | "revenue";

export type AudienceCitiesSortState = {
  column: AudienceCitiesOrderBy;
  direction: "asc" | "desc";
};

export const DEFAULT_AUDIENCE_CITIES_SORT: AudienceCitiesSortState = {
  column: "revenue",
  direction: "desc",
};

function citiesOrderBy(column: string): AudienceCitiesOrderBy {
  if (column === "name" || column === "visitors" || column === "revenue") {
    return column;
  }
  return DEFAULT_AUDIENCE_CITIES_SORT.column;
}

/** 95% Wilson score confidence interval for a conversion rate (0..1 bounds). */
export type ConversionInterval = {
  lower: number;
  upper: number;
};

/**
 * Display-ready metrics shared by audience breakdown rows, computed server-side.
 * Counts and revenue are raw current-period values; `revenue_per_visitor` is
 * derived from the current period; the `*_change` fields compare against the
 * prior 30 days (normalized to the current period).
 *
 * Conversion rates are country-only — see {@link CountryRow}.
 */
export type RowMetrics = {
  visitors: number;
  /** First visits with attributed payment revenue. */
  converting_visitors: number;
  /** Sum of qualifying payment amounts (thousandths); excludes subscription renewals. */
  revenue: number;
  /** Revenue divided by visitors (thousandths). */
  revenue_per_visitor: number;
  /** Normalized revenue change versus the prior 30 days. */
  revenue_change: MetricChange;
  /** Normalized visitor change versus the prior 30 days. */
  visitors_change: MetricChange;
  /** Normalized conversion-count change versus the prior 30 days. */
  conversions_change: MetricChange;
};

/**
 * Per-country row. `country` is an ISO 3166-1 alpha-2 code (uppercase); it is
 * absent for landings with no resolved country. Conversion rate fields are
 * country-only.
 */
export type CountryRow = {
  country?: string;
  country_name: string;
  /** `converting_visitors` divided by `visitors`. */
  conversion_rate: number;
  /** Lower bound of the 95% Wilson interval, used for ranking. */
  conversion_lower_bound: number;
  /** 95% Wilson confidence interval for the conversion rate (absent when no visitors). */
  conversion_ci?: ConversionInterval;
} & RowMetrics;

/** Country breakdown for the requested window, with server-computed deltas. */
export type AudienceCountries = {
  current: CountryRow[];
};

/** Per-city row. */
export type CityRow = {
  /** City name (English) from GeoIP. */
  city: string;
  /** ISO 3166-1 alpha-2 code when resolved for this city row. */
  country?: string;
} & RowMetrics;

/**
 * City breakdown for the requested window, paginated (`limit` cities per page).
 * Ranked by the requested `order_by` (default: revenue, visitors, then city name).
 */
export type AudienceCities = {
  current: CityRow[];
  next_offset?: number;
  offset: number;
  limit: number;
};

/** Per-browser-family row. */
export type BrowserRow = {
  /** Stored slug (`chrome`, `firefox`, …) or `unknown` when not resolved. */
  browser_family: string;
  browser_family_name: string;
} & RowMetrics;

/** Browser-family breakdown for the requested window. */
export type AudienceBrowsers = {
  current: BrowserRow[];
};

/** Per-OS row. */
export type OsRow = {
  /** Stored slug (`windows`, `macos`, …) or `unknown` when not resolved. */
  os: string;
  os_name: string;
} & RowMetrics;

/** OS breakdown for the requested window. */
export type AudienceOs = {
  current: OsRow[];
};

/** Per-device row. */
export type DeviceRow = {
  /** Stored slug (`desktop`, `mobile`, …). */
  device: string;
  device_name: string;
} & RowMetrics;

/** Device breakdown for the requested window. */
export type AudienceDevices = {
  current: DeviceRow[];
};

/** Per screen-width bucket row. */
export type ScreenSizeRow = {
  /** Bucket slug (`small_mobile`, `mobile`, …). */
  screen_size: string;
} & RowMetrics;

/** Screen-width bucket breakdown for the requested window. */
export type AudienceScreenSizes = {
  current: ScreenSizeRow[];
};

/** Per viewport-width bucket row. */
export type ViewportRow = {
  /** Bucket slug (`small_mobile`, `mobile`, …). */
  viewport: string;
} & RowMetrics;

/** Viewport-width bucket breakdown for the requested window. */
export type AudienceViewports = {
  current: ViewportRow[];
};

/**
 * Audience response. Dimension data lives under named keys so more audience
 * breakdowns can be added as siblings later.
 */
export type AudienceResponse = {
  countries: AudienceCountries;
  cities: AudienceCities;
  browsers: AudienceBrowsers;
  os: AudienceOs;
  devices: AudienceDevices;
  screen_sizes: AudienceScreenSizes;
  viewports: AudienceViewports;
};

export async function fetchAudience(
  session: AuthSession | undefined,
  projectId: string,
  start: number,
  end: number,
  offset: number,
  sort: AudienceCitiesSortState,
  segments: SegmentFilters = EMPTY_SEGMENT_FILTERS,
): Promise<AudienceResponse> {
  const url = new URL(projectApiPath("read", projectId, "audience"), getApiBaseUrl());
  url.searchParams.set("start", String(start));
  url.searchParams.set("end", String(end));
  url.searchParams.set("offset", String(offset));
  url.searchParams.set("order_by", citiesOrderBy(sort.column));
  url.searchParams.set("direction", sort.direction);
  appendSegmentParams(url, segments);

  return fetchJson<AudienceResponse>(
    url,
    authRequestInit(session),
    "Unable to load audience data.",
  );
}
