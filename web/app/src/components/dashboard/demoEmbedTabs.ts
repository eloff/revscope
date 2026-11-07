import { batch, type Component, createEffect, createSignal, on } from "solid-js";
import type { DashboardShellProps } from "./DashboardShellContent";
import { clearDashboardTargetParams } from "./dashboardRoute";
import type { DashboardTab } from "./dashboardTabs";
import { loadDashboardShell } from "./loadDashboardShell";

/** Fetches the full dashboard without rendering it, so a later tab click rarely waits. */
function prefetchDashboardShell() {
  // A failed prefetch is forgotten by the loader; the click that needs the
  // module retries and handles the failure itself.
  loadDashboardShell().catch(() => {});
}

function whenIdle(run: () => void) {
  if ("requestIdleCallback" in window) {
    window.requestIdleCallback(run, { timeout: 2000 });
  } else {
    setTimeout(run, 200);
  }
}

/**
 * Focuses the element once it renders. Some tabs appear only after a request
 * (Ask waits on whether chat is available), so it may not exist yet.
 */
function focusWhenRendered(selector: string) {
  const tryFocus = () => {
    const element = document.querySelector<HTMLElement>(selector);
    element?.focus();
    return element !== null;
  };
  queueMicrotask(() => {
    if (tryFocus()) {
      return;
    }
    const observer = new MutationObserver(() => {
      // Once the visitor has focused something else, a late tab mustn't take it.
      // The clicked overview link may keep focus while hidden until the browser
      // next renders, so it still counts as ours.
      const active = document.activeElement;
      const focusMoved = active !== document.body && !active?.matches("[data-demo-tab]");
      if (focusMoved || tryFocus()) {
        observer.disconnect();
      }
    });
    observer.observe(document.body, { childList: true, subtree: true });
    setTimeout(() => observer.disconnect(), 5000);
  });
}

function saveDataRequested(): boolean {
  const connection = (navigator as Navigator & { connection?: { saveData?: boolean } }).connection;
  return connection?.saveData === true;
}

/**
 * Points the frame's URL at the tab the dashboard should open on (it reads the
 * URL on mount), or clears it. Only params named like the dashboard's own are
 * cleared; others the host page put on the frame's src survive.
 */
function writeFrameTab(tab: DashboardTab | null) {
  const url = new URL(window.location.href);
  clearDashboardTargetParams(url.searchParams);
  if (tab) {
    url.searchParams.set("tab", tab);
  } else {
    url.searchParams.delete("tab");
  }
  window.history.replaceState(null, "", url);
}

/**
 * Lets the expanded home page embed switch tabs in place: the first tab click
 * swaps the overview for the full dashboard, loaded on demand. Collapsing goes
 * back to the overview, so the home page always previews the same thing.
 */
export function createDemoEmbedTabs(expanded: () => boolean) {
  const [requestedTab, setRequestedTab] = createSignal<DashboardTab | null>(null);
  const [failed, setFailed] = createSignal(false);
  const [shell, setShell] = createSignal<Component<DashboardShellProps>>();

  const reset = () => {
    setRequestedTab(null);
    setFailed(false);
  };

  createEffect(
    on(expanded, (isExpanded) => {
      if (isExpanded) {
        if (!saveDataRequested()) {
          whenIdle(prefetchDashboardShell);
        }
        return;
      }
      reset();
      if (shell()) {
        setShell(undefined);
        writeFrameTab(null);
      }
    }),
  );

  const open = (tab: DashboardTab) => {
    // Together, or going through no pending tab would build the overview only to drop it.
    batch(() => {
      reset();
      if (tab !== "overview") {
        setRequestedTab(tab);
      }
    });
    if (tab === "overview") {
      return;
    }
    loadDashboardShell().then(
      (module) => {
        // A later click, or collapsing, superseded this one while it loaded.
        if (requestedTab() !== tab) {
          return;
        }
        // The clicked link is hidden with the overview, so keyboard focus moves
        // to the same tab in the dashboard rather than falling to the body.
        const refocus = document.activeElement?.matches("[data-demo-tab]") ?? false;
        writeFrameTab(tab);
        setShell(() => module.default);
        if (refocus) {
          focusWhenRendered(`[data-dashboard-tab="${tab}"]`);
        }
      },
      () => {
        if (requestedTab() === tab) {
          setFailed(true);
        }
      },
    );
  };

  return {
    shell,
    /** The tab waiting on the dashboard to load, shown selected meanwhile. */
    pendingTab: () => (shell() ? null : requestedTab()),
    /**
     * The pending tab's download failed, typically because a deploy renamed the
     * chunk since this page loaded. The overview then links to the full demo
     * page instead: navigating the host page by script needs a recent click,
     * which a slow failure outlasts.
     */
    failed,
    open,
    prefetch: prefetchDashboardShell,
  };
}
