import { createEffect, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { writeHistoryUrl } from "../../lib/embedHistory";
import { setVisitorFilterParams, type VisitorSessionFilter } from "../../lib/visitorsApi";
import {
  clearDashboardTargetParams,
  type DashboardRoute,
  readDashboardRoute,
  rewriteLegacyTabUrl,
  setPagesModeParams,
  setPeopleModeParams,
  setRevenueSourceTargetParam,
} from "./dashboardRoute";
import type { DashboardTab } from "./dashboardTabs";
import type { PagesMode } from "./Pages";
import type { RevenueSourceLinkTarget } from "./revenueSourcesLogic";
import type { PeopleMode } from "./Visitors";

export type DashboardNavigation = ReturnType<typeof createDashboardNavigation>;

/**
 * The dashboard's URL-backed state: which tab is open and which row a link
 * drilled into. Every navigation pushes a history entry, so Back retraces it
 * (except in an embed; see writeHistoryUrl).
 */
export function createDashboardNavigation(options: {
  /** Maps a requested tab to one the viewer may open. */
  allowedTab: (tab: DashboardTab) => DashboardTab;
}) {
  const [routeRevision, setRouteRevision] = createSignal(0);
  const [tabsReady, setTabsReady] = createSignal(false);
  const [activeTab, setActiveTab] = createSignal<DashboardTab>("overview");
  const routeChanged = () => setRouteRevision((revision) => revision + 1);

  const route = createMemo(() => {
    routeRevision();
    return readDashboardRoute();
  });

  const writeUrl = (configure: (params: URLSearchParams) => void) => {
    if (typeof window === "undefined") {
      return;
    }
    const url = new URL(window.location.href);
    configure(url.searchParams);
    writeHistoryUrl(url);
    routeChanged();
  };

  const navigate = (tab: DashboardTab, configure?: (params: URLSearchParams) => void) => {
    setActiveTab(tab);
    writeUrl((params) => {
      params.set("tab", tab);
      clearDashboardTargetParams(params);
      configure?.(params);
    });
  };

  /** A target a tab scrolls to; `id` changes on every navigation so re-clicking a link scrolls again. */
  const scrollTarget = <K extends string, V>(name: K, pick: (route: DashboardRoute) => V | null) =>
    createMemo(() => {
      const value = pick(route());
      return value === null
        ? null
        : ({ [name]: value, id: routeRevision() } as Record<K, V> & {
            id: number;
          });
    });

  createEffect(() => {
    if (!tabsReady()) {
      return;
    }
    const tab = options.allowedTab(route().tab);
    if (activeTab() !== tab) {
      setActiveTab(tab);
    }
  });

  onMount(() => {
    setActiveTab(options.allowedTab(readDashboardRoute().tab));
    setTabsReady(true);
    if (rewriteLegacyTabUrl()) {
      routeChanged();
    }
    window.addEventListener("popstate", routeChanged);
    onCleanup(() => window.removeEventListener("popstate", routeChanged));
  });

  return {
    route,
    activeTab,
    tabsReady,
    navigate,
    toPagePath: (pathId: number | null) =>
      navigate("pages", (params) => {
        if (pathId !== null) {
          params.set("path_id", String(pathId));
        }
      }),
    toLandingPath: (pathId: number | null) =>
      navigate("pages", (params) => {
        params.set("pages", "landing");
        if (pathId !== null) {
          params.set("landing_path_id", String(pathId));
        }
      }),
    toRevenueSource: (target: RevenueSourceLinkTarget | null) =>
      navigate("revenue_sources", (params) => {
        if (target) {
          setRevenueSourceTargetParam(params, target);
        }
      }),
    toCountry: (countryCode: string) =>
      navigate("audience", (params) => {
        params.set("cc", countryCode.toUpperCase());
      }),
    toVisitors: (filter: VisitorSessionFilter | null = null) =>
      navigate("visitors", (params) => {
        setVisitorFilterParams(params, filter);
      }),
    setPeopleMode: (mode: PeopleMode) => writeUrl((params) => setPeopleModeParams(params, mode)),
    setPagesMode: (mode: PagesMode) => writeUrl((params) => setPagesModeParams(params, mode)),
    // readDashboardRoute() nulls each drill-down outside its own tab and mode.
    visitorsFilter: createMemo(() => route().visitorFilter),
    revenueSourceScrollTarget: scrollTarget("key", (r) => r.revenueSourceKey),
    pathScrollTarget: scrollTarget("pathId", (r) => r.pathId),
    landingPathScrollTarget: scrollTarget("pathId", (r) => r.landingPathId),
    countryScrollTarget: scrollTarget("key", (r) => r.countryCode),
  };
}
