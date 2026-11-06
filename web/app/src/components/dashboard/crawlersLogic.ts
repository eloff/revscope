import {
  type CrawlerCategory,
  type CrawlerOperatorRow,
  type CrawlerRow,
  type CrawlerSource,
  type CrawlersSortState,
  DEFAULT_CRAWLERS_SORT,
} from "../../lib/crawlersApi";
import { nextColumnSort } from "./columnSort";

export type { CrawlerCategory };

export const CRAWLER_CATEGORY_ORDER: CrawlerCategory[] = [
  "ai_crawler",
  "ai_search",
  "ai_assistant",
  "ai_browser",
  "search_engine",
  "archive",
  "headless",
];

export const CRAWLER_CATEGORY_LABELS: Record<CrawlerCategory, string> = {
  ai_crawler: "AI crawler",
  ai_search: "AI search",
  ai_assistant: "AI assistant",
  ai_browser: "AI browser",
  search_engine: "Search engine",
  archive: "Archive",
  headless: "Headless browser",
};

export const CRAWLER_CATEGORY_COLORS: Record<CrawlerCategory, string> = {
  ai_crawler: "#9333ea",
  ai_search: "#2563eb",
  ai_assistant: "#0ea5e9",
  ai_browser: "#16a34a",
  search_engine: "#f59e0b",
  archive: "#78716c",
  headless: "#6b7280",
};

export const UNKNOWN_OPERATOR_LABEL = "Unknown";
export const UNVERIFIED_OPERATOR_LABEL = "Unverified";
export const CRAWLERS_TREEMAP_TOP_OPERATORS = 20;
export const CRAWLERS_TREEMAP_OTHER_KEY = "__crawlers_other__";

/** Display names for slugs whose brand casing is not first-letter capitalize. */
const CRAWLER_OPERATOR_LABELS: Record<string, string> = {
  openai: "OpenAI",
  duckduckgo: "DuckDuckGo",
  commoncrawl: "Common Crawl",
};

const OPERATOR_COLORS = [
  "#2563eb",
  "#9333ea",
  "#16a34a",
  "#f59e0b",
  "#0ea5e9",
  "#db2777",
  "#ea580c",
  "#7c3aed",
  "#0891b2",
  "#65a30d",
];

export type CrawlerOperatorLeaf = {
  datumKey: string;
  label: string;
  views: number;
  share: number;
  color: string;
  isOther: boolean;
  /** Number of operators rolled into the Other leaf; 0 otherwise. */
  otherCount: number;
};

export function crawlerRowKey(row: CrawlerRow): string {
  return `${row.useragent_id}:${row.source}:${row.verified ? 1 : 0}`;
}

export function categoryForRow(row: { category?: string }): CrawlerCategory {
  const category = row.category;
  if (category && (CRAWLER_CATEGORY_ORDER as readonly string[]).includes(category)) {
    return category as CrawlerCategory;
  }
  return "headless";
}

function capitalizeOperatorLabel(label: string): string {
  if (label.length === 0) {
    return label;
  }
  return label.charAt(0).toUpperCase() + label.slice(1);
}

export function operatorTrustTitle(row: { source: CrawlerSource; verified: boolean }): string {
  if (!row.verified) {
    return "Unverified user agent";
  }
  if (row.source === 1) {
    return "Reported by Cloudflare";
  }
  return "IP matched operator's published ranges";
}

export function operatorLabelForRow(row: {
  verified: boolean;
  operator?: string;
  ip?: string;
}): string {
  if (!row.verified) {
    const ip = row.ip?.trim();
    if (ip) {
      return ip;
    }
    return UNKNOWN_OPERATOR_LABEL;
  }
  const slug = row.operator;
  if (!slug) {
    return UNKNOWN_OPERATOR_LABEL;
  }
  return CRAWLER_OPERATOR_LABELS[slug] ?? capitalizeOperatorLabel(slug);
}

export function isCrawlerCategoryEnabled(
  category: CrawlerCategory,
  disabledCategories: ReadonlySet<CrawlerCategory>,
): boolean {
  return !disabledCategories.has(category);
}

export function toggleDisabledCrawlerCategory(
  category: CrawlerCategory,
  disabledCategories: ReadonlySet<CrawlerCategory>,
): Set<CrawlerCategory> {
  const next = new Set(disabledCategories);
  if (next.has(category)) {
    next.delete(category);
  } else {
    next.add(category);
  }
  return next;
}

/** True when `category` is the only enabled crawler category. */
export function isCrawlerCategorySoloed(
  category: CrawlerCategory,
  disabledCategories: ReadonlySet<CrawlerCategory>,
): boolean {
  return (
    disabledCategories.size === CRAWLER_CATEGORY_ORDER.length - 1 &&
    !disabledCategories.has(category)
  );
}

/** Solo `category`, or select all categories if it is already the only one enabled. */
export function soloCrawlerCategory(
  category: CrawlerCategory,
  disabledCategories: ReadonlySet<CrawlerCategory> = new Set(),
): Set<CrawlerCategory> {
  if (isCrawlerCategorySoloed(category, disabledCategories)) {
    return new Set();
  }
  return new Set(CRAWLER_CATEGORY_ORDER.filter((candidate) => candidate !== category));
}

export function filterRowsByCategories<T extends { category?: string }>(
  rows: T[],
  disabledCategories: ReadonlySet<CrawlerCategory>,
): T[] {
  if (disabledCategories.size === 0) {
    return rows;
  }
  return rows.filter((row) => isCrawlerCategoryEnabled(categoryForRow(row), disabledCategories));
}

/**
 * Treemap grouping for a row: verified rows by operator, all unverified rows
 * in one bucket (the grid still shows their individual IPs).
 */
export function treemapOperatorLabelForRow(row: {
  verified: boolean;
  operator?: string;
  ip?: string;
}): string {
  return row.verified ? operatorLabelForRow(row) : UNVERIFIED_OPERATOR_LABEL;
}

function operatorColor(label: string): string {
  if (label === UNKNOWN_OPERATOR_LABEL || label === UNVERIFIED_OPERATOR_LABEL) {
    return CRAWLER_CATEGORY_COLORS.headless;
  }
  let hash = 0;
  for (let index = 0; index < label.length; index += 1) {
    hash = (hash * 31 + label.charCodeAt(index)) >>> 0;
  }
  return OPERATOR_COLORS[hash % OPERATOR_COLORS.length] ?? OPERATOR_COLORS[0];
}

export function buildOperatorLeaves(
  rows: Array<CrawlerRow | CrawlerOperatorRow>,
): CrawlerOperatorLeaf[] {
  const viewsByOperator = new Map<string, number>();
  for (const row of rows) {
    const label = treemapOperatorLabelForRow(row);
    viewsByOperator.set(label, (viewsByOperator.get(label) ?? 0) + row.views);
  }
  const total = [...viewsByOperator.values()].reduce((sum, views) => sum + views, 0);
  const share = (views: number) => (total > 0 ? views / total : 0);
  const ranked = [...viewsByOperator.entries()].sort((left, right) => {
    if (right[1] !== left[1]) {
      return right[1] - left[1];
    }
    return left[0].localeCompare(right[0]);
  });
  const leaves: CrawlerOperatorLeaf[] = ranked
    .slice(0, CRAWLERS_TREEMAP_TOP_OPERATORS)
    .map(([label, views]) => ({
      datumKey: label,
      label,
      views,
      share: share(views),
      color: operatorColor(label),
      isOther: false,
      otherCount: 0,
    }));
  const rest = ranked.slice(CRAWLERS_TREEMAP_TOP_OPERATORS);
  if (rest.length > 0) {
    const views = rest.reduce((sum, [, operatorViews]) => sum + operatorViews, 0);
    leaves.push({
      datumKey: CRAWLERS_TREEMAP_OTHER_KEY,
      label: "Other",
      views,
      share: share(views),
      color: CRAWLER_CATEGORY_COLORS.headless,
      isOther: true,
      otherCount: rest.length,
    });
  }
  return leaves;
}

export function nextCrawlersSortState(
  sort: CrawlersSortState,
  column: CrawlersSortState["column"],
): CrawlersSortState {
  return nextColumnSort(sort, column, column === "views" ? "desc" : "asc");
}

/** True when the table sort matches the ranking the crawlers treemap uses. */
export function isCrawlersTreemapSort(sort: CrawlersSortState): boolean {
  return (
    sort.column === DEFAULT_CRAWLERS_SORT.column &&
    sort.direction === DEFAULT_CRAWLERS_SORT.direction
  );
}
