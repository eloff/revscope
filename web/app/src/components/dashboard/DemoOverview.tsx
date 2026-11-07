/// <reference types="solid-js" />
/**
 * Homepage island: overview metrics/chart plus tab links. Kept separate from
 * `Demo` so the landing page does not download the rest of the dashboard until
 * a visitor expands the embed (see demoEmbedTabs.ts).
 */
import { appPath } from "@revscope/web-shared/links";
import { createEffect, createMemo, createSignal, For, Show } from "solid-js";
import { Dynamic } from "solid-js/web";
import {
  DEMO_PROJECT_ID,
  deriveMetrics,
  fetchOverview,
  type OverviewResponse,
} from "../../lib/dashboardApi";
import { type CacheEntry, isCacheEntryStale, rangesEqual } from "../../lib/dashboardDataCache";
import type { DateRangeSeconds } from "../../lib/dateRange";
import { createEmbedExpansion } from "../../lib/embedExpansion";
import { isPlainLeftClick } from "../../lib/plainClick";
import { hasSegmentFilters, segmentFiltersKey } from "../../lib/segmentFilters";
import DashboardFilters from "./DashboardFilters";
import {
  embeddedShell,
  tabContent,
  tabIcon,
  tabLink,
  tabList,
  tabsRoot,
  tabTrigger,
} from "./DashboardShell.css";
import {
  closeButton,
  closeIcon,
  embed,
  expandHint,
  expandOverlay,
  fullscreenIcon,
  fullscreenLink,
} from "./Demo.css";
import { DashboardStateProvider, useDashboardState } from "./dashboardPeriodContext";
import { type DashboardTab, DEMO_TAB_ITEMS, demoTabHref } from "./dashboardTabs";
import { createDemoEmbedTabs } from "./demoEmbedTabs";
import { message } from "./messages.css";
import OverviewTabContent from "./OverviewTabContent";
import { hasOverviewData } from "./tabDataPresence";
import { visuallyHidden } from "./visuallyHidden.css";

/** Must match the `embed` the home page's EmbedFrame uses. */
export const DEMO_EMBED_ID = "demo";

export default function DemoOverview() {
  const expansion = createEmbedExpansion(DEMO_EMBED_ID);
  const collapsed = () => expansion.enabled() && !expansion.expanded();
  const tabs = createDemoEmbedTabs(expansion.expanded);
  return (
    <div class={embed}>
      <Show when={collapsed()}>
        <button type="button" class={expandOverlay} onClick={expansion.expand}>
          <span class={expandHint}>Click to explore the demo</span>
        </button>
      </Show>
      <Show when={expansion.expanded()}>
        <button
          type="button"
          ref={(button) => queueMicrotask(() => button.focus())}
          class={closeButton}
          onClick={expansion.collapse}
          aria-label="Close demo"
          title="Close (Esc)"
        >
          <svg
            class={closeIcon}
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            aria-hidden="true"
          >
            <path d="M6 6l12 12" />
            <path d="M18 6L6 18" />
          </svg>
        </button>
      </Show>
      <div inert={collapsed()}>
        <Show when={!expansion.expanded()}>
          <a href={appPath("/demo")} class={fullscreenLink} title="Full screen">
            <span class={visuallyHidden}>Open demo in full screen</span>
            <svg
              class={fullscreenIcon}
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              <path d="M8 3H5a2 2 0 0 0-2 2v3" />
              <path d="M21 8V5a2 2 0 0 0-2-2h-3" />
              <path d="M3 16v3a2 2 0 0 0 2 2h3" />
              <path d="M16 21h3a2 2 0 0 0 2-2v-3" />
            </svg>
          </a>
        </Show>
        {/* Shared by the overview and the full dashboard, so the period and filters carry over. */}
        <DashboardStateProvider>
          {/* Stays behind the dashboard, so collapsing keeps its data without a new request and its reserved height is right for the host page to collapse to. The tab's content, chart included, is rebuilt after a tab click, so the chart draws once it's revealed. */}
          <div hidden={Boolean(tabs.shell())}>
            <DemoOverviewContent
              concealed={Boolean(tabs.shell())}
              pendingTab={tabs.pendingTab()}
              failed={tabs.failed()}
              openInPlace={expansion.expanded()}
              onTabIntent={tabs.prefetch}
              onTabOpen={tabs.open}
            />
          </div>
          <Show when={tabs.shell()}>
            {(shell) => (
              <Dynamic
                component={shell()}
                projectId={DEMO_PROJECT_ID}
                publicAccess
                layout="embedded"
              />
            )}
          </Show>
        </DashboardStateProvider>
      </div>
    </div>
  );
}

type DemoOverviewContentProps = {
  /**
   * Hidden behind the full dashboard. The filter bar is unmounted meanwhile: it
   * labels its date preset once on mount, so it remounts to match whatever
   * period the dashboard left behind.
   */
  concealed: boolean;
  /** A tab clicked while the full dashboard loads; shown selected in place of the overview. */
  pendingTab: DashboardTab | null;
  /** The pending tab couldn't be downloaded. */
  failed: boolean;
  /** Whether tab clicks stay in the embed rather than following their link to `/demo`. */
  openInPlace: boolean;
  onTabIntent: () => void;
  onTabOpen: (tab: DashboardTab) => void;
};

function DemoOverviewContent(props: DemoOverviewContentProps) {
  const { range, rangeReady, segments } = useDashboardState();
  const [overviewCache, setOverviewCache] = createSignal<
    CacheEntry<OverviewResponse> | undefined
  >();
  const [overviewLoading, setOverviewLoading] = createSignal(false);
  const [overviewError, setOverviewError] = createSignal("");
  const overviewData = createMemo(() => overviewCache()?.data);
  const metrics = createMemo(() => {
    const data = overviewData();
    return data ? deriveMetrics(data) : undefined;
  });
  const filtersActive = createMemo(() => hasSegmentFilters(segments()));
  const overviewEmpty = createMemo(() => {
    const data = overviewData();
    return (
      !overviewLoading() &&
      !overviewError() &&
      data !== undefined &&
      !hasOverviewData(data) &&
      !filtersActive()
    );
  });

  let inFlight: Promise<void> | undefined;
  let inFlightKey: string | undefined;
  // A request for a range the visitor has since moved off must not land last.
  let latestRequest = 0;

  const loadOverview = async (
    nextRange: DateRangeSeconds,
    options: { background?: boolean } = {},
  ) => {
    const background = options.background ?? false;
    const key = `${nextRange.start}:${nextRange.end}:${segmentFiltersKey(segments())}`;
    if (inFlight && inFlightKey === key) {
      return inFlight;
    }
    const request = ++latestRequest;
    const promise = (async () => {
      const hasCachedData = overviewCache() !== undefined;
      if (!background || !hasCachedData) {
        setOverviewLoading(true);
        setOverviewError("");
      }
      try {
        const data = await fetchOverview(
          undefined,
          DEMO_PROJECT_ID,
          nextRange.start,
          nextRange.end,
          segments(),
        );
        if (request !== latestRequest) {
          return;
        }
        setOverviewCache({
          data,
          fetchedAt: Date.now(),
        });
      } catch (loadError) {
        if (request !== latestRequest) {
          return;
        }
        if (!background || !hasCachedData) {
          setOverviewCache(undefined);
          setOverviewError(
            loadError instanceof Error ? loadError.message : "Unable to load overview data.",
          );
        }
      } finally {
        if (request === latestRequest && (!background || !hasCachedData)) {
          setOverviewLoading(false);
        }
      }
    })().finally(() => {
      if (inFlight === promise) {
        inFlight = undefined;
        inFlightKey = undefined;
      }
    });
    inFlight = promise;
    inFlightKey = key;
    return promise;
  };

  createEffect<DateRangeSeconds | undefined>((previousRange) => {
    const currentRange = range();
    if (previousRange && !rangesEqual(previousRange, currentRange)) {
      setOverviewCache(undefined);
    }
    return currentRange;
  });

  createEffect<string | undefined>((previousSegmentKey) => {
    const currentSegmentKey = segmentFiltersKey(segments());
    if (previousSegmentKey !== undefined && previousSegmentKey !== currentSegmentKey) {
      setOverviewCache(undefined);
    }
    return currentSegmentKey;
  });

  createEffect(() => {
    const currentRange = range();
    if (!rangeReady()) {
      return;
    }
    const overviewEntry = overviewCache();
    if (!overviewEntry) {
      void loadOverview(currentRange);
    } else if (isCacheEntryStale(overviewEntry.fetchedAt)) {
      void loadOverview(currentRange, { background: true });
    }
  });

  return (
    <div class={embeddedShell}>
      <Show when={!props.concealed}>
        <DashboardFilters showBrand={false} showAccount={false} />
      </Show>
      <div class={tabsRoot}>
        <nav class={tabList} aria-label="Dashboard views">
          <For each={DEMO_TAB_ITEMS}>
            {(item) => (
              <Show
                when={item.value !== "overview" || props.pendingTab !== null}
                fallback={
                  <span class={tabTrigger} data-selected="true" aria-current="page">
                    <span class={`${item.iconClass} ${tabIcon}`} aria-hidden="true" />
                    <span>{item.label}</span>
                  </span>
                }
              >
                {/* A pending tab stays this same link, so keyboard focus survives the click. */}
                <a
                  href={demoTabHref(item.value)}
                  class={item.value === props.pendingTab ? tabTrigger : tabLink}
                  data-selected={item.value === props.pendingTab ? "true" : undefined}
                  aria-current={item.value === props.pendingTab ? "page" : undefined}
                  data-demo-tab={item.value}
                  onPointerEnter={() => props.openInPlace && props.onTabIntent()}
                  onFocus={() => props.openInPlace && props.onTabIntent()}
                  onClick={(event) => {
                    // Modified clicks still open the full demo in a new tab.
                    if (!props.openInPlace || !isPlainLeftClick(event)) {
                      return;
                    }
                    event.preventDefault();
                    props.onTabOpen(item.value);
                  }}
                >
                  <span class={`${item.iconClass} ${tabIcon}`} aria-hidden="true" />
                  <span>{item.label}</span>
                </a>
              </Show>
            )}
          </For>
        </nav>
        <div class={tabContent}>
          <Show
            when={rangeReady() && props.pendingTab === null}
            fallback={
              <Show
                when={props.pendingTab !== null && props.failed}
                fallback={<p class={message}>Loading analytics…</p>}
              >
                <p class={message}>
                  This view couldn't be loaded.{" "}
                  <a href={demoTabHref(props.pendingTab ?? "overview")}>Open it in the full demo</a>
                </p>
              </Show>
            }
          >
            <OverviewTabContent
              data={overviewData()}
              metrics={metrics()}
              loading={overviewLoading()}
              error={overviewError()}
              empty={overviewEmpty()}
            />
          </Show>
        </div>
      </div>
    </div>
  );
}
