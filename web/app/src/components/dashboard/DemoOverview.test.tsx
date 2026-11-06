import { render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { HttpResponse } from "msw";
import { afterEach, describe, expect, it, vi } from "vitest";
import { mockApi } from "../../../../shared/test/api";
import { testBucket, testOverview, testPeriodMetrics } from "../../../test/dashboardFixtures";
import DemoOverview, { DEMO_EMBED_ID } from "./DemoOverview";

vi.mock("./apexcharts", () => import("../../../test/apexchartsMock"));

const OVERVIEW = "/v2/read/projects/1/overview";

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
});
