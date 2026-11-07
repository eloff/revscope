/// <reference types="solid-js" />
import { Tabs } from "@ark-ui/solid/tabs";
import { type AuthSession, useAuthState } from "@revscope/web-shared/authState";
import {
  isDemoProjectId,
  type ProjectOption,
  planAllowsMultipleProjects,
  projectTimeZone,
} from "@revscope/web-shared/projectSelection";
import { createEffect, createSignal, For, onMount, Show } from "solid-js";
import { fetchChatAvailable } from "../../lib/chatApi";
import { type DateRangeSeconds, periodInTimeZone } from "../../lib/dateRange";
import Campaigns from "./Campaigns";
import AiChat from "./chat/AiChat";
import DashboardFilters from "./DashboardFilters";
import {
  embeddedShell,
  fullscreenShell,
  page,
  tabContent,
  tabContentStack,
  tabIcon,
  tabList,
  tabsRoot,
  tabTrigger,
} from "./DashboardShell.css";
import {
  AudiencePanel,
  CrawlersPanel,
  EventsPanel,
  OverviewPanel,
  PagesPanel,
  RevenuePanel,
  RevenueSourcesPanel,
  SearchTermsPanel,
  VisitorsPanel,
} from "./DashboardTabPanels";
import { createDashboardData } from "./dashboardData";
import { createDashboardNavigation } from "./dashboardNavigation";
import { useDashboardState } from "./dashboardPeriodContext";
import { createDashboardProjectApi } from "./dashboardProjectApi";
import { DASHBOARD_TAB_ITEMS, type DashboardTab } from "./dashboardTabs";
import Funnels from "./Funnels";
import { ImportedCoverageProvider, ImportedDataNotice } from "./ImportedCoverageContext";
import { message } from "./messages.css";
import type { ProjectOverviewCache } from "./ProjectOverviewCard";
import ProjectsOverview from "./ProjectsOverview";

export type DashboardLayout = "page" | "embedded" | "fullscreen";

export type DashboardShellProps = {
  projectId: string;
  /** Projects shown in the account menu for switching. */
  projects?: ProjectOption[];
  /** `range` is the period to open the project on, when the viewer chose one. */
  onProjectChange?: (projectId: string, range?: DateRangeSeconds) => void;
  /** Period to open on instead of the default; see `onProjectChange`. */
  initialRange?: DateRangeSeconds;
  /** Outlives the shell, which remounts on every project switch. */
  projectOverviewCache?: ProjectOverviewCache;
  publicAccess?: boolean;
  layout?: DashboardLayout;
  showHeader?: boolean;
};

const SHELL_CLASS: Record<DashboardLayout, string> = {
  page,
  embedded: embeddedShell,
  fullscreen: fullscreenShell,
};

/** The dashboard without its own period state, for callers that already provide one. */
export default function DashboardShellContent(props: DashboardShellProps) {
  const layout = () => props.layout ?? "page";
  const { authUser, authResolved } = useAuthState();
  const { range, rangeReady, timeZone, segments } = useDashboardState();
  const [chatAvailable, setChatAvailable] = createSignal(false);

  const authReady = () => props.publicAccess || authResolved();
  const canLoad = () => props.publicAccess || authUser();

  // Asking costs money, so the tab needs a configured AI gateway. Own projects
  // also need a signed-in viewer; the public demo is open to anonymous visitors
  // (spend is capped per client IP).
  const canAsk = () => chatAvailable() && (Boolean(authUser()) || isDemoProjectId(props.projectId));
  // Shown to accounts that hold, or may add, more than one project. One left
  // over from a downgrade still lists so the rest stay reachable.
  const canShowProjects = () => {
    const user = authUser();
    return (
      Boolean(user) &&
      !props.publicAccess &&
      (planAllowsMultipleProjects(user?.plan ?? "") || (props.projects?.length ?? 0) > 1)
    );
  };
  const visibleTabItems = () =>
    DASHBOARD_TAB_ITEMS.filter(
      (item) =>
        (item.value !== "ask" || canAsk()) && (item.value !== "projects" || canShowProjects()),
    );

  onMount(() => {
    void fetchChatAvailable().then(setChatAvailable);
  });

  const nav = createDashboardNavigation({
    allowedTab: (tab) => (tab === "projects" && !canShowProjects() ? "overview" : tab),
  });

  const data = createDashboardData({
    api: createDashboardProjectApi({
      projectId: () => props.projectId,
      publicAccess: () => Boolean(props.publicAccess),
      session: (): AuthSession => {
        const user = authUser();
        if (!user) {
          throw new Error("Not authenticated");
        }
        return user;
      },
    }),
    range,
    segments,
    visitorsFilter: nav.visitorsFilter,
  });

  createEffect(() => {
    if (nav.tabsReady() && canLoad() && rangeReady()) {
      data.syncTab(nav.activeTab(), nav.route);
    }
  });

  // The card showed this period on the opened project's own days, so its
  // Overview opens on the same one rather than the default.
  const openProject = (projectId: string) => {
    nav.navigate("overview");
    if (projectId !== props.projectId) {
      const target = projectTimeZone(props.projects, projectId);
      props.onProjectChange?.(projectId, periodInTimeZone(range(), timeZone(), target));
    }
  };
  const fallbackProjectOverviewCache: ProjectOverviewCache = new Map();

  return (
    <div class={SHELL_CLASS[layout()]}>
      <Show when={authReady()} fallback={<p class={message}>Loading analytics…</p>}>
        <Show when={canLoad()} fallback={<p class={message}>Log in to view analytics.</p>}>
          <DashboardFilters
            showBrand={layout() !== "embedded"}
            showAccount={layout() !== "embedded"}
            projects={props.projects}
            projectId={props.projectId}
            onProjectChange={(projectId) => props.onProjectChange?.(projectId)}
            dateRangeDisabled={nav.activeTab() === "revenue"}
          />

          <Show when={nav.tabsReady()}>
            <ImportedCoverageProvider
              projectId={props.projectId}
              publicAccess={Boolean(props.publicAccess)}
            >
              <Tabs.Root
                value={nav.activeTab()}
                onValueChange={(details) => nav.navigate(details.value as DashboardTab)}
                class={tabsRoot}
                lazyMount
                unmountOnExit
              >
                <Tabs.List class={tabList}>
                  <For each={visibleTabItems()}>
                    {(item) => (
                      <Tabs.Trigger
                        value={item.value}
                        class={tabTrigger}
                        data-dashboard-tab={item.value}
                      >
                        <span class={`${item.iconClass} ${tabIcon}`} aria-hidden="true" />
                        <span>{item.label}</span>
                      </Tabs.Trigger>
                    )}
                  </For>
                </Tabs.List>
                <ImportedDataNotice tab={nav.activeTab()} />
                <Show when={canShowProjects() && authUser()}>
                  {(user) => (
                    <Tabs.Content value="projects" class={tabContent}>
                      <ProjectsOverview
                        session={user()}
                        projects={props.projects ?? []}
                        currentProjectId={props.projectId}
                        cache={props.projectOverviewCache ?? fallbackProjectOverviewCache}
                        onOpenProject={openProject}
                      />
                    </Tabs.Content>
                  )}
                </Show>
                <Tabs.Content value="overview" class={tabContent}>
                  <OverviewPanel data={data} nav={nav} />
                </Tabs.Content>
                <Tabs.Content value="revenue" class={`${tabContent} ${tabContentStack}`}>
                  <RevenuePanel data={data} nav={nav} />
                </Tabs.Content>
                <Tabs.Content value="campaigns" class={tabContent}>
                  <Campaigns
                    projectId={props.projectId}
                    publicAccess={props.publicAccess}
                    session={authUser() ?? undefined}
                  />
                </Tabs.Content>
                <Tabs.Content value="revenue_sources" class={tabContent}>
                  <RevenueSourcesPanel data={data} nav={nav} />
                </Tabs.Content>
                <Tabs.Content value="search_terms" class={tabContent}>
                  <SearchTermsPanel data={data} nav={nav} />
                </Tabs.Content>
                <Tabs.Content value="audience" class={tabContent}>
                  <AudiencePanel data={data} nav={nav} />
                </Tabs.Content>
                <Tabs.Content value="pages" class={tabContent}>
                  <PagesPanel data={data} nav={nav} />
                </Tabs.Content>
                <Tabs.Content value="visitors" class={tabContent}>
                  <VisitorsPanel data={data} nav={nav} />
                </Tabs.Content>
                <Tabs.Content value="crawlers" class={tabContent}>
                  <CrawlersPanel data={data} nav={nav} />
                </Tabs.Content>
                <Tabs.Content value="events" class={tabContent}>
                  <EventsPanel data={data} nav={nav} />
                </Tabs.Content>
                <Tabs.Content value="funnels" class={tabContent}>
                  <Funnels
                    projectId={props.projectId}
                    publicAccess={props.publicAccess}
                    session={authUser() ?? undefined}
                  />
                </Tabs.Content>
                <Tabs.Content value="ask" class={tabContent}>
                  <Show when={canAsk()}>
                    <AiChat projectId={props.projectId} session={authUser() ?? undefined} />
                  </Show>
                </Tabs.Content>
              </Tabs.Root>
            </ImportedCoverageProvider>
          </Show>
        </Show>
      </Show>
    </div>
  );
}
