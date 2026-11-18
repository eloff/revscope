import { appPath } from "@revscope/web-shared/links";
import {
  dashboardTabAdminIcon,
  dashboardTabAskIcon,
  dashboardTabAudienceIcon,
  dashboardTabCampaignsIcon,
  dashboardTabCrawlersIcon,
  dashboardTabEventsIcon,
  dashboardTabFunnelsIcon,
  dashboardTabOverviewIcon,
  dashboardTabPagesIcon,
  dashboardTabProjectsIcon,
  dashboardTabRevenueIcon,
  dashboardTabRevenueSourcesIcon,
  dashboardTabSearchTermsIcon,
  dashboardTabVisitorsIcon,
} from "./icons.css";

export type DashboardTab =
  | "projects"
  | "overview"
  | "revenue"
  | "revenue_sources"
  | "campaigns"
  | "search_terms"
  | "audience"
  | "pages"
  | "visitors"
  | "crawlers"
  | "events"
  | "funnels"
  | "ask"
  | "admin";

export const DASHBOARD_TABS = new Set<DashboardTab>([
  "projects",
  "overview",
  "revenue",
  "revenue_sources",
  "campaigns",
  "search_terms",
  "audience",
  "pages",
  "visitors",
  "crawlers",
  "events",
  "funnels",
  "ask",
  "admin",
]);

export const DASHBOARD_TAB_ITEMS: {
  value: DashboardTab;
  label: string;
  iconClass: string;
}[] = [
  {
    value: "projects",
    label: "Projects",
    iconClass: dashboardTabProjectsIcon,
  },
  {
    value: "overview",
    label: "Overview",
    iconClass: dashboardTabOverviewIcon,
  },
  {
    value: "revenue",
    label: "Revenue",
    iconClass: dashboardTabRevenueIcon,
  },
  {
    value: "revenue_sources",
    label: "Sources",
    iconClass: dashboardTabRevenueSourcesIcon,
  },
  {
    value: "campaigns",
    label: "Campaigns",
    iconClass: dashboardTabCampaignsIcon,
  },
  {
    value: "search_terms",
    label: "Search terms",
    iconClass: dashboardTabSearchTermsIcon,
  },
  {
    value: "audience",
    label: "Audience",
    iconClass: dashboardTabAudienceIcon,
  },
  {
    value: "pages",
    label: "Pages",
    iconClass: dashboardTabPagesIcon,
  },
  {
    value: "visitors",
    label: "Visitors",
    iconClass: dashboardTabVisitorsIcon,
  },
  {
    value: "crawlers",
    label: "Crawlers",
    iconClass: dashboardTabCrawlersIcon,
  },
  {
    value: "events",
    label: "Events",
    iconClass: dashboardTabEventsIcon,
  },
  {
    value: "funnels",
    label: "Funnels",
    iconClass: dashboardTabFunnelsIcon,
  },
  {
    value: "ask",
    label: "Ask AI",
    iconClass: dashboardTabAskIcon,
  },
  {
    value: "admin",
    label: "Admin",
    iconClass: dashboardTabAdminIcon,
  },
];

/** Tabs about the signed-in account rather than one project. */
const ACCOUNT_TABS = new Set<DashboardTab>(["projects", "admin"]);

/** The demo is a single project with no account behind it, so it has neither account tab. */
export const DEMO_TAB_ITEMS = DASHBOARD_TAB_ITEMS.filter((item) => !ACCOUNT_TABS.has(item.value));

export function isDashboardTab(value: string | null): value is DashboardTab {
  return value !== null && DASHBOARD_TABS.has(value as DashboardTab);
}

export function demoTabHref(tab: DashboardTab): string {
  const demo = appPath("/demo");
  return tab === "overview" ? demo : `${demo}?tab=${tab}`;
}
