import { render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import { charts, seriesData } from "../../../test/apexchartsMock";
import { testPayment } from "../../../test/dashboardFixtures";
import { disabledSourceGroups, setDisabledSourceGroups } from "../../lib/dashboardPreferences";
import type {
  GroupSummaries,
  RevenueSourcePeriod,
  RevenueSourceResponse,
  SourceGroup,
  SourceMetrics,
} from "../../lib/revenueSourcesApi";
import { DashboardStateProvider } from "./dashboardPeriodContext";
import RevenueSources, { type RevenueSourcesProps } from "./RevenueSources";
import { SOURCE_GROUP_ORDER } from "./revenueSourcesLogic";

vi.mock("./apexcharts", () => import("../../../test/apexchartsMock"));

afterEach(() => setDisabledSourceGroups([]));

function metrics(visitors: number, converting: number, revenue: number): SourceMetrics {
  return {
    visitors,
    converting_visitors: converting,
    revenue,
    conversion_rate: visitors ? converting / visitors : 0,
    revenue_per_visitor: visitors ? revenue / visitors : 0,
    conversion_lower_bound: visitors ? converting / visitors / 2 : 0,
    revenue_change: { ratio: 0.2 },
    visitors_change: { ratio: -0.1 },
    conversions_change: { ratio: null },
    revenue_per_visitor_change: { ratio: null },
    conversion_rate_change: { ratio: null },
  };
}

const period = {
  entries: [
    {
      kind: "referrer",
      id: 3,
      referrer: "google.com",
      referrer_type: "search",
      group: "search",
      landing_paths: [{ id: 5, path: "/pricing", count: 40 }],
      ...metrics(400, 8, 80_000),
    },
    { kind: "referrer", group: "direct", ...metrics(300, 3, 30_000) },
  ],
  total_visitors: 700,
  total_revenue: 110_000,
  direct_visitors: 300,
  direct_revenue: 30_000,
  search_visitors: 400,
  search_revenue: 80_000,
} as unknown as RevenueSourcePeriod;

const groups = Object.fromEntries(
  SOURCE_GROUP_ORDER.map((group) => [group, { group, ...metrics(0, 0, 0) }]),
) as GroupSummaries;
groups.search = { group: "search", ...metrics(400, 8, 80_000) };
groups.direct = { group: "direct", ...metrics(300, 3, 30_000) };

const data: RevenueSourceResponse = { current: period, groups };

function renderSources(overrides: Partial<RevenueSourcesProps> = {}) {
  const props: RevenueSourcesProps = {
    data,
    loading: false,
    error: "",
    fetchPayments: vi.fn(async () => ({
      conversions: [testPayment({ customer_email: "amy@x.io" })],
    })),
    fetchJourney: vi.fn(),
    ...overrides,
  };
  render(() => (
    <DashboardStateProvider>
      <RevenueSources {...props} />
    </DashboardStateProvider>
  ));
  return props;
}

const groupTile = (label: string) =>
  screen
    .getAllByRole("button")
    .find((button) => button.firstElementChild?.textContent === label) as HTMLElement;

describe("RevenueSources", () => {
  it("summarizes each source group with its change", () => {
    renderSources();
    const search = groupTile("Search");
    expect(search).toHaveTextContent("$80");
    expect(search).toHaveTextContent("400");
    expect(search).toHaveTextContent("20");
    expect(
      screen.getByText(/Displaying sources for sessions that began between/),
    ).toBeInTheDocument();
  });

  it("lists sources in the grid and the treemap", () => {
    renderSources();
    expect(
      screen.getByRole("button", { name: "Show conversions for google.com" }),
    ).toBeInTheDocument();
    const treemap = charts.find((chart) => chart.options.chart?.type === "treemap");
    expect(treemap && seriesData(treemap).map((series) => series.name)).toEqual([
      "Search",
      "Direct",
    ]);
  });

  it("hides a group from the grid and treemap", async () => {
    renderSources();
    await userEvent.click(groupTile("Search"));
    expect(disabledSourceGroups()).toEqual<SourceGroup[]>(["search"]);
    expect(screen.queryByRole("button", { name: "Show conversions for google.com" })).toBeNull();
  });

  it("selects a source, then shows its conversions once the parent scrolls to it", async () => {
    const [scrollTarget, setScrollTarget] = createSignal<{ key: string; id: number } | null>(null);
    const onRevenueSourceSelect = vi.fn();
    const fetchPayments = vi.fn(async () => ({
      conversions: [testPayment({ customer_email: "amy@x.io" })],
    }));
    render(() => (
      <DashboardStateProvider>
        <RevenueSources
          data={data}
          loading={false}
          error=""
          scrollTarget={scrollTarget()}
          onRevenueSourceSelect={onRevenueSourceSelect}
          fetchPayments={fetchPayments}
          fetchJourney={vi.fn()}
        />
      </DashboardStateProvider>
    ));
    const row = screen.getByRole("button", { name: "Show conversions for google.com" });
    await userEvent.click(row);
    expect(onRevenueSourceSelect).toHaveBeenCalledWith({ kind: "referrer", id: 3 });

    setScrollTarget({ key: row.dataset.recordKey ?? "", id: 1 });
    expect(await screen.findByText("amy@x.io")).toBeInTheDocument();
    expect(fetchPayments).toHaveBeenCalledWith({ referrerId: 3 }, undefined);
  });

  it("adds a source to the segment filters", async () => {
    const onSourceFilter = vi.fn();
    renderSources({ onSourceFilter, activeSourceFilterKeys: new Set() });
    await userEvent.click(screen.getByRole("button", { name: "Filter by google.com" }));
    expect(onSourceFilter).toHaveBeenCalledWith(
      expect.objectContaining({ kind: "referrer", id: 3 }),
    );
  });

  it("links a landing path", async () => {
    const onLandingPathSelect = vi.fn();
    renderSources({ onLandingPathSelect });
    await userEvent.click(
      screen.getByRole("button", { name: "View details for /pricing in Landing Pages" }),
    );
    expect(onLandingPathSelect).toHaveBeenCalledWith(5);
  });

  it("shows loading and errors", () => {
    renderSources({ data: undefined, loading: true });
    expect(screen.getByText("Loading sources…")).toBeInTheDocument();
  });

  it("shows the tab error", () => {
    renderSources({ error: "Sources failed." });
    expect(screen.getByText("Sources failed.")).toBeInTheDocument();
    expect(screen.queryByRole("table")).toBeNull();
  });
});
