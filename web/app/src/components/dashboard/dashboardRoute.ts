import type { VisitorSessionFilter } from "../../lib/visitorsApi";
import { type DashboardTab, isDashboardTab } from "./dashboardTabs";
import type { PagesMode } from "./Pages";
import {
  campaignSourceKey,
  DIRECT_SOURCE_KEY,
  type RevenueSourceLinkTarget,
  referrerSourceKey,
} from "./revenueSourcesLogic";
import type { PeopleMode } from "./Visitors";

export type DashboardRoute = {
  tab: DashboardTab;
  peopleMode: PeopleMode;
  pagesMode: PagesMode;
  revenueSourceKey: string | null;
  pathId: number | null;
  landingPathId: number | null;
  countryCode: string | null;
  visitorFilter: VisitorSessionFilter | null;
};

const DEFAULT_ROUTE: DashboardRoute = {
  tab: "overview",
  peopleMode: "visitors",
  pagesMode: "pages",
  revenueSourceKey: null,
  pathId: null,
  landingPathId: null,
  countryCode: null,
  visitorFilter: null,
};

function parsePeopleMode(value: string | null): PeopleMode {
  if (value === "customers" || value === "conversions" || value === "new_customers") {
    return "customers";
  }
  return "visitors";
}

function parsePagesMode(
  value: string | null,
  landingPathId: number | null,
  pathId: number | null,
): PagesMode {
  if (value === "landing" || value === "landing_pages" || value === "landingPages") {
    return "landing";
  }
  if (value === "pages") {
    return "pages";
  }
  // Infer from drill-down params when mode is omitted.
  if (landingPathId !== null && pathId === null) {
    return "landing";
  }
  return "pages";
}

function parsePositiveInt(value: string | null): number | null {
  if (!value) {
    return null;
  }
  const parsed = Number(value);
  return Number.isInteger(parsed) && parsed > 0 ? parsed : null;
}

function parseCountryCode(value: string | null): string | null {
  if (!value) {
    return null;
  }
  const code = value.toUpperCase();
  return /^[A-Z]{2}$/.test(code) ? code : null;
}

function parseRevenueSourceKey(value: string | null): string | null {
  if (!value) {
    return null;
  }
  if (value === "direct") {
    return DIRECT_SOURCE_KEY;
  }
  const [kind, idText] = value.split(":", 2);
  const id = parsePositiveInt(idText ?? null);
  if (id === null) {
    return null;
  }
  if (kind === "campaign") {
    return campaignSourceKey(id);
  }
  if (kind === "referrer") {
    return referrerSourceKey(id);
  }
  return null;
}

function parseVisitorSessionFilter(params: URLSearchParams): VisitorSessionFilter | null {
  const filter = params.get("filter") ?? params.get("visitor_filter");
  if (!filter || filter === "all") {
    return null;
  }
  const pathId = parsePositiveInt(params.get("path_id"));
  if (filter === "pageview" || filter === "page_view") {
    return pathId === null ? null : { kind: "pageview", pathId };
  }
  if (filter === "landing_page" || filter === "landing") {
    return pathId === null ? null : { kind: "landing_page", pathId };
  }
  if (filter === "event") {
    const eventNameId = parsePositiveInt(params.get("event_name_id"));
    return pathId === null || eventNameId === null ? null : { kind: "event", eventNameId, pathId };
  }
  return null;
}

/** The tab a URL without a valid `tab` param opens, going by its drill-down params. */
function impliedTab(route: {
  countryCode: string | null;
  revenueSourceKey: string | null;
  visitorFilter: VisitorSessionFilter | null;
  pathId: number | null;
  landingPathId: number | null;
}): DashboardTab {
  if (route.countryCode) {
    return "audience";
  }
  if (route.revenueSourceKey) {
    return "revenue_sources";
  }
  if (route.visitorFilter) {
    return "visitors";
  }
  if (route.landingPathId || route.pathId) {
    return "pages";
  }
  return "overview";
}

export function readDashboardRoute(): DashboardRoute {
  if (typeof window === "undefined") {
    return DEFAULT_ROUTE;
  }
  const params = new URLSearchParams(window.location.search);
  const revenueSourceKey = parseRevenueSourceKey(params.get("source"));
  const pathId = parsePositiveInt(params.get("path_id"));
  const landingPathId = parsePositiveInt(params.get("landing_path_id"));
  const countryCode = parseCountryCode(params.get("cc"));
  const visitorFilter = parseVisitorSessionFilter(params);
  const requestedTab = params.get("tab");
  // Legacy ?tab=conversions bookmarks land on Visitors → New Customers.
  const legacyConversionsTab = requestedTab === "conversions";
  // Legacy ?tab=landing_pages bookmarks land on Pages → Landing Pages.
  const legacyLandingPagesTab = requestedTab === "landing_pages";
  const tab = legacyConversionsTab
    ? "visitors"
    : legacyLandingPagesTab
      ? "pages"
      : isDashboardTab(requestedTab)
        ? requestedTab
        : impliedTab({ countryCode, revenueSourceKey, visitorFilter, pathId, landingPathId });
  const peopleMode =
    tab === "visitors"
      ? legacyConversionsTab
        ? "customers"
        : parsePeopleMode(params.get("people"))
      : "visitors";
  const pagesMode =
    tab === "pages"
      ? legacyLandingPagesTab
        ? "landing"
        : parsePagesMode(params.get("pages"), landingPathId, pathId)
      : "pages";
  return {
    tab,
    peopleMode,
    pagesMode,
    revenueSourceKey: tab === "revenue_sources" ? revenueSourceKey : null,
    pathId: tab === "pages" && pagesMode === "pages" ? pathId : null,
    landingPathId: tab === "pages" && pagesMode === "landing" ? landingPathId : null,
    countryCode: tab === "audience" ? countryCode : null,
    visitorFilter: tab === "visitors" && peopleMode === "visitors" ? visitorFilter : null,
  };
}

/**
 * Rewrites a legacy ?tab=conversions / ?tab=landing_pages URL to the combined
 * tab that replaced it. Returns whether the URL changed.
 */
export function rewriteLegacyTabUrl(): boolean {
  if (typeof window === "undefined") {
    return false;
  }
  const url = new URL(window.location.href);
  const requestedTab = url.searchParams.get("tab");
  if (requestedTab === "conversions") {
    url.searchParams.set("tab", "visitors");
    url.searchParams.set("people", "customers");
  } else if (requestedTab === "landing_pages") {
    url.searchParams.set("tab", "pages");
    url.searchParams.set("pages", "landing");
  } else {
    return false;
  }
  window.history.replaceState(null, "", url);
  return true;
}

function deleteParams(params: URLSearchParams, names: readonly string[]) {
  for (const name of names) {
    params.delete(name);
  }
}

const TARGET_PARAMS = [
  "source",
  "path_id",
  "landing_path_id",
  "cc",
  "payment_id",
  "visitor_timestamp_id",
  "filter",
  "visitor_filter",
  "event_name_id",
  "people",
  "pages",
] as const;

export function clearDashboardTargetParams(params: URLSearchParams) {
  deleteParams(params, TARGET_PARAMS);
}

/** Drill-down params name rows of one project, so a project switch drops them. */
export function clearDashboardTargetUrlParams(): void {
  if (typeof window === "undefined") {
    return;
  }
  const url = new URL(window.location.href);
  clearDashboardTargetParams(url.searchParams);
  window.history.replaceState(null, "", url);
}

export function setRevenueSourceTargetParam(
  params: URLSearchParams,
  target: RevenueSourceLinkTarget,
) {
  switch (target.kind) {
    case "campaign":
      params.set("source", `campaign:${target.id}`);
      return;
    case "referrer":
      params.set("source", `referrer:${target.id}`);
      return;
    case "direct":
      params.set("source", "direct");
      return;
  }
}

/** Switches the Visitors tab between people modes, dropping the other mode's drill-downs. */
export function setPeopleModeParams(params: URLSearchParams, mode: PeopleMode) {
  params.set("tab", "visitors");
  if (mode === "customers") {
    params.set("people", "customers");
    deleteParams(params, [
      "visitor_timestamp_id",
      "filter",
      "visitor_filter",
      "event_name_id",
      "path_id",
    ]);
  } else {
    deleteParams(params, ["people", "payment_id"]);
  }
}

/** Switches the Pages tab between modes, dropping the other mode's drill-downs. */
export function setPagesModeParams(params: URLSearchParams, mode: PagesMode) {
  params.set("tab", "pages");
  if (mode === "landing") {
    params.set("pages", "landing");
    deleteParams(params, ["path_id", "payment_id", "visitor_timestamp_id"]);
  } else {
    deleteParams(params, ["pages", "landing_path_id", "payment_id", "visitor_timestamp_id"]);
  }
}
