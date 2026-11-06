import { type Accessor, createEffect } from "solid-js";
import {
  type AudienceCitiesSortState,
  type AudienceResponse,
  DEFAULT_AUDIENCE_CITIES_SORT,
} from "../../lib/audienceApi";
import type {
  CrawlerCategory,
  CrawlerDetailResponse,
  CrawlerSource,
  CrawlersResponse,
  CrawlersSortState,
} from "../../lib/crawlersApi";
import type { OverviewResponse } from "../../lib/dashboardApi";
import { rangesEqual } from "../../lib/dashboardDataCache";
import {
  audienceBreakdownSorts,
  crawlersSort,
  disabledCrawlerCategories,
  eventsSort,
  landingPagesSort,
  pagesSort,
  searchTermsSort,
  visitorsSort,
} from "../../lib/dashboardPreferences";
import type { DateRangeSeconds } from "../../lib/dateRange";
import type { EventsResponse } from "../../lib/eventsApi";
import {
  DEFAULT_PAGES_SORT,
  type PagesResponse,
  type PageTransitionsResponse,
  type PageTransitionsVariant,
} from "../../lib/pagesApi";
import type { PaymentActivityResponse } from "../../lib/paymentActivityApi";
import {
  CONVERSIONS_PAGE_SIZE,
  type ConversionsResponse,
  type PaymentJourneyResponse,
  type PaymentsFilter,
} from "../../lib/paymentsApi";
import type { RevenueSourceResponse } from "../../lib/revenueSourcesApi";
import { DEFAULT_SEARCH_TERMS_SORT, type SearchTermsResponse } from "../../lib/searchTermsApi";
import { type SegmentFilters, segmentFiltersKey } from "../../lib/segmentFilters";
import {
  type VisitorJourneyResponse,
  type VisitorSessionFilter,
  type VisitorSortState,
  type VisitorsResponse,
  visitorSessionFilterKey,
} from "../../lib/visitorsApi";
import { createCursorTabData, createOffsetTabData, createSnapshotData } from "./createTabData";
import type { DashboardProjectApi } from "./dashboardProjectApi";
import {
  createGenerationalCache,
  createRequestTracker,
  dateRangeInFlightKey,
} from "./dashboardRequests";
import type { DashboardRoute } from "./dashboardRoute";
import type { DashboardTab } from "./dashboardTabs";

function sortedPageKey(offset: number, sort: { column: string; direction: string }): string {
  return `offset:${offset}:order:${sort.column}:${sort.direction}`;
}

function visitorsPageKey(
  offset: number,
  sort: VisitorSortState,
  filter: VisitorSessionFilter | null | undefined,
): string {
  return `filter:${visitorSessionFilterKey(filter)}:offset:${offset}:order:${sort.orderBy}:${sort.direction}`;
}

function crawlersPageKey(
  offset: number,
  sort: CrawlersSortState,
  disabledCategories: readonly string[],
): string {
  const disabled =
    disabledCategories.length === 0 ? "none" : [...disabledCategories].sort().join(",");
  return `${sortedPageKey(offset, sort)}:disabled:${disabled}`;
}

function filteredPaymentsKey(filters: PaymentsFilter, cursor?: string): string {
  let filterKey = "all";
  if (filters.pathId !== undefined) {
    filterKey = `path:${filters.pathId}`;
  } else if (filters.landingPathId !== undefined) {
    filterKey = `landing:${filters.landingPathId}`;
  } else if (filters.campaignId !== undefined) {
    filterKey = `campaign:${filters.campaignId}`;
  } else if (filters.referrerId !== undefined) {
    filterKey = `referrer:${filters.referrerId}`;
  } else if (filters.direct) {
    filterKey = "direct";
  }
  return `${filterKey}:cursor:${cursor ?? "start"}`;
}

/** The cities table only sorts by the columns the API can order by. */
function audienceCitiesSort(): AudienceCitiesSortState {
  const sort = audienceBreakdownSorts().cities;
  const column =
    sort.column === "name" || sort.column === "visitors" || sort.column === "revenue"
      ? sort.column
      : "revenue";
  return { column, direction: sort.direction };
}

export type DashboardData = ReturnType<typeof createDashboardData>;

/**
 * Every tab's data plus the cached detail fetchers rows open on demand.
 * Clears itself when the range or segment filters change; payment activity
 * ignores the range, so only a filter change clears it.
 */
export function createDashboardData(options: {
  api: DashboardProjectApi;
  range: Accessor<DateRangeSeconds>;
  segments: Accessor<SegmentFilters>;
  visitorsFilter: Accessor<VisitorSessionFilter | null>;
}) {
  const { api, range, segments } = options;
  const requests = createRequestTracker();

  const overview = createSnapshotData<OverviewResponse>({
    requests,
    slot: "overview",
    errorFallback: "Unable to load overview data.",
    inFlightKey: dateRangeInFlightKey,
    fetch: (r) => api.overview(r.start, r.end, segments()),
  });
  const revenueSources = createSnapshotData<RevenueSourceResponse>({
    requests,
    slot: "revenueSources",
    errorFallback: "Unable to load sources data.",
    inFlightKey: dateRangeInFlightKey,
    fetch: (r) => api.revenueSources(r.start, r.end, segments()),
  });
  // Unlike the other tabs, payment activity isn't scoped to the selected date
  // range: it's a fixed trailing window, so only segment changes clear it.
  const paymentActivity = createSnapshotData<PaymentActivityResponse>({
    requests,
    slot: "paymentActivity",
    errorFallback: "Unable to load revenue activity.",
    inFlightKey: () => segmentFiltersKey(segments()),
    fetch: () => api.paymentActivity(segments()),
  });

  const searchTerms = createOffsetTabData<SearchTermsResponse, typeof DEFAULT_SEARCH_TERMS_SORT>({
    requests,
    name: "searchTerms",
    errorFallback: "Unable to load search terms data.",
    sort: searchTermsSort,
    cacheKey: (offset, sort) =>
      sortedPageKey(offset, { column: sort.orderBy, direction: sort.direction }),
    fetch: (r, offset, sort) => api.searchTerms(r.start, r.end, offset, sort, segments()),
    treemapSort: DEFAULT_SEARCH_TERMS_SORT,
  });
  const audience = createOffsetTabData<AudienceResponse, AudienceCitiesSortState>({
    requests,
    name: "audience",
    errorFallback: "Unable to load audience data.",
    sort: audienceCitiesSort,
    cacheKey: sortedPageKey,
    fetch: (r, offset, sort) => api.audience(r.start, r.end, offset, sort, segments()),
    treemapSort: DEFAULT_AUDIENCE_CITIES_SORT,
    // The countries and treemap stay up while only the cities table pages.
    clearOnFetch: false,
  });
  const pages = createOffsetTabData({
    requests,
    name: "pages",
    errorFallback: "Unable to load pages data.",
    sort: pagesSort,
    cacheKey: sortedPageKey,
    fetch: (r, offset, sort): Promise<PagesResponse> =>
      api.pages(r.start, r.end, offset, sort, segments()),
    treemapSort: DEFAULT_PAGES_SORT,
  });
  const landingPages = createOffsetTabData({
    requests,
    name: "landingPages",
    errorFallback: "Unable to load landing pages data.",
    sort: landingPagesSort,
    cacheKey: sortedPageKey,
    fetch: (r, offset, sort): Promise<PagesResponse> =>
      api.landingPages(r.start, r.end, offset, sort, segments()),
    treemapSort: DEFAULT_PAGES_SORT,
  });
  const events = createOffsetTabData({
    requests,
    name: "events",
    errorFallback: "Unable to load events data.",
    sort: eventsSort,
    cacheKey: sortedPageKey,
    fetch: (r, offset, sort): Promise<EventsResponse> =>
      api.events(r.start, r.end, offset, sort, segments()),
  });
  const crawlers = createOffsetTabData<
    CrawlersResponse,
    { sort: CrawlersSortState; disabled: readonly CrawlerCategory[] }
  >({
    requests,
    name: "crawlers",
    errorFallback: "Unable to load crawlers data.",
    sort: () => ({ sort: crawlersSort(), disabled: disabledCrawlerCategories() }),
    cacheKey: (offset, { sort, disabled }) => crawlersPageKey(offset, sort, disabled),
    fetch: (r, offset, { sort, disabled }) => api.crawlers(r.start, r.end, offset, sort, disabled),
  });
  const visitors = createOffsetTabData<
    VisitorsResponse,
    { sort: VisitorSortState; filter: VisitorSessionFilter | null }
  >({
    requests,
    name: "visitors",
    errorFallback: "Unable to load visitors data.",
    sort: () => ({ sort: visitorsSort(), filter: options.visitorsFilter() }),
    cacheKey: (offset, { sort, filter }) => visitorsPageKey(offset, sort, filter),
    fetch: (r, offset, { sort, filter }) =>
      api.visitors(r.start, r.end, offset, sort, filter, segments()),
  });
  const conversions = createCursorTabData<ConversionsResponse>({
    requests,
    name: "conversions",
    errorFallback: "Unable to load conversions data.",
    pageSize: CONVERSIONS_PAGE_SIZE,
    fetch: (r, cursor) => api.conversions(r.start, r.end, cursor, {}, segments()),
  });

  const filteredPaymentsCache = createGenerationalCache<ConversionsResponse>();
  const paymentJourneyCache = createGenerationalCache<PaymentJourneyResponse>();
  const pageTransitionsCache = createGenerationalCache<PageTransitionsResponse>();
  const visitorJourneyCache = createGenerationalCache<VisitorJourneyResponse>();
  const crawlerDetailCache = createGenerationalCache<CrawlerDetailResponse>();

  const details = {
    payments: (filters: PaymentsFilter, cursor?: string) => {
      const r = range();
      return filteredPaymentsCache.getOrFetch(filteredPaymentsKey(filters, cursor), () =>
        api.payments(r.start, r.end, filters, segments(), cursor),
      );
    },
    paymentJourney: (paymentId: number) =>
      paymentJourneyCache.getOrFetch(String(paymentId), () => api.paymentJourney(paymentId)),
    pageTransitions: (pathId: number, variant: PageTransitionsVariant) => {
      const r = range();
      return pageTransitionsCache.getOrFetch(`${variant}:${pathId}`, () =>
        api.pageTransitions(r.start, r.end, pathId, variant, segments()),
      );
    },
    /** Shares the Visitors tab's cache, since a filtered page is the same request. */
    filteredVisitors: (filter: VisitorSessionFilter, offset: number, sort: VisitorSortState) => {
      const r = range();
      return visitors.cache.getOrFetch(visitorsPageKey(offset, sort, filter), () =>
        api.visitors(r.start, r.end, offset, sort, filter, segments()),
      );
    },
    visitorJourney: (visitorTimestampId: string) =>
      visitorJourneyCache.getOrFetch(visitorTimestampId, () =>
        api.visitorJourney(visitorTimestampId),
      ),
    crawlerDetail: (useragentId: number, source: CrawlerSource, verified: boolean) => {
      const r = range();
      return crawlerDetailCache.getOrFetch(`${useragentId}:${source}:${verified ? 1 : 0}`, () =>
        api.crawlerDetail(r.start, r.end, useragentId, source, verified),
      );
    },
  };

  const pagedTabs = [searchTerms, audience, pages, landingPages, events, crawlers, visitors];
  const detailCaches = [
    filteredPaymentsCache,
    paymentJourneyCache,
    pageTransitionsCache,
    visitorJourneyCache,
    crawlerDetailCache,
  ];
  const resetRangeScoped = () => {
    overview.clear();
    revenueSources.clear();
    for (const tab of pagedTabs) {
      tab.reset();
    }
    conversions.reset();
    for (const cache of detailCaches) {
      cache.clear();
    }
  };

  createEffect<DateRangeSeconds | undefined>((previousRange) => {
    const currentRange = range();
    if (previousRange && !rangesEqual(previousRange, currentRange)) {
      resetRangeScoped();
    }
    return currentRange;
  });

  createEffect<string | undefined>((previousSegmentKey) => {
    const currentSegmentKey = segmentFiltersKey(segments());
    if (previousSegmentKey !== undefined && previousSegmentKey !== currentSegmentKey) {
      resetRangeScoped();
      paymentActivity.clear();
    }
    return currentSegmentKey;
  });

  createEffect<string | undefined>((previousFilterKey) => {
    const currentFilterKey = visitorSessionFilterKey(options.visitorsFilter());
    if (previousFilterKey !== undefined && previousFilterKey !== currentFilterKey) {
      visitors.changeOffset(0);
    }
    return currentFilterKey;
  });

  /**
   * Loads what the open tab shows, from cache when fresh. Call it from an
   * effect. `route` is read only by tabs with modes, so other tabs don't
   * refetch a failed load on every navigation.
   */
  const syncTab = (tab: DashboardTab, route: () => DashboardRoute) => {
    const currentRange = range();
    switch (tab) {
      case "overview":
        return overview.sync(currentRange);
      case "revenue":
        return paymentActivity.sync(currentRange);
      case "revenue_sources":
        return revenueSources.sync(currentRange);
      case "search_terms":
        return searchTerms.sync(currentRange);
      case "audience":
        return audience.sync(currentRange);
      case "pages":
        return (route().pagesMode === "landing" ? landingPages : pages).sync(currentRange);
      case "crawlers":
        return crawlers.sync(currentRange);
      case "events":
        return events.sync(currentRange);
      case "visitors":
        return (route().peopleMode === "customers" ? conversions : visitors).sync(currentRange);
    }
  };

  return {
    overview,
    revenueSources,
    paymentActivity,
    searchTerms,
    audience,
    pages,
    landingPages,
    events,
    crawlers,
    visitors,
    conversions,
    details,
    syncTab,
  };
}
