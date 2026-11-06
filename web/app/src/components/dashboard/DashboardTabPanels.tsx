/// <reference types="solid-js" />
import { createMemo, Show } from "solid-js";
import type { CrawlersSortState } from "../../lib/crawlersApi";
import { deriveMetrics } from "../../lib/dashboardApi";
import {
  crawlersSort,
  eventsSort,
  searchTermsSort,
  setCrawlersSort,
  setEventsSort,
  setSearchTermsSort,
  setVisitorsSort,
  visitorsSort,
} from "../../lib/dashboardPreferences";
import type { EventsSortState } from "../../lib/eventsApi";
import type { SearchTermsSortState } from "../../lib/searchTermsApi";
import {
  eventFilterKey,
  hasSegmentFilters,
  landingPageFilterKey,
  pageFilterKey,
  sourceFilterKey,
} from "../../lib/segmentFilters";
import { describeVisitorSessionFilter, type VisitorSortState } from "../../lib/visitorsApi";
import type { ActiveFilterChip } from "./ActiveFilters";
import ActivityGraph from "./ActivityGraph";
import Audience from "./Audience";
import Crawlers from "./Crawlers";
import type { LoadState, OffsetTabData } from "./createTabData";
import type { DashboardData } from "./dashboardData";
import type { DashboardNavigation } from "./dashboardNavigation";
import { useDashboardState } from "./dashboardPeriodContext";
import Events from "./Events";
import NoDataView from "./NoDataView";
import OverviewTabContent from "./OverviewTabContent";
import Pages from "./Pages";
import RevenueSources from "./RevenueSources";
import RunwayCalculator from "./RunwayCalculator";
import SearchTerms from "./SearchTerms";
import {
  hasAudienceData,
  hasConversionsData,
  hasCrawlersData,
  hasEventsData,
  hasOverviewData,
  hasPagesData,
  hasPaymentActivityData,
  hasRevenueSourcesData,
  hasSearchTermsData,
  hasVisitorsData,
} from "./tabDataPresence";
import Visitors from "./Visitors";

/**
 * The shell owns each tab's data rather than the panel, because tabs unmount
 * when the viewer leaves them and the cache must outlive that.
 */
type TabPanelProps = {
  data: DashboardData;
  nav: DashboardNavigation;
};

/**
 * Whether a tab should show the setup prompt for a project with nothing
 * tracked. Narrowing filters get the tables' own "no rows" copy instead, since
 * the prompt would be misleading for a project that does have data.
 */
function useShowsNoData() {
  const { segments } = useDashboardState();
  return <T,>(state: LoadState<T>, hasData: (data: T) => boolean) =>
    !hasSegmentFilters(segments()) && state.settledEmpty(hasData);
}

/** A segment filter added from a row switches to Overview so its impact is visible. */
function thenOverview<A extends unknown[]>(nav: DashboardNavigation, add: (...args: A) => void) {
  return (...args: A) => {
    add(...args);
    nav.navigate("overview");
  };
}

/** Sorting starts over from the first page. */
function sortHandler<S>(
  tab: Pick<OffsetTabData<unknown>, "changeOffset">,
  setSort: (sort: S) => void,
) {
  return (sort: S) => {
    setSort(sort);
    tab.changeOffset(0);
  };
}

export function OverviewPanel(props: TabPanelProps) {
  const overview = () => props.data.overview;
  const showsNoData = useShowsNoData();
  const metrics = createMemo(() => {
    const data = overview().data();
    return data ? deriveMetrics(data) : undefined;
  });
  return (
    <OverviewTabContent
      data={overview().data()}
      metrics={metrics()}
      loading={overview().loading()}
      error={overview().error()}
      empty={showsNoData(overview(), hasOverviewData)}
    />
  );
}

export function RevenuePanel(props: TabPanelProps) {
  const activity = () => props.data.paymentActivity;
  const showsNoData = useShowsNoData();
  const loading = () => activity().loading() && !activity().data();
  return (
    <Show
      when={!showsNoData(activity(), hasPaymentActivityData)}
      fallback={<NoDataView variant="payment-activity" />}
    >
      <ActivityGraph data={activity().data()} loading={loading()} error={activity().error()} />
      <RunwayCalculator data={activity().data()} loading={loading()} error={activity().error()} />
    </Show>
  );
}

export function RevenueSourcesPanel(props: TabPanelProps) {
  const sources = () => props.data.revenueSources;
  const showsNoData = useShowsNoData();
  const { segments, addSourceFilter } = useDashboardState();
  const activeSourceFilterKeys = createMemo(() => new Set(segments().sources.map(sourceFilterKey)));
  return (
    <Show when={!showsNoData(sources(), hasRevenueSourcesData)} fallback={<NoDataView />}>
      <RevenueSources
        data={sources().data()}
        loading={sources().loading()}
        error={sources().error()}
        scrollTarget={props.nav.revenueSourceScrollTarget()}
        onLandingPathSelect={props.nav.toLandingPath}
        fetchPayments={props.data.details.payments}
        fetchJourney={props.data.details.paymentJourney}
        onRevenueSourceSelect={props.nav.toRevenueSource}
        onSourceFilter={thenOverview(props.nav, addSourceFilter)}
        activeSourceFilterKeys={activeSourceFilterKeys()}
      />
    </Show>
  );
}

export function SearchTermsPanel(props: TabPanelProps) {
  const terms = () => props.data.searchTerms;
  const showsNoData = useShowsNoData();
  return (
    <Show when={!showsNoData(terms().table, hasSearchTermsData)} fallback={<NoDataView />}>
      <SearchTerms
        data={terms().table.data()}
        treemapData={terms().treemap.data()}
        loading={terms().table.loading()}
        treemapLoading={terms().treemap.loading()}
        error={terms().table.error()}
        sort={searchTermsSort()}
        onSortChange={sortHandler<SearchTermsSortState>(terms(), setSearchTermsSort)}
        onPageChange={terms().changeOffset}
        onLandingPathSelect={props.nav.toLandingPath}
      />
    </Show>
  );
}

export function AudiencePanel(props: TabPanelProps) {
  const audience = () => props.data.audience;
  const showsNoData = useShowsNoData();
  const { addCountryFilter, addAttributeFilter } = useDashboardState();
  // The countries stay up while a new cities page loads, so only the cities
  // table shows it is loading.
  const citiesLoading = () =>
    audience().table.loading() &&
    audience().table.data() !== undefined &&
    audience().table.loadedKey() !== audience().currentKey();
  return (
    <Show when={!showsNoData(audience().table, hasAudienceData)} fallback={<NoDataView />}>
      <Audience
        data={audience().table.data()}
        treemapData={audience().treemap.data()}
        loading={audience().table.loading()}
        treemapLoading={audience().treemap.loading()}
        citiesLoading={citiesLoading()}
        citiesOffset={audience().offset()}
        error={audience().table.error()}
        scrollTarget={props.nav.countryScrollTarget()}
        onCountrySelect={props.nav.toCountry}
        onCountryFilter={thenOverview(props.nav, addCountryFilter)}
        onAttributeFilter={thenOverview(props.nav, addAttributeFilter)}
        onCitiesPageChange={audience().changeOffset}
        onCitiesSortChange={() => audience().changeOffset(0)}
      />
    </Show>
  );
}

export function PagesPanel(props: TabPanelProps) {
  const pages = () => props.data.pages;
  const landing = () => props.data.landingPages;
  const details = () => props.data.details;
  const showsNoData = useShowsNoData();
  const { segments, addPageFilter, addLandingPageFilter } = useDashboardState();
  const activePageFilterKeys = createMemo(() => new Set(segments().pages.map(pageFilterKey)));
  const activeLandingPageFilterKeys = createMemo(
    () => new Set(segments().landingPages.map(landingPageFilterKey)),
  );
  return (
    <Pages
      pagesMode={props.nav.route().pagesMode}
      onPagesModeChange={props.nav.setPagesMode}
      pagesEmpty={showsNoData(pages().table, hasPagesData)}
      landingPagesEmpty={showsNoData(landing().table, hasPagesData)}
      data={pages().table.data()}
      treemapData={pages().treemap.data()}
      loading={pages().table.loading()}
      treemapLoading={pages().treemap.loading()}
      error={pages().table.error()}
      scrollTarget={props.nav.pathScrollTarget()}
      onPathSelect={props.nav.toPagePath}
      landingData={landing().table.data()}
      landingTreemapData={landing().treemap.data()}
      landingLoading={landing().table.loading()}
      landingTreemapLoading={landing().treemap.loading()}
      landingError={landing().table.error()}
      landingScrollTarget={props.nav.landingPathScrollTarget()}
      onLandingPathSelect={props.nav.toLandingPath}
      onTransitionPathSelect={props.nav.toPagePath}
      onPagesPageChange={pages().changeOffset}
      onLandingPagesPageChange={landing().changeOffset}
      onPagesSortChange={() => pages().changeOffset(0)}
      onLandingPagesSortChange={() => landing().changeOffset(0)}
      fetchTransitions={details().pageTransitions}
      fetchPayments={details().payments}
      fetchVisitors={details().filteredVisitors}
      fetchJourney={details().paymentJourney}
      fetchVisitorJourney={details().visitorJourney}
      onRevenueSourceSelect={props.nav.toRevenueSource}
      onVisitorPathSelect={props.nav.toPagePath}
      onVisitorLandingPathSelect={props.nav.toLandingPath}
      onCountrySelect={props.nav.toCountry}
      onPageFilter={thenOverview(props.nav, (id: number, name: string) =>
        addPageFilter({ id, name }),
      )}
      activePageFilterKeys={activePageFilterKeys()}
      onLandingPageFilter={thenOverview(props.nav, (id: number, name: string) =>
        addLandingPageFilter({ id, name }),
      )}
      activeLandingPageFilterKeys={activeLandingPageFilterKeys()}
    />
  );
}

export function VisitorsPanel(props: TabPanelProps) {
  const visitors = () => props.data.visitors;
  const conversions = () => props.data.conversions;
  const showsNoData = useShowsNoData();
  const filter = () => props.nav.visitorsFilter();
  const filterChips = createMemo<ActiveFilterChip[]>(() => {
    const current = filter();
    if (!current) {
      return [];
    }
    return [
      {
        label: describeVisitorSessionFilter(current, visitors().table.data()?.filter),
        onRemove: () => props.nav.toVisitors(null),
      },
    ];
  });
  return (
    <Visitors
      peopleMode={props.nav.route().peopleMode}
      onPeopleModeChange={props.nav.setPeopleMode}
      filterChips={filterChips()}
      visitorsEmpty={!filter() && showsNoData(visitors().table, hasVisitorsData)}
      conversionsEmpty={showsNoData(conversions().table, hasConversionsData)}
      data={visitors().table.data()}
      loading={visitors().table.loading()}
      error={visitors().table.error()}
      sort={visitorsSort()}
      onSortChange={sortHandler<VisitorSortState>(visitors(), setVisitorsSort)}
      onPageChange={visitors().changeOffset}
      fetchJourney={props.data.details.visitorJourney}
      conversionsData={conversions().table.data()}
      conversionsLoading={conversions().table.loading()}
      conversionsError={conversions().table.error()}
      conversionsOffset={conversions().offset()}
      onConversionsPageChange={conversions().changePage}
      fetchPaymentJourney={props.data.details.paymentJourney}
      onRevenueSourceSelect={props.nav.toRevenueSource}
      onPathSelect={props.nav.toPagePath}
      onLandingPathSelect={props.nav.toLandingPath}
      onCountrySelect={props.nav.toCountry}
      emptyLabel={filter() ? "No visitors match this filter" : "No visitors for this period"}
    />
  );
}

export function CrawlersPanel(props: TabPanelProps) {
  const crawlers = () => props.data.crawlers;
  const showsNoData = useShowsNoData();
  return (
    <Show
      when={!showsNoData(crawlers().table, hasCrawlersData)}
      fallback={<NoDataView variant="crawlers" />}
    >
      <Crawlers
        data={crawlers().table.data()}
        loading={crawlers().table.loading()}
        error={crawlers().table.error()}
        sort={crawlersSort()}
        onSortChange={sortHandler<CrawlersSortState>(crawlers(), setCrawlersSort)}
        onPageChange={crawlers().changeOffset}
        onFilterChange={() => crawlers().changeOffset(0)}
        fetchDetail={props.data.details.crawlerDetail}
      />
    </Show>
  );
}

export function EventsPanel(props: TabPanelProps) {
  const events = () => props.data.events;
  const showsNoData = useShowsNoData();
  const { segments, addEventFilter } = useDashboardState();
  const activeEventFilterKeys = createMemo(() => new Set(segments().events.map(eventFilterKey)));
  return (
    <Show when={!showsNoData(events().table, hasEventsData)} fallback={<NoDataView />}>
      <Events
        data={events().table.data()}
        loading={events().table.loading()}
        error={events().table.error()}
        sort={eventsSort()}
        onSortChange={sortHandler<EventsSortState>(events(), setEventsSort)}
        onPageChange={events().changeOffset}
        onPathSelect={props.nav.toPagePath}
        onVisitorsSelect={props.nav.toVisitors}
        onEventFilter={thenOverview(props.nav, (id: number, name: string) =>
          addEventFilter({ id, name }),
        )}
        activeEventFilterKeys={activeEventFilterKeys()}
      />
    </Show>
  );
}
