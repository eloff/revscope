import { isEmbedPage, writeHistoryUrl } from "./embedHistory";

export function readJourneyHistoryParam(param: string): string | null {
  if (typeof window === "undefined") {
    return null;
  }
  return new URLSearchParams(window.location.search).get(param);
}

export function pushJourneyHistoryParam(param: string, value: string) {
  if (typeof window === "undefined") {
    return;
  }
  const url = new URL(window.location.href);
  url.searchParams.set(param, value);
  writeHistoryUrl(url);
}

export function clearJourneyHistoryParam(param: string) {
  if (typeof window === "undefined") {
    return;
  }
  const url = new URL(window.location.href);
  if (!url.searchParams.has(param)) {
    return;
  }
  url.searchParams.delete(param);
  window.history.replaceState(null, "", url);
}

/** `close` runs instead of Back in an embed, where opening the journey added no entry to go back over. */
export function backFromJourneyHistory(close: () => void) {
  if (typeof window === "undefined") {
    return;
  }
  if (isEmbedPage()) {
    close();
  } else {
    window.history.back();
  }
}

const NEAR_PAGE_TOP_PX = 720;

function scrollableAncestor(element: HTMLElement): HTMLElement | null {
  let parent = element.parentElement;
  while (parent) {
    const { overflowY } = getComputedStyle(parent);
    if (overflowY === "auto" || overflowY === "scroll") {
      return parent;
    }
    parent = parent.parentElement;
  }
  return null;
}

/**
 * Show the start of a journey that just opened. Journeys near the top of the
 * page reset the scrollport; ones nested farther down (a page drill-in) are
 * aligned to the top of the scrollport.
 */
export function scrollOpenedPanelIntoView(element: HTMLElement) {
  const parent = scrollableAncestor(element);
  const viewportTop = parent ? parent.getBoundingClientRect().top : 0;
  const currentTop = parent ? parent.scrollTop : window.scrollY;
  const contentTop = currentTop + (element.getBoundingClientRect().top - viewportTop);
  const scrollTo = (top: number) => {
    if (parent) {
      parent.scrollTo({ top, behavior: "auto" });
      return;
    }
    window.scrollTo({ top, behavior: "auto" });
  };

  if (contentTop < NEAR_PAGE_TOP_PX) {
    if (currentTop !== 0) {
      scrollTo(0);
    }
    return;
  }
  const topOffset = element.getBoundingClientRect().top - viewportTop;
  if (topOffset < 0 || topOffset > 12) {
    scrollTo(Math.max(0, contentTop - 12));
  }
}

export function installJourneyHistoryListener(
  param: string,
  onPopState: (value: string | null) => void,
) {
  if (typeof window === "undefined") {
    return () => {};
  }
  const handlePopState = () => onPopState(readJourneyHistoryParam(param));
  window.addEventListener("popstate", handlePopState);
  return () => window.removeEventListener("popstate", handlePopState);
}
