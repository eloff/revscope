import { afterEach, describe, expect, it, vi } from "vitest";
import { EMBED_PAGE_ATTRIBUTE } from "./embedHistory";
import {
  backFromJourneyHistory,
  clearJourneyHistoryParam,
  installJourneyHistoryListener,
  pushJourneyHistoryParam,
  readJourneyHistoryParam,
  scrollOpenedPanelIntoView,
} from "./journeyHistory";

afterEach(() => {
  document.documentElement.removeAttribute(EMBED_PAGE_ATTRIBUTE);
  window.history.replaceState(null, "", "/");
  vi.restoreAllMocks();
});

describe("journey history in a framed page that isn't an embed", () => {
  it("keeps pushing entries, as when the dashboard sits in someone's dashboard panel", () => {
    const realParent = window.parent;
    Object.defineProperty(window, "parent", { value: {}, configurable: true });
    try {
      const length = window.history.length;
      pushJourneyHistoryParam("journey", "abc");
      expect(window.history.length).toBe(length + 1);
    } finally {
      Object.defineProperty(window, "parent", { value: realParent, configurable: true });
    }
  });
});

describe("journey history in an embed", () => {
  // The home page demo embed shares the host page's session history.
  it("adds no entries, and Back closes the journey instead of leaving the host page", () => {
    document.documentElement.setAttribute(EMBED_PAGE_ATTRIBUTE, "");
    window.history.replaceState(null, "", "/embed?tab=visitors");
    const length = window.history.length;
    pushJourneyHistoryParam("journey", "abc");
    expect(window.location.search).toBe("?tab=visitors&journey=abc");
    expect(window.history.length).toBe(length);

    const back = vi.spyOn(window.history, "back");
    const close = vi.fn();
    backFromJourneyHistory(close);
    expect(close).toHaveBeenCalledTimes(1);
    expect(back).not.toHaveBeenCalled();
  });
});

describe("journey history params", () => {
  it("pushes, reads and clears a param", () => {
    window.history.replaceState(null, "", "/dashboard?tab=pages");
    const length = window.history.length;
    pushJourneyHistoryParam("journey", "abc");
    expect(window.location.search).toBe("?tab=pages&journey=abc");
    expect(window.history.length).toBe(length + 1);
    expect(readJourneyHistoryParam("journey")).toBe("abc");

    clearJourneyHistoryParam("journey");
    expect(window.location.search).toBe("?tab=pages");
    expect(window.history.length).toBe(length + 1);
  });

  it("does not touch history when the param is already absent", () => {
    const replace = vi.spyOn(window.history, "replaceState");
    clearJourneyHistoryParam("journey");
    expect(replace).not.toHaveBeenCalled();
  });

  it("reports the param on popstate until uninstalled", () => {
    const onPopState = vi.fn();
    const uninstall = installJourneyHistoryListener("journey", onPopState);
    window.history.replaceState(null, "", "/?journey=x");
    window.dispatchEvent(new PopStateEvent("popstate"));
    uninstall();
    window.dispatchEvent(new PopStateEvent("popstate"));
    expect(onPopState.mock.calls).toEqual([["x"]]);
  });
});

describe("scrollOpenedPanelIntoView", () => {
  function panelAt(top: number, scroller?: HTMLElement) {
    const element = document.createElement("div");
    (scroller ?? document.body).append(element);
    vi.spyOn(element, "getBoundingClientRect").mockReturnValue({ top } as DOMRect);
    return element;
  }

  it("resets the window scroll for a panel near the top", () => {
    const scrollTo = vi.fn();
    vi.stubGlobal("scrollTo", scrollTo);
    vi.spyOn(window, "scrollY", "get").mockReturnValue(100);
    scrollOpenedPanelIntoView(panelAt(200));
    expect(scrollTo).toHaveBeenCalledWith({ top: 0, behavior: "auto" });
    vi.unstubAllGlobals();
  });

  it("aligns a panel deep in a scroll container to its top", () => {
    const scroller = document.createElement("div");
    scroller.style.overflowY = "auto";
    document.body.append(scroller);
    scroller.scrollTop = 1000;
    scroller.scrollTo = vi.fn();
    vi.spyOn(scroller, "getBoundingClientRect").mockReturnValue({ top: 50 } as DOMRect);
    scrollOpenedPanelIntoView(panelAt(450, scroller));
    // 1000 scrolled + 400 below the scroller's top, less the 12px margin.
    expect(scroller.scrollTo).toHaveBeenCalledWith({ top: 1388, behavior: "auto" });
    scroller.remove();
  });

  it("leaves a deep panel already at the top alone", () => {
    const scrollTo = vi.fn();
    vi.stubGlobal("scrollTo", scrollTo);
    vi.spyOn(window, "scrollY", "get").mockReturnValue(2000);
    scrollOpenedPanelIntoView(panelAt(5));
    expect(scrollTo).not.toHaveBeenCalled();
    vi.unstubAllGlobals();
  });
});
