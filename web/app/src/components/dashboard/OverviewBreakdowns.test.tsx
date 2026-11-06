import { render, screen, within } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { mockApi } from "../../../../shared/test/api";
import {
  emptyBreakdowns,
  testBreakdownRow,
  testBucket,
  testOverview,
  testPeriodMetrics,
} from "../../../test/dashboardFixtures";
import DemoOverview from "./DemoOverview";

vi.mock("./apexcharts", () => import("../../../test/apexchartsMock"));

const OVERVIEW = "/v2/read/projects/1/overview";

const overview = testOverview({
  current_metrics: testPeriodMetrics({ visitors: 30 }),
  buckets: [testBucket({ visitors: 30 })],
  breakdowns: {
    ...emptyBreakdowns(),
    countries: [
      testBreakdownRow({ value: "US", label: "United States", visitors: 20, revenue: 1_000 }),
      testBreakdownRow({ value: "FR", label: "France", visitors: 10, revenue: 9_000 }),
    ],
    os: [testBreakdownRow({ value: "macos", label: "macOS", visitors: 30 })],
  },
});

function countries(count: number) {
  return Array.from({ length: count }, (_, index) =>
    testBreakdownRow({
      value: `C${index}`,
      label: `Country ${index}`,
      visitors: 500 - index,
    }),
  );
}

function panel(name: string): HTMLElement {
  const tab = screen.getByRole("tab", { name });
  const root = tab.closest("[data-scope='tabs'][data-part='root']");
  if (!(root instanceof HTMLElement)) {
    throw new Error(`no panel for ${name}`);
  }
  return root;
}

function rowLabels(container: HTMLElement): string[] {
  return within(container)
    .queryAllByRole("button", { pressed: false })
    .concat(within(container).queryAllByRole("button", { pressed: true }))
    .map((button) => button.textContent ?? "");
}

describe("OverviewBreakdowns", () => {
  it("refetches the overview filtered by a clicked row, and removes the filter on a second click", async () => {
    const calls = mockApi("get", OVERVIEW, overview);
    render(() => <DemoOverview />);
    const locations = await vi.waitFor(() => panel("Country"));
    const us = await within(locations).findByRole("button", { name: /United States/ });
    expect(us.getAttribute("aria-pressed")).toBe("false");

    await userEvent.click(us);
    await vi.waitFor(() =>
      expect(calls.at(-1)?.url.searchParams.getAll("country")).toEqual(["US"]),
    );
    const filtered = await within(panel("Country")).findByRole("button", {
      name: /United States/,
      pressed: true,
    });
    expect(screen.getByText("Country: United States")).toBeTruthy();

    await userEvent.click(filtered);
    await within(panel("Country")).findByRole("button", { name: /United States/, pressed: false });
    expect(screen.queryByText("Country: United States")).toBeNull();
  });

  it("filters by a landing attribute from another tab", async () => {
    const calls = mockApi("get", OVERVIEW, overview);
    render(() => <DemoOverview />);
    await vi.waitFor(() => panel("OS"));
    await userEvent.click(screen.getByRole("tab", { name: "OS" }));
    await userEvent.click(await screen.findByRole("button", { name: /macOS/ }));
    await vi.waitFor(() => expect(calls.at(-1)?.url.searchParams.get("os")).toBe("macos"));
    expect(screen.getByText("OS: macOS")).toBeTruthy();
  });

  it("re-sorts a panel by a clicked column header, flipping direction on a second click", async () => {
    mockApi("get", OVERVIEW, overview);
    render(() => <DemoOverview />);
    const locations = await vi.waitFor(() => panel("Country"));
    await within(locations).findByRole("button", { name: /United States/ });
    expect(rowLabels(locations)[0]).toMatch(/^United States/);
    const header = (name: string) => within(locations).getByRole("button", { name });
    expect(header("Visitors, sorted descending").textContent).toContain("▼");

    await userEvent.click(header("Revenue"));
    expect(rowLabels(locations)[0]).toMatch(/^France/);
    expect(header("Revenue, sorted descending").textContent).toContain("▼");
    expect(header("Visitors").textContent).toBe("Visitors");

    await userEvent.click(header("Revenue, sorted descending"));
    expect(rowLabels(locations)[0]).toMatch(/^United States/);
    expect(header("Revenue, sorted ascending").textContent).toContain("▲");
  });

  it("filters by source from the campaign tab, which has no spend column", async () => {
    const calls = mockApi("get", OVERVIEW, {
      ...overview,
      breakdowns: {
        ...overview.breakdowns,
        campaigns: [testBreakdownRow({ value: "campaign-4", label: "Launch", visitors: 12 })],
      },
    });
    render(() => <DemoOverview />);
    await vi.waitFor(() => panel("Campaign"));
    expect(screen.queryByRole("tab", { name: "Region" })).toBeNull();
    await userEvent.click(screen.getByRole("tab", { name: "Campaign" }));
    const sources = panel("Campaign");
    expect(within(sources).queryByRole("button", { name: /Spend/ })).toBeNull();
    const launch = await within(sources).findByRole("button", { name: /Launch/ });
    await userEvent.click(launch);
    await vi.waitFor(() => expect(calls.at(-1)?.url.searchParams.get("source")).toBe("campaign-4"));
  });

  it("shows every row the overview sent, past 100, with no More button", async () => {
    const rows = [
      ...countries(150),
      // Ranks last by visitors; the overview sends it for its revenue.
      testBreakdownRow({ value: "rich", label: "Rich", visitors: 1, revenue: 9_000 }),
    ];
    mockApi("get", OVERVIEW, {
      ...overview,
      breakdowns: { ...emptyBreakdowns(), countries: rows, os: overview.breakdowns.os },
    });
    render(() => <DemoOverview />);
    const locations = await vi.waitFor(() => panel("Country"));
    await within(locations).findByRole("button", { name: /Country 0/ });
    expect(rowLabels(locations)).toHaveLength(151);
    // Unopened tabs render no rows; there could be hundreds per tab.
    expect(screen.queryByTitle(/^macOS/)).toBeNull();
    await userEvent.click(screen.getByRole("tab", { name: "OS" }));
    expect(await screen.findByTitle(/^macOS/)).toBeTruthy();
    expect(within(locations).queryByRole("button", { name: /^(More|Less)$/ })).toBeNull();
  });
});
