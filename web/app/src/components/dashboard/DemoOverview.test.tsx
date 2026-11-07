import { fireEvent, render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { HttpResponse } from "msw";
import { afterEach, beforeAll, beforeEach, describe, expect, it, onTestFinished, vi } from "vitest";
import { mockApi, queryOf } from "../../../../shared/test/api";
import { testBucket, testOverview, testPeriodMetrics } from "../../../test/dashboardFixtures";
import { EMBED_REPORT_HEIGHT_EVENT } from "../../lib/embedExpansion";
import { EMBED_PAGE_ATTRIBUTE } from "../../lib/embedHistory";
import { createOnceLoader } from "../../lib/onceLoader";
import DemoOverview, { DEMO_EMBED_ID } from "./DemoOverview";

vi.mock("./apexcharts", () => import("../../../test/apexchartsMock"));

// Each test gets a fresh once-loader around a counted import, so download counts
// and slow or failed loads don't leak between tests through the module-level loader.
const shellLoad = vi.hoisted(() => ({
  loader: undefined as (() => Promise<unknown>) | undefined,
}));
vi.mock("./loadDashboardShell", () => ({
  // Tests that only expand still prefetch; outside the tab tests that load never lands.
  loadDashboardShell: () => (shellLoad.loader ?? (() => new Promise(() => {})))(),
}));

const READ = "/v2/read/projects/1";
const OVERVIEW = `${READ}/overview`;

const withVisitors = (visitors: number) =>
  testOverview({
    current_metrics: testPeriodMetrics({ visitors, total_revenue: 5000 }),
    buckets: [testBucket({ visitors })],
  });

/** The Visitors tile's value (hover cards repeat the label, so go by the tile button). */
function visitorsTile(): string {
  const tile = screen
    .getAllByRole("button")
    .find(
      (button) =>
        button.querySelector(":scope > div > div:first-child")?.textContent === "Visitors",
    );
  return tile?.querySelector(":scope > div > div:nth-child(2)")?.textContent ?? "";
}

describe("DemoOverview", () => {
  it("loads the demo project's overview without a session", async () => {
    const calls = mockApi("get", OVERVIEW, withVisitors(1234));
    render(() => <DemoOverview />);
    await vi.waitFor(() => expect(visitorsTile()).toBe((1234).toLocaleString()));
    expect(calls[0]?.credentials).toBe("omit");
  });

  it("links every other tab to the full demo", () => {
    mockApi("get", OVERVIEW, withVisitors(1));
    render(() => <DemoOverview />);
    expect(screen.getByRole("link", { name: "Pages" })).toHaveAttribute("href", "/demo?tab=pages");
    expect(screen.getByRole("link", { name: /Open demo in full screen/ })).toHaveAttribute(
      "href",
      "/demo",
    );
    expect(screen.queryByRole("link", { name: "Overview" })).toBeNull();
  });

  it("shows the setup prompt for an empty period", async () => {
    mockApi("get", OVERVIEW, testOverview());
    render(() => <DemoOverview />);
    expect(await screen.findByText("No data for the selected date range")).toBeInTheDocument();
  });

  it("shows a load error", async () => {
    mockApi("get", OVERVIEW, { error: "Demo is down." }, 503);
    render(() => <DemoOverview />);
    expect(await screen.findByText("Demo is down.")).toBeInTheDocument();
  });

  it("shows the latest range's numbers when an earlier request finishes last", async () => {
    let releaseFirst!: () => void;
    const firstGate = new Promise<void>((resolve) => {
      releaseFirst = resolve;
    });
    let requests = 0;
    mockApi("get", OVERVIEW, async () => {
      requests += 1;
      if (requests === 1) {
        await firstGate;
        return HttpResponse.json(withVisitors(111));
      }
      return HttpResponse.json(withVisitors(222));
    });
    render(() => <DemoOverview />);
    await vi.waitFor(() => expect(requests).toBe(1));
    await userEvent.click(screen.getByRole("button", { name: "Previous period" }));
    await vi.waitFor(() => expect(visitorsTile()).toBe("222"));
    releaseFirst();
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(visitorsTile()).toBe("222");
  });

  describe("framed on the home page", () => {
    const realParent = window.parent;
    afterEach(() => {
      Object.defineProperty(window, "parent", { value: realParent, configurable: true });
    });

    /** Frames the demo, returning what it posts to the host page. */
    function frame() {
      const postMessage = vi.fn();
      Object.defineProperty(window, "parent", { value: { postMessage }, configurable: true });
      return postMessage;
    }
    /** Solid sets `inert` as a property, which jsdom doesn't reflect to the attribute. */
    const isInert = (element: Element) => {
      for (let node: HTMLElement | null = element as HTMLElement; node; node = node.parentElement) {
        if (node.inert) {
          return true;
        }
      }
      return false;
    };
    const expandMessage = (expanded: boolean) => ({
      type: "revscope-embed-expand",
      embed: DEMO_EMBED_ID,
      expanded,
    });

    it("covers the inert dashboard with an expand overlay until clicked", async () => {
      const postMessage = frame();
      mockApi("get", OVERVIEW, withVisitors(1));
      render(() => <DemoOverview />);
      expect(document.documentElement.style.overflow).toBe("hidden");
      const pages = screen.getByRole("link", { name: "Pages" });
      expect(isInert(pages)).toBe(true);
      await userEvent.click(screen.getByRole("button", { name: "Click to explore the demo" }));
      expect(postMessage).toHaveBeenCalledWith(expandMessage(true), "*");
      expect(document.documentElement.style.overflow).toBe("");
      expect(isInert(pages)).toBe(false);
      expect(screen.queryByRole("button", { name: "Click to explore the demo" })).toBeNull();
      await vi.waitFor(() =>
        expect(screen.getByRole("button", { name: "Close demo" })).toHaveFocus(),
      );
    });

    it("collapses on the close button, Esc, or a request from the host page", async () => {
      const postMessage = frame();
      mockApi("get", OVERVIEW, withVisitors(1));
      render(() => <DemoOverview />);
      const expand = () =>
        userEvent.click(screen.getByRole("button", { name: "Click to explore the demo" }));

      await expand();
      await userEvent.click(screen.getByRole("button", { name: "Close demo" }));
      expect(postMessage).toHaveBeenLastCalledWith(expandMessage(false), "*");
      expect(document.documentElement.style.overflow).toBe("hidden");

      await expand();
      await userEvent.keyboard("{Escape}");
      expect(postMessage).toHaveBeenLastCalledWith(expandMessage(false), "*");

      await expand();
      window.dispatchEvent(
        new MessageEvent("message", {
          source: window.parent as Window,
          data: { type: "revscope-embed-collapse", embed: DEMO_EMBED_ID },
        }),
      );
      expect(postMessage).toHaveBeenLastCalledWith(expandMessage(false), "*");
      expect(screen.queryByRole("button", { name: "Close demo" })).toBeNull();
    });

    it("leaves Esc in a text box or input method alone", async () => {
      const postMessage = frame();
      mockApi("get", OVERVIEW, withVisitors(1));
      render(() => <DemoOverview />);
      await userEvent.click(screen.getByRole("button", { name: "Click to explore the demo" }));
      const question = document.createElement("textarea");
      document.body.append(question);
      question.focus();
      await userEvent.keyboard("{Escape}");
      question.remove();
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", isComposing: true }));
      expect(postMessage).toHaveBeenLastCalledWith(expandMessage(true), "*");
      expect(screen.getByRole("button", { name: "Close demo" })).toBeInTheDocument();
    });

    it("lets Esc close an open date picker before collapsing", async () => {
      const postMessage = frame();
      mockApi("get", OVERVIEW, withVisitors(1));
      render(() => <DemoOverview />);
      await userEvent.click(screen.getByRole("button", { name: "Click to explore the demo" }));
      await userEvent.click(screen.getAllByRole("textbox")[0] as HTMLElement);
      await screen.findByRole("listbox", { name: "Date range presets" });
      await userEvent.keyboard("{Escape}");
      await vi.waitFor(() =>
        expect(screen.queryByRole("listbox", { name: "Date range presets" })).toBeNull(),
      );
      expect(postMessage).toHaveBeenLastCalledWith(expandMessage(true), "*");
      await userEvent.keyboard("{Escape}");
      expect(postMessage).toHaveBeenLastCalledWith(expandMessage(false), "*");
    });
  });

  describe("tabs in the expanded embed", () => {
    // Longer than the idle prefetch's fallback delay, which jsdom's missing requestIdleCallback takes.
    const IDLE_FALLBACK_MS = 300;
    const realParent = window.parent;
    let importShell: ReturnType<typeof vi.fn<() => Promise<unknown>>>;
    // The first import compiles the whole dashboard, which outlasts waitFor's timeout.
    beforeAll(() => import("./DashboardShellContent"), 30_000);
    beforeEach(() => {
      Object.defineProperty(window, "parent", {
        value: { postMessage: vi.fn() },
        configurable: true,
      });
      document.documentElement.setAttribute(EMBED_PAGE_ATTRIBUTE, "");
      importShell = vi.fn(() => import("./DashboardShellContent"));
      shellLoad.loader = createOnceLoader(() => importShell());
      mockApi("get", "/v2/public/chat/status", { available: false });
      mockApi("get", OVERVIEW, withVisitors(1));
    });
    afterEach(() => {
      Object.defineProperty(window, "parent", { value: realParent, configurable: true });
      window.history.replaceState(null, "", "/");
      document.documentElement.removeAttribute(EMBED_PAGE_ATTRIBUTE);
      shellLoad.loader = undefined;
      vi.restoreAllMocks();
    });

    const mockEvents = () =>
      mockApi("get", `${READ}/events`, { events: [], offset: 0, limit: 200 });
    const expand = () =>
      userEvent.click(screen.getByRole("button", { name: "Click to explore the demo" }));
    /** The tab link or selected-tab marker labelled `name` in the overview's tab bar. */
    const overviewTab = (name: string) =>
      screen.getByText(name, { selector: "nav span" }).parentElement as HTMLElement;
    const dashboardTabSelected = (name: string) =>
      screen.getByRole("tab", { name }).getAttribute("aria-selected") === "true";

    it("downloads the dashboard once however many tabs are hovered before a click", async () => {
      mockEvents();
      render(() => <DemoOverview />);
      await expand();
      for (const name of ["Pages", "Events", "Visitors", "Audience"]) {
        await userEvent.hover(overviewTab(name));
      }
      await userEvent.click(overviewTab("Events"));
      await vi.waitFor(() => expect(dashboardTabSelected("Events")).toBe(true));
      expect(importShell).toHaveBeenCalledTimes(1);
    });

    /**
     * Earlier tests' idle prefetch timers outlive them and would count as this
     * test's download, so let them fire and start from an unused loader.
     */
    const startWithNoPendingPrefetch = async () => {
      await new Promise((resolve) => setTimeout(resolve, IDLE_FALLBACK_MS));
      importShell.mockClear();
      shellLoad.loader = createOnceLoader(() => importShell());
    };

    it("with data saver on, waits for a hovered tab before downloading the dashboard", async () => {
      await startWithNoPendingPrefetch();
      Object.defineProperty(navigator, "connection", {
        value: { saveData: true },
        configurable: true,
      });
      try {
        render(() => <DemoOverview />);
        await expand();
        await new Promise((resolve) => setTimeout(resolve, IDLE_FALLBACK_MS));
        expect(importShell).not.toHaveBeenCalled();

        await userEvent.hover(overviewTab("Events"));
        expect(importShell).toHaveBeenCalledTimes(1);
      } finally {
        Reflect.deleteProperty(navigator, "connection");
      }
    });

    it("downloads nothing for a visitor who hasn't expanded the demo", async () => {
      await startWithNoPendingPrefetch();
      render(() => <DemoOverview />);
      fireEvent.pointerEnter(overviewTab("Events"));
      overviewTab("Events").focus();
      await new Promise((resolve) => setTimeout(resolve, IDLE_FALLBACK_MS));
      expect(importShell).not.toHaveBeenCalled();
    });

    it("with data saver on, a focused tab also downloads the dashboard", async () => {
      await startWithNoPendingPrefetch();
      Object.defineProperty(navigator, "connection", {
        value: { saveData: true },
        configurable: true,
      });
      try {
        render(() => <DemoOverview />);
        await expand();
        overviewTab("Events").focus();
        expect(importShell).toHaveBeenCalledTimes(1);
      } finally {
        Reflect.deleteProperty(navigator, "connection");
      }
    });

    it("downloads the dashboard on idle after expanding, without a hover", async () => {
      await startWithNoPendingPrefetch();
      render(() => <DemoOverview />);
      await expand();
      await vi.waitFor(() => expect(importShell).toHaveBeenCalledTimes(1));
    });

    it("opens the tab in place on the chosen period without adding history entries", async () => {
      const overviews = mockApi("get", OVERVIEW, withVisitors(1));
      const events = mockEvents();
      render(() => <DemoOverview />);
      await expand();
      await userEvent.click(screen.getByRole("button", { name: "Previous period" }));
      await vi.waitFor(() => expect(overviews.length).toBeGreaterThan(1));
      const historyLength = window.history.length;

      await userEvent.click(overviewTab("Events"));
      await vi.waitFor(() => expect(events.length).toBe(1));
      const period = queryOf(overviews.at(-1));
      expect(queryOf(events[0])).toMatchObject({ start: period.start, end: period.end });
      expect(window.location.search).toBe("?tab=events");

      await userEvent.click(screen.getByRole("tab", { name: "Overview" }));
      expect(dashboardTabSelected("Overview")).toBe(true);
      expect(window.history.length).toBe(historyLength);
    });

    it("returns to the overview on collapse, keeping the frame's own URL params", async () => {
      window.history.replaceState(null, "", "/embed?ref=home");
      const overviews = mockApi("get", OVERVIEW, withVisitors(1));
      mockEvents();
      render(() => <DemoOverview />);
      await expand();
      await userEvent.click(overviewTab("Events"));
      await vi.waitFor(() => expect(dashboardTabSelected("Events")).toBe(true));
      expect(window.location.search).toBe("?ref=home&tab=events");
      const overviewLoads = overviews.length;

      // The host sizes the collapse from the last height it heard, so the
      // overview must be back and reported before the collapse message goes out.
      const order: string[] = [];
      const stopListening = new AbortController();
      onTestFinished(() => stopListening.abort());
      window.addEventListener(
        EMBED_REPORT_HEIGHT_EVENT,
        () => order.push(overviewTab("Overview").closest("[hidden]") ? "report:hidden" : "report"),
        { signal: stopListening.signal },
      );
      vi.mocked(window.parent.postMessage).mockImplementation(() => {
        order.push("message");
      });
      await userEvent.click(screen.getByRole("button", { name: "Close demo" }));
      expect(order).toEqual(["report", "message"]);
      expect(overviews.length).toBe(overviewLoads);
      expect(screen.queryByRole("tab", { name: "Events" })).toBeNull();
      expect(overviewTab("Overview")).toHaveAttribute("aria-current", "page");
      expect(window.location.search).toBe("?ref=home");
    });

    it("keeps keyboard focus on the tab opened from the keyboard", async () => {
      mockEvents();
      render(() => <DemoOverview />);
      await expand();
      overviewTab("Events").focus();
      await userEvent.keyboard("{Enter}");
      await vi.waitFor(() => expect(screen.getByRole("tab", { name: "Events" })).toHaveFocus());
    });

    it("shows the last clicked tab while the dashboard loads", async () => {
      let release!: () => void;
      const gate = new Promise<void>((resolve) => {
        release = resolve;
      });
      importShell.mockImplementation(async () => {
        await gate;
        return import("./DashboardShellContent");
      });
      mockEvents();
      mockApi("get", `${READ}/pages`, { pages: [], offset: 0, limit: 200 });
      render(() => <DemoOverview />);
      await expand();
      await userEvent.click(overviewTab("Pages"));
      expect(overviewTab("Pages")).toHaveAttribute("aria-current", "page");
      expect(screen.getByText("Loading analytics…")).toBeInTheDocument();
      await userEvent.click(overviewTab("Events"));
      expect(overviewTab("Events")).toHaveAttribute("aria-current", "page");
      expect(overviewTab("Pages")).not.toHaveAttribute("aria-current");
      release();
      await vi.waitFor(() => expect(dashboardTabSelected("Events")).toBe(true));
    });

    it("cancels a pending tab when Overview is clicked", async () => {
      let release!: () => void;
      let landed!: Promise<unknown>;
      importShell.mockImplementation(() => {
        landed = new Promise<void>((resolve) => {
          release = resolve;
        }).then(() => import("./DashboardShellContent"));
        return landed;
      });
      render(() => <DemoOverview />);
      await expand();
      await userEvent.click(overviewTab("Events"));
      await userEvent.click(overviewTab("Overview"));
      expect(overviewTab("Overview")).toHaveAttribute("aria-current", "page");
      release();
      await landed;
      // Every handler chained on the settled load has run once the timer fires.
      await new Promise((resolve) => setTimeout(resolve, 0));
      expect(screen.queryByRole("tab", { name: "Events" })).toBeNull();
    });

    it("links to the full demo page when the dashboard can't be downloaded", async () => {
      importShell.mockRejectedValue(new Error("chunk 404"));
      render(() => <DemoOverview />);
      await expand();
      await userEvent.click(overviewTab("Events"));
      const link = await screen.findByRole("link", { name: "Open it in the full demo" });
      expect(link).toHaveAttribute("href", "/demo?tab=events");
      expect(overviewTab("Events")).toHaveAttribute("aria-current", "page");
    });
  });
});
