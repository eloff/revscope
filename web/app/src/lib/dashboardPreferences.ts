import { makePersisted } from "@solid-primitives/storage";
import { createSignal } from "solid-js";
import { isServer } from "solid-js/web";
import {
  type AudienceBreakdownColumn,
  type AudienceBreakdownSortState,
  DEFAULT_AUDIENCE_BREAKDOWN_SORT,
} from "../components/dashboard/audienceLogic";
import {
  type ChartMetricVisibility,
  createDefaultChartVisibility,
  DEFAULT_BAR_SERIES,
  LINE_METRIC_KEYS,
  type LineMetricKey,
} from "../components/dashboard/chartMetrics";
import {
  CONVERSIONS_SORT_COLUMNS,
  type ConversionsSortColumn,
  type ConversionsSortState,
  DEFAULT_CONVERSIONS_SORT,
} from "../components/dashboard/conversionsLogic";
import {
  CRAWLER_CATEGORY_ORDER,
  type CrawlerCategory,
} from "../components/dashboard/crawlersLogic";
import {
  DEFAULT_PAGES_SORT,
  PAGES_SORT_COLUMNS,
  type PagesSortColumn,
  type PagesSortState,
} from "../components/dashboard/pagesLogic";
import {
  DEFAULT_SORT,
  type SortColumn as RevenueSourceSortColumn,
  SOURCE_GROUP_ORDER,
  type SortState,
  type SourceGroup,
} from "../components/dashboard/revenueSourcesLogic";
import {
  CRAWLERS_SORT_COLUMNS,
  type CrawlersOrderBy,
  type CrawlersSortState,
  DEFAULT_CRAWLERS_SORT,
} from "./crawlersApi";
import {
  DEFAULT_EVENTS_SORT,
  EVENTS_SORT_COLUMNS,
  type EventsOrderBy,
  type EventsSortState,
} from "./eventsApi";
import {
  DEFAULT_SEARCH_TERMS_SORT,
  SEARCH_TERMS_ORDER_BY_COLUMNS,
  type SearchTermsOrderBy,
  type SearchTermsSortState,
} from "./searchTermsApi";
import {
  DEFAULT_VISITOR_SORT,
  VISITOR_ORDER_BY_COLUMNS,
  type VisitorOrderBy,
  type VisitorSortState,
} from "./visitorsApi";

const STORAGE_PREFIX = "revscope.dashboard";

const clientStorage = (): Storage | undefined => (isServer ? undefined : localStorage);

const REVENUE_SOURCE_SORT_COLUMNS = new Set<RevenueSourceSortColumn>([
  "group",
  "source",
  "landingPaths",
  "visitors",
  "revenue",
  "conversion",
]);

const LEGACY_SORT_COLUMNS = new Set([
  "visitorsChange",
  "revenueChange",
  "revenuePerVisitor",
  "adjustedConversion",
]);

const SEARCH_TERMS_ORDER_BY_SET = new Set<SearchTermsOrderBy>(SEARCH_TERMS_ORDER_BY_COLUMNS);

const EVENTS_SORT_COLUMN_SET = new Set<EventsOrderBy>(EVENTS_SORT_COLUMNS);

const CRAWLERS_SORT_COLUMN_SET = new Set<CrawlersOrderBy>(CRAWLERS_SORT_COLUMNS);

const CONVERSIONS_SORT_COLUMN_SET = new Set<ConversionsSortColumn>(CONVERSIONS_SORT_COLUMNS);

const PAGES_SORT_COLUMN_SET = new Set<PagesSortColumn>(PAGES_SORT_COLUMNS);

const VISITOR_ORDER_BY_COLUMN_SET = new Set<VisitorOrderBy>(VISITOR_ORDER_BY_COLUMNS);

const AUDIENCE_BREAKDOWN_SORT_COLUMNS = new Set<AudienceBreakdownColumn>([
  "name",
  "visitors",
  "revenue",
  "conversion",
]);

/** Conversion sort is country-only; other dimensions omit that column. */
const AUDIENCE_BREAKDOWN_SORT_COLUMNS_WITHOUT_CONVERSION = new Set<AudienceBreakdownColumn>([
  "name",
  "visitors",
  "revenue",
]);

export type AudienceBreakdownPreferenceKey =
  | "country"
  | "cities"
  | "browser"
  | "os"
  | "device"
  | "screenSize"
  | "viewport";

export type AudienceBreakdownSortPreferences = Record<
  AudienceBreakdownPreferenceKey,
  AudienceBreakdownSortState
>;

function isLineMetricKey(value: unknown): value is LineMetricKey {
  return typeof value === "string" && (LINE_METRIC_KEYS as readonly string[]).includes(value);
}

function parseStoredJson(raw: string): unknown {
  try {
    return JSON.parse(raw) as unknown;
  } catch {
    return undefined;
  }
}

function parseChartVisibility(raw: unknown): ChartMetricVisibility {
  const defaults = createDefaultChartVisibility();
  if (!raw || typeof raw !== "object") {
    return defaults;
  }
  const value = raw as Partial<ChartMetricVisibility>;
  const barSeries =
    value.barSeries && typeof value.barSeries === "object"
      ? { ...DEFAULT_BAR_SERIES, ...value.barSeries }
      : defaults.barSeries;
  return {
    lineMetric:
      value.lineMetric === null || isLineMetricKey(value.lineMetric)
        ? value.lineMetric
        : defaults.lineMetric,
    barEnabled: typeof value.barEnabled === "boolean" ? value.barEnabled : defaults.barEnabled,
    barSeries,
  };
}

function parseDisabledSourceGroups(raw: unknown): SourceGroup[] {
  if (!Array.isArray(raw)) {
    return [];
  }
  return raw.filter(
    (group): group is SourceGroup =>
      typeof group === "string" && SOURCE_GROUP_ORDER.includes(group as SourceGroup),
  );
}

function parseRevenueSourceSortState(raw: unknown): SortState {
  if (!raw || typeof raw !== "object") {
    return DEFAULT_SORT;
  }
  const value = raw as Partial<SortState>;
  if (
    typeof value.column !== "string" ||
    LEGACY_SORT_COLUMNS.has(value.column) ||
    !REVENUE_SOURCE_SORT_COLUMNS.has(value.column as RevenueSourceSortColumn) ||
    (value.direction !== "asc" && value.direction !== "desc")
  ) {
    return DEFAULT_SORT;
  }
  return {
    column: value.column as RevenueSourceSortColumn,
    direction: value.direction,
  };
}

function parseConversionsSortState(raw: unknown): ConversionsSortState {
  if (!raw || typeof raw !== "object") {
    return DEFAULT_CONVERSIONS_SORT;
  }
  const value = raw as Partial<ConversionsSortState>;
  if (
    typeof value.column !== "string" ||
    !CONVERSIONS_SORT_COLUMN_SET.has(value.column as ConversionsSortColumn) ||
    (value.direction !== "asc" && value.direction !== "desc")
  ) {
    return DEFAULT_CONVERSIONS_SORT;
  }
  return {
    column: value.column as ConversionsSortColumn,
    direction: value.direction,
  };
}

function parseDisabledCrawlerCategories(raw: unknown): CrawlerCategory[] {
  if (!Array.isArray(raw)) {
    return [];
  }
  return raw.filter(
    (category): category is CrawlerCategory =>
      typeof category === "string" && CRAWLER_CATEGORY_ORDER.includes(category as CrawlerCategory),
  );
}

function parseCrawlersSortState(raw: unknown): CrawlersSortState {
  if (!raw || typeof raw !== "object") {
    return DEFAULT_CRAWLERS_SORT;
  }
  const value = raw as Partial<CrawlersSortState>;
  if (
    typeof value.column !== "string" ||
    !CRAWLERS_SORT_COLUMN_SET.has(value.column as CrawlersOrderBy) ||
    (value.direction !== "asc" && value.direction !== "desc")
  ) {
    return DEFAULT_CRAWLERS_SORT;
  }
  return {
    column: value.column as CrawlersOrderBy,
    direction: value.direction,
  };
}

function parseEventsSortState(raw: unknown): EventsSortState {
  if (!raw || typeof raw !== "object") {
    return DEFAULT_EVENTS_SORT;
  }
  const value = raw as Partial<EventsSortState>;
  if (
    typeof value.column !== "string" ||
    !EVENTS_SORT_COLUMN_SET.has(value.column as EventsOrderBy) ||
    (value.direction !== "asc" && value.direction !== "desc")
  ) {
    return DEFAULT_EVENTS_SORT;
  }
  return {
    column: value.column as EventsOrderBy,
    direction: value.direction,
  };
}

function parseSearchTermsSortState(raw: unknown): SearchTermsSortState {
  if (!raw || typeof raw !== "object") {
    return DEFAULT_SEARCH_TERMS_SORT;
  }
  const value = raw as Partial<SearchTermsSortState> & { column?: string };
  // Prefer orderBy (visitors-style); accept legacy `column` from older prefs.
  const orderBy =
    typeof value.orderBy === "string"
      ? value.orderBy
      : typeof value.column === "string"
        ? value.column
        : undefined;
  if (
    typeof orderBy !== "string" ||
    !SEARCH_TERMS_ORDER_BY_SET.has(orderBy as SearchTermsOrderBy) ||
    (value.direction !== "asc" && value.direction !== "desc")
  ) {
    return DEFAULT_SEARCH_TERMS_SORT;
  }
  return {
    orderBy: orderBy as SearchTermsOrderBy,
    direction: value.direction,
  };
}

function parsePagesSortState(raw: unknown): PagesSortState {
  if (!raw || typeof raw !== "object") {
    return DEFAULT_PAGES_SORT;
  }
  const value = raw as Partial<PagesSortState>;
  if (
    typeof value.column !== "string" ||
    !PAGES_SORT_COLUMN_SET.has(value.column as PagesSortColumn) ||
    (value.direction !== "asc" && value.direction !== "desc")
  ) {
    return DEFAULT_PAGES_SORT;
  }
  return {
    column: value.column as PagesSortColumn,
    direction: value.direction,
  };
}

function parseVisitorsSortState(raw: unknown): VisitorSortState {
  if (!raw || typeof raw !== "object") {
    return DEFAULT_VISITOR_SORT;
  }
  const value = raw as Partial<VisitorSortState>;
  if (
    typeof value.orderBy !== "string" ||
    !VISITOR_ORDER_BY_COLUMN_SET.has(value.orderBy as VisitorOrderBy) ||
    (value.direction !== "asc" && value.direction !== "desc")
  ) {
    return DEFAULT_VISITOR_SORT;
  }
  return {
    orderBy: value.orderBy as VisitorOrderBy,
    direction: value.direction,
  };
}

function parseAudienceBreakdownSortState(
  raw: unknown,
  allowedColumns: ReadonlySet<AudienceBreakdownColumn> = AUDIENCE_BREAKDOWN_SORT_COLUMNS,
): AudienceBreakdownSortState {
  if (!raw || typeof raw !== "object") {
    return DEFAULT_AUDIENCE_BREAKDOWN_SORT;
  }
  const value = raw as Partial<AudienceBreakdownSortState>;
  if (
    typeof value.column !== "string" ||
    !allowedColumns.has(value.column as AudienceBreakdownColumn) ||
    (value.direction !== "asc" && value.direction !== "desc")
  ) {
    return DEFAULT_AUDIENCE_BREAKDOWN_SORT;
  }
  return {
    column: value.column as AudienceBreakdownColumn,
    direction: value.direction,
  };
}

function createDefaultAudienceBreakdownSorts(): AudienceBreakdownSortPreferences {
  return {
    country: { ...DEFAULT_AUDIENCE_BREAKDOWN_SORT },
    cities: { ...DEFAULT_AUDIENCE_BREAKDOWN_SORT },
    browser: { ...DEFAULT_AUDIENCE_BREAKDOWN_SORT },
    os: { ...DEFAULT_AUDIENCE_BREAKDOWN_SORT },
    device: { ...DEFAULT_AUDIENCE_BREAKDOWN_SORT },
    screenSize: { ...DEFAULT_AUDIENCE_BREAKDOWN_SORT },
    viewport: { ...DEFAULT_AUDIENCE_BREAKDOWN_SORT },
  };
}

function parseAudienceBreakdownSorts(raw: unknown): AudienceBreakdownSortPreferences {
  const defaults = createDefaultAudienceBreakdownSorts();
  if (!raw || typeof raw !== "object") {
    return defaults;
  }
  const value = raw as Partial<Record<AudienceBreakdownPreferenceKey, unknown>>;
  return {
    country: parseAudienceBreakdownSortState(value.country),
    cities: parseAudienceBreakdownSortState(
      value.cities,
      AUDIENCE_BREAKDOWN_SORT_COLUMNS_WITHOUT_CONVERSION,
    ),
    browser: parseAudienceBreakdownSortState(
      value.browser,
      AUDIENCE_BREAKDOWN_SORT_COLUMNS_WITHOUT_CONVERSION,
    ),
    os: parseAudienceBreakdownSortState(
      value.os,
      AUDIENCE_BREAKDOWN_SORT_COLUMNS_WITHOUT_CONVERSION,
    ),
    device: parseAudienceBreakdownSortState(
      value.device,
      AUDIENCE_BREAKDOWN_SORT_COLUMNS_WITHOUT_CONVERSION,
    ),
    screenSize: parseAudienceBreakdownSortState(
      value.screenSize,
      AUDIENCE_BREAKDOWN_SORT_COLUMNS_WITHOUT_CONVERSION,
    ),
    viewport: parseAudienceBreakdownSortState(
      value.viewport,
      AUDIENCE_BREAKDOWN_SORT_COLUMNS_WITHOUT_CONVERSION,
    ),
  };
}

export const [chartVisibility, setChartVisibility] = makePersisted(
  createSignal<ChartMetricVisibility>(createDefaultChartVisibility()),
  {
    name: `${STORAGE_PREFIX}.chartVisibility`,
    storage: clientStorage(),
    deserialize: (raw) => parseChartVisibility(parseStoredJson(raw)),
  },
);

export const [disabledSourceGroups, setDisabledSourceGroups] = makePersisted(
  createSignal<SourceGroup[]>([]),
  {
    name: `${STORAGE_PREFIX}.disabledSourceGroups`,
    storage: clientStorage(),
    deserialize: (raw) => parseDisabledSourceGroups(parseStoredJson(raw)),
  },
);

export const [revenueSourceSort, setRevenueSourceSort] = makePersisted(
  createSignal<SortState>(DEFAULT_SORT),
  {
    name: `${STORAGE_PREFIX}.revenueSourceSort`,
    storage: clientStorage(),
    deserialize: (raw) => parseRevenueSourceSortState(parseStoredJson(raw)),
  },
);

export const [searchTermsSort, setSearchTermsSort] = makePersisted(
  createSignal<SearchTermsSortState>(DEFAULT_SEARCH_TERMS_SORT),
  {
    name: `${STORAGE_PREFIX}.searchTermsSort`,
    storage: clientStorage(),
    deserialize: (raw) => parseSearchTermsSortState(parseStoredJson(raw)),
  },
);

export const [conversionsSort, setConversionsSort] = makePersisted(
  createSignal<ConversionsSortState>(DEFAULT_CONVERSIONS_SORT),
  {
    name: `${STORAGE_PREFIX}.conversionsSort`,
    storage: clientStorage(),
    deserialize: (raw) => parseConversionsSortState(parseStoredJson(raw)),
  },
);

export const [pagesSort, setPagesSort] = makePersisted(
  createSignal<PagesSortState>(DEFAULT_PAGES_SORT),
  {
    name: `${STORAGE_PREFIX}.pagesSort`,
    storage: clientStorage(),
    deserialize: (raw) => parsePagesSortState(parseStoredJson(raw)),
  },
);

export const [landingPagesSort, setLandingPagesSort] = makePersisted(
  createSignal<PagesSortState>(DEFAULT_PAGES_SORT),
  {
    name: `${STORAGE_PREFIX}.landingPagesSort`,
    storage: clientStorage(),
    deserialize: (raw) => parsePagesSortState(parseStoredJson(raw)),
  },
);

export const [eventsSort, setEventsSort] = makePersisted(
  createSignal<EventsSortState>(DEFAULT_EVENTS_SORT),
  {
    name: `${STORAGE_PREFIX}.eventsSort`,
    storage: clientStorage(),
    deserialize: (raw) => parseEventsSortState(parseStoredJson(raw)),
  },
);

export const [disabledCrawlerCategories, setDisabledCrawlerCategories] = makePersisted(
  createSignal<CrawlerCategory[]>([]),
  {
    name: `${STORAGE_PREFIX}.disabledCrawlerCategories`,
    storage: clientStorage(),
    deserialize: (raw) => parseDisabledCrawlerCategories(parseStoredJson(raw)),
  },
);

export const [crawlersSort, setCrawlersSort] = makePersisted(
  createSignal<CrawlersSortState>(DEFAULT_CRAWLERS_SORT),
  {
    name: `${STORAGE_PREFIX}.crawlersSort`,
    storage: clientStorage(),
    deserialize: (raw) => parseCrawlersSortState(parseStoredJson(raw)),
  },
);

export const [visitorsSort, setVisitorsSort] = makePersisted(
  createSignal<VisitorSortState>(DEFAULT_VISITOR_SORT),
  {
    name: `${STORAGE_PREFIX}.visitorsSort`,
    storage: clientStorage(),
    deserialize: (raw) => parseVisitorsSortState(parseStoredJson(raw)),
  },
);

export const [audienceBreakdownSorts, setAudienceBreakdownSorts] = makePersisted(
  createSignal<AudienceBreakdownSortPreferences>(createDefaultAudienceBreakdownSorts()),
  {
    name: `${STORAGE_PREFIX}.audienceBreakdownSorts`,
    storage: clientStorage(),
    deserialize: (raw) => parseAudienceBreakdownSorts(parseStoredJson(raw)),
  },
);

export function disabledSourceGroupsSet(): Set<SourceGroup> {
  return new Set(disabledSourceGroups());
}

export function updateDisabledSourceGroups(
  updater: (previous: Set<SourceGroup>) => Set<SourceGroup>,
): void {
  setDisabledSourceGroups(Array.from(updater(disabledSourceGroupsSet())));
}

export function disabledCrawlerCategoriesSet(): Set<CrawlerCategory> {
  return new Set(disabledCrawlerCategories());
}

export function updateDisabledCrawlerCategories(
  updater: (previous: Set<CrawlerCategory>) => Set<CrawlerCategory>,
): void {
  setDisabledCrawlerCategories(Array.from(updater(disabledCrawlerCategoriesSet())));
}

export function setAudienceBreakdownSort(
  key: AudienceBreakdownPreferenceKey,
  sort: AudienceBreakdownSortState,
): void {
  setAudienceBreakdownSorts((previous) => ({
    ...previous,
    [key]: sort,
  }));
}

export function useDashboardPreferences() {
  return {
    chartVisibility,
    setChartVisibility,
    disabledSourceGroups,
    setDisabledSourceGroups,
    disabledSourceGroupsSet,
    updateDisabledSourceGroups,
    disabledCrawlerCategories,
    setDisabledCrawlerCategories,
    disabledCrawlerCategoriesSet,
    updateDisabledCrawlerCategories,
    crawlersSort,
    setCrawlersSort,
    revenueSourceSort,
    setRevenueSourceSort,
    searchTermsSort,
    setSearchTermsSort,
    conversionsSort,
    setConversionsSort,
    pagesSort,
    setPagesSort,
    landingPagesSort,
    setLandingPagesSort,
    eventsSort,
    setEventsSort,
    visitorsSort,
    setVisitorsSort,
    audienceBreakdownSorts,
    setAudienceBreakdownSorts,
    setAudienceBreakdownSort,
  };
}
