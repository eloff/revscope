import { getApiBaseUrl } from "@revscope/web-shared/apiBaseUrl";
import { projectApiPath } from "@revscope/web-shared/apiPaths";
import { authRequestInit, fetchJson } from "@revscope/web-shared/apiRequest";
import type { AuthSession } from "@revscope/web-shared/authState";
import { type BucketGranularity, bucketGranularityForRange } from "./dashboardApi";

export type CrawlerSource = 0 | 1;

export type CrawlerCategory =
  | "ai_crawler"
  | "ai_search"
  | "ai_assistant"
  | "ai_browser"
  | "archive"
  | "headless"
  | "search_engine";

export const CRAWLERS_PAGE_SIZE = 100;

export type CrawlersOrderBy = "views" | "operator" | "category" | "useragent";

export type CrawlersSortDirection = "asc" | "desc";

export type CrawlersSortState = {
  column: CrawlersOrderBy;
  direction: CrawlersSortDirection;
};

export const CRAWLERS_SORT_COLUMNS = [
  "views",
  "operator",
  "category",
  "useragent",
] as const satisfies readonly CrawlersOrderBy[];

export const DEFAULT_CRAWLERS_SORT: CrawlersSortState = {
  column: "views",
  direction: "desc",
};

export type CrawlerRow = {
  useragent_id: number;
  source: CrawlerSource;
  verified: boolean;
  useragent: string;
  ip?: string;
  operator?: string;
  category?: string;
  views: number;
};

export type CrawlerOperatorRow = {
  source: CrawlerSource;
  verified: boolean;
  operator?: string;
  ip?: string;
  category?: string;
  views: number;
};

export type CrawlerCategoryTotals = Record<CrawlerCategory, number>;

export type CrawlersResponse = {
  rows: CrawlerRow[];
  category_totals: CrawlerCategoryTotals;
  operators: CrawlerOperatorRow[];
  next_offset?: number;
  offset: number;
  limit: number;
};

export type CrawlerSeriesPoint = {
  start: number;
  views: number;
};

/** Each `CrawlerPageRow.last_3_days` value is views in one 4-hour bucket. */
export const CRAWLER_LAST_3_DAYS_BUCKET_SECONDS = 4 * 3600;
export const CRAWLER_LAST_3_DAYS_BUCKETS = 18;
const ZERO_LAST_3_DAYS = Array.from({ length: CRAWLER_LAST_3_DAYS_BUCKETS }, () => 0);

/** Empty `last_3_days` means every 4-hour bucket is zero. */
export function last3DaysValues(values: number[] | undefined): number[] {
  return values && values.length > 0 ? values : ZERO_LAST_3_DAYS;
}

export type CrawlerPageRow = {
  path_id: number;
  path: string;
  hostname: string;
  views: number;
  /** Views per 4-hour bucket starting at `CrawlerDetailResponse.last_3_days_start`. Empty means all zeros. */
  last_3_days: number[];
};

export type CrawlerDetailResponse = {
  granularity: "hour" | "day" | "week";
  series: CrawlerSeriesPoint[];
  pages: CrawlerPageRow[];
  /** Unix second at the start of `pages[].last_3_days[0]`. */
  last_3_days_start: number;
};

function crawlersOrderBy(column: string): CrawlersOrderBy {
  if ((CRAWLERS_SORT_COLUMNS as readonly string[]).includes(column)) {
    return column as CrawlersOrderBy;
  }
  return DEFAULT_CRAWLERS_SORT.column;
}

export async function fetchCrawlers(
  session: AuthSession | undefined,
  projectId: string,
  start: number,
  end: number,
  offset: number,
  sort: CrawlersSortState,
  disabledCategories: readonly CrawlerCategory[] = [],
): Promise<CrawlersResponse> {
  const url = new URL(projectApiPath("read", projectId, "crawlers"), getApiBaseUrl());
  url.searchParams.set("start", String(start));
  url.searchParams.set("end", String(end));
  url.searchParams.set("offset", String(offset));
  url.searchParams.set("order_by", crawlersOrderBy(sort.column));
  url.searchParams.set("direction", sort.direction);
  for (const category of disabledCategories) {
    url.searchParams.append("disabled_category", category);
  }

  return fetchJson<CrawlersResponse>(
    url,
    authRequestInit(session),
    "Unable to load crawlers data.",
  );
}

export async function fetchCrawlerDetail(
  session: AuthSession | undefined,
  projectId: string,
  start: number,
  end: number,
  useragentId: number,
  source: CrawlerSource,
  verified: boolean,
  bucket: BucketGranularity = bucketGranularityForRange(start, end),
): Promise<CrawlerDetailResponse> {
  const url = new URL(projectApiPath("read", projectId, "crawler-detail"), getApiBaseUrl());
  url.searchParams.set("start", String(start));
  url.searchParams.set("end", String(end));
  url.searchParams.set("useragent_id", String(useragentId));
  url.searchParams.set("source", String(source));
  url.searchParams.set("verified", verified ? "1" : "0");
  url.searchParams.set("bucket", bucket);

  return fetchJson<CrawlerDetailResponse>(
    url,
    authRequestInit(session),
    "Unable to load crawler details.",
  );
}
