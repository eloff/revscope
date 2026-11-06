/// <reference types="solid-js" />
/**
 * Homepage island: overview metrics/chart plus tab links to `/demo`.
 * Kept separate from `Demo` so the landing page does not download the rest of
 * the dashboard until a visitor opens another tab.
 */
import { appPath } from "@revscope/web-shared/links";
import { createEffect, createMemo, createSignal, For, Show } from "solid-js";
import {
  DEMO_PROJECT_ID,
  deriveMetrics,
  fetchOverview,
  type OverviewResponse,
} from "../../lib/dashboardApi";
import { type CacheEntry, isCacheEntryStale, rangesEqual } from "../../lib/dashboardDataCache";
import type { DateRangeSeconds } from "../../lib/dateRange";
import { createEmbedExpansion } from "../../lib/embedExpansion";
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
import { DEMO_TAB_ITEMS, demoTabHref } from "./dashboardTabs";
import { message } from "./messages.css";
import OverviewTabContent from "./OverviewTabContent";
import { hasOverviewData } from "./tabDataPresence";
import { visuallyHidden } from "./visuallyHidden.css";

/** Must match the `embed` the home page's EmbedFrame uses. */
export const DEMO_EMBED_ID = "demo";

export default function DemoOverview() {
  const expansion = createEmbedExpansion(DEMO_EMBED_ID);
  const collapsed = () => expansion.enabled() && !expansion.expanded();
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
        <DashboardStateProvider>
          <DemoOverviewContent />
        </DashboardStateProvider>
      </div>
    </div>
  );
}

function DemoOverviewContent() {
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
      <DashboardFilters showBrand={false} showAccount={false} />
      <div class={tabsRoot}>
        <nav class={tabList} aria-label="Dashboard views">
          <For each={DEMO_TAB_ITEMS}>
            {(item) => (
              <Show
                when={item.value !== "overview"}
                fallback={
                  <span class={tabTrigger} data-selected="true" aria-current="page">
                    <span class={`${item.iconClass} ${tabIcon}`} aria-hidden="true" />
                    <span>{item.label}</span>
                  </span>
                }
              >
                <a href={demoTabHref(item.value)} class={tabLink}>
                  <span class={`${item.iconClass} ${tabIcon}`} aria-hidden="true" />
                  <span>{item.label}</span>
                </a>
              </Show>
            )}
          </For>
        </nav>
        <div class={tabContent}>
          <Show when={rangeReady()} fallback={<p class={message}>Loading analytics…</p>}>
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
