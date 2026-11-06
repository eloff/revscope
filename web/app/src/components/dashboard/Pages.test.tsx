import { render, screen, within } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { batch, createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import { charts, seriesData } from "../../../test/apexchartsMock";
import { testPayment, testVisitor } from "../../../test/dashboardFixtures";
import { pagesSort, setPagesSort } from "../../lib/dashboardPreferences";
import { DEFAULT_PAGES_SORT, type PageRow, type PagesResponse } from "../../lib/pagesApi";
import { DEFAULT_VISITOR_SORT } from "../../lib/visitorsApi";
import { DashboardStateProvider } from "./dashboardPeriodContext";
import Pages, { type PagesProps } from "./Pages";
import PagesGrid from "./PagesGrid";

vi.mock("./apexcharts", () => import("../../../test/apexchartsMock"));

afterEach(() => setPagesSort(DEFAULT_PAGES_SORT));

function page(id: number, overrides: Partial<PageRow> = {}): PageRow {
  return {
    id,
    path: `/p${id}`,
    hostname: "x.io",
    visitors: 100,
    converting_visitors: 2,
    attributed_revenue: 10_000 * id,
    conversion_rate: 0.02,
    conversion_lower_bound: 0.01,
    conversion_ci: { lower: 0.01, upper: 0.05 },
    bounce_rate: 0.3,
    exit_rate: 0.6,
    avg_session_duration: 90,
    avg_page_duration: 40,
    avg_scroll_percent: 55,
    revenue_per_visitor: 100,
    increase_visitors_25pct: 0,
    increase_conversion_25pct: 0,
    reduce_exits_25pct: 0,
    reduce_bounces_25pct: 0,
    increase_session_25pct: 0,
    best_visitors_uplift: false,
    best_conversion_uplift: false,
    best_exit_uplift: false,
    best_bounce_uplift: false,
    best_session_uplift: false,
    highest_exit_rate: false,
    highest_bounce_rate: false,
    ...overrides,
  };
}

const data: PagesResponse = { pages: [page(2), page(1, { visitors: 0 })], offset: 0, limit: 200 };

type PanelProps = Parameters<typeof PagesGrid>[0];

function renderPanel(overrides: Partial<PanelProps> = {}) {
  const defaults: PanelProps = {
    data,
    treemapData: data,
    loading: false,
    treemapLoading: false,
    error: "",
    onPageChange: vi.fn(),
    onSortChange: vi.fn(),
    fetchTransitions: vi.fn(async () => ({
      total_visitors: 100,
      transitions: [{ path_id: 9, path: "/next", hostname: "x.io", visitors: 60, share: 0.6 }],
    })),
    fetchPayments: vi.fn(async () => ({
      conversions: [testPayment({ customer_email: "amy@x.io" })],
    })),
    fetchVisitors: vi.fn(async () => ({
      visitors: [testVisitor({ visitor_timestamp_id: "v7" })],
      offset: 0,
      limit: 100,
    })),
    fetchJourney: vi.fn(),
    fetchVisitorJourney: vi.fn(),
  };
  // Unlike a spread, this keeps an override's getter reactive. Unlike mergeProps, an
  // override can still be undefined, as data is during a reload.
  const props: PanelProps = Object.defineProperties(
    defaults,
    Object.getOwnPropertyDescriptors(overrides),
  );
  render(() => (
    <DashboardStateProvider>
      <PagesGrid {...props} />
    </DashboardStateProvider>
  ));
  return props;
}

describe("PagesGrid", () => {
  it("lists pages with exit rate and page duration", () => {
    renderPanel();
    const row = screen.getByRole("button", { name: "Show transitions for x.io/p2" });
    expect(row).toHaveTextContent("$20");
    expect(row).toHaveTextContent("60.0%");
    expect(row).toHaveTextContent("40s");
    expect(screen.getByText("Showing 1-2 pages")).toBeInTheDocument();
  });

  it("shows bounce rate and session duration for landing pages", () => {
    renderPanel({ variant: "landingPages" });
    const row = screen.getByRole("button", { name: "Show transitions for x.io/p2" });
    expect(row).toHaveTextContent("30.0%");
    expect(row).toHaveTextContent("1m 30s");
    expect(screen.getByText("Showing 1-2 landing pages")).toBeInTheDocument();
  });

  it("only drills into pages with visitors", () => {
    renderPanel();
    expect(screen.queryByRole("button", { name: "Show transitions for x.io/p1" })).toBeNull();
  });

  it("expands a page into its transitions, conversions and visitors", async () => {
    const props = renderPanel();
    await userEvent.click(screen.getByRole("button", { name: "Show transitions for x.io/p2" }));
    expect(props.fetchTransitions).toHaveBeenCalledWith(2, "pages");
    await vi.waitFor(() =>
      expect(charts.some((chart) => seriesData(chart)[0]?.data[0]?.[0] === "path:9")).toBe(true),
    );

    await userEvent.click(screen.getByRole("tab", { name: "Conversions" }));
    expect(await screen.findByText("amy@x.io")).toBeInTheDocument();
    expect(props.fetchPayments).toHaveBeenCalledWith({ pathId: 2 }, undefined);

    await userEvent.click(screen.getByRole("tab", { name: "Visitors" }));
    expect(
      await screen.findByRole("button", { name: "View journey for visitor v7" }),
    ).toBeInTheDocument();
    expect(props.fetchVisitors).toHaveBeenCalledWith(
      { kind: "pageview", pathId: 2 },
      0,
      DEFAULT_VISITOR_SORT,
    );

    await userEvent.click(screen.getByRole("tab", { name: "Conversions" }));
    expect(props.fetchPayments).toHaveBeenCalledTimes(1);
  });

  it("filters landing page conversions and visitors by landing", async () => {
    const props = renderPanel({ variant: "landingPages" });
    await userEvent.click(screen.getByRole("button", { name: "Show transitions for x.io/p2" }));
    expect(props.fetchTransitions).toHaveBeenCalledWith(2, "landingPages");
    await userEvent.click(screen.getByRole("tab", { name: "Conversions" }));
    await userEvent.click(screen.getByRole("tab", { name: "Visitors" }));
    expect(props.fetchPayments).toHaveBeenCalledWith({ landingPathId: 2 }, undefined);
    expect(props.fetchVisitors).toHaveBeenCalledWith(
      { kind: "landing_page", pathId: 2 },
      0,
      DEFAULT_VISITOR_SORT,
    );
  });

  it("shows a transitions error in the drill-down", async () => {
    renderPanel({
      fetchTransitions: async () => {
        throw new Error("Transitions unavailable.");
      },
    });
    await userEvent.click(screen.getByRole("button", { name: "Show transitions for x.io/p2" }));
    expect(await screen.findByText("Transitions unavailable.")).toBeInTheDocument();
  });

  it("hands row selection to the parent when it owns the URL", async () => {
    const onPathSelect = vi.fn();
    const props = renderPanel({ onPathSelect });
    await userEvent.click(screen.getByRole("button", { name: "Show transitions for x.io/p2" }));
    expect(onPathSelect).toHaveBeenCalledWith(2);
    expect(props.fetchTransitions).not.toHaveBeenCalled();
  });

  it("collapses a URL-selected row and reopens it when selected again", async () => {
    const [scrollTarget, setScrollTarget] = createSignal<PanelProps["scrollTarget"]>({
      pathId: 2,
      id: 1,
    });
    const onPathSelect = vi.fn((pathId: number | null) =>
      setScrollTarget(pathId === null ? null : { pathId, id: 2 }),
    );
    const [current, setCurrent] = createSignal<PagesResponse | undefined>(data);
    const [loading, setLoading] = createSignal(false);
    const props = renderPanel({
      onPathSelect,
      get data() {
        return current();
      },
      get loading() {
        return loading();
      },
      get scrollTarget() {
        return scrollTarget();
      },
    });

    await userEvent.click(screen.getByRole("button", { name: "Hide transitions for x.io/p2" }));
    expect(onPathSelect).toHaveBeenLastCalledWith(null);
    expect(screen.getByRole("button", { name: "Show transitions for x.io/p2" })).toHaveAttribute(
      "aria-expanded",
      "false",
    );

    // Navigating can trigger a refresh, which must not bring the row back.
    setCurrent(structuredClone(data));
    setLoading(true);
    setLoading(false);
    const collapsed = screen.getByRole("button", { name: "Show transitions for x.io/p2" });
    expect(collapsed).toHaveAttribute("aria-expanded", "false");
    expect(props.fetchTransitions).toHaveBeenCalledTimes(1);

    await userEvent.click(collapsed);
    expect(onPathSelect).toHaveBeenLastCalledWith(2);
    expect(screen.getByRole("button", { name: "Hide transitions for x.io/p2" })).toHaveAttribute(
      "aria-expanded",
      "true",
    );
  });

  it("reloads a URL-selected row's transitions on each data refresh", async () => {
    const [current, setCurrent] = createSignal<PagesResponse | undefined>(data);
    const [loading, setLoading] = createSignal(false);
    const props = renderPanel({
      onPathSelect: vi.fn(),
      scrollTarget: { pathId: 2, id: 1 },
      get data() {
        return current();
      },
      get loading() {
        return loading();
      },
    });
    const expectOpen = () =>
      expect(screen.getByRole("button", { name: "Hide transitions for x.io/p2" })).toHaveAttribute(
        "aria-expanded",
        "true",
      );
    expectOpen();
    expect(props.fetchTransitions).toHaveBeenCalledTimes(1);

    for (let round = 0; round < 6; round += 1) {
      // A background refresh swaps the data in place.
      setCurrent(structuredClone(data));
      expectOpen();
      expect(props.fetchTransitions).toHaveBeenCalledTimes(2 * round + 2);
      // A foreground reload clears it first.
      batch(() => {
        setLoading(true);
        setCurrent(undefined);
      });
      batch(() => {
        setLoading(false);
        setCurrent(structuredClone(data));
      });
      expectOpen();
      expect(props.fetchTransitions).toHaveBeenCalledTimes(2 * round + 3);
    }
  });

  it("leaves an open row alone when only the loading flag or route changes", async () => {
    const [loading, setLoading] = createSignal(false);
    const [scrollTarget, setScrollTarget] = createSignal({ pathId: 2, id: 1 });
    const props = renderPanel({
      onPathSelect: vi.fn(),
      get loading() {
        return loading();
      },
      get scrollTarget() {
        return scrollTarget();
      },
    });
    await userEvent.click(await screen.findByRole("tab", { name: "Conversions" }));

    setLoading(true);
    setLoading(false);
    setScrollTarget({ pathId: 2, id: 2 });

    expect(screen.getByRole("tab", { name: "Conversions" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(props.fetchTransitions).toHaveBeenCalledTimes(1);
  });

  it("sorts and tells the parent to refetch", async () => {
    const props = renderPanel();
    await userEvent.click(
      within(screen.getAllByRole("rowgroup")[0] as HTMLElement).getByRole("button", {
        name: "Visitors",
      }),
    );
    expect(pagesSort()).toEqual({ column: "visitors", direction: "desc" });
    expect(props.onSortChange).toHaveBeenCalled();
  });

  it("filters the dashboard by a page", async () => {
    const onPageFilter = vi.fn();
    renderPanel({ onPageFilter, activePageFilterKeys: new Set(["1"]) });
    expect(screen.getByRole("button", { name: "Filtering by x.io/p1" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await userEvent.click(screen.getByRole("button", { name: "Filter by x.io/p2" }));
    expect(onPageFilter).toHaveBeenCalledWith(2, "x.io/p2");
  });

  it("returns to the treemap's ranking for a tile not on this page", () => {
    setPagesSort({ column: "visitors", direction: "asc" });
    const props = renderPanel({ data: { pages: [page(5)], offset: 200, limit: 200 } });
    const treemap = charts.find(
      (chart) => chart.options.chart?.type === "treemap" && chart.history.length > 1,
    );
    treemap?.fire("dataPointSelection", { seriesIndex: 0, dataPointIndex: 0 });
    expect(pagesSort()).toEqual(DEFAULT_PAGES_SORT);
    expect(props.onSortChange).toHaveBeenCalled();
  });
});

describe("Pages", () => {
  function renderPages(overrides: Partial<PagesProps> = {}) {
    const props = {
      pagesMode: "pages",
      onPagesModeChange: vi.fn(),
      pagesEmpty: false,
      landingPagesEmpty: true,
      data,
      treemapData: data,
      loading: false,
      treemapLoading: false,
      error: "",
      landingData: undefined,
      landingTreemapData: undefined,
      landingLoading: false,
      landingTreemapLoading: false,
      landingError: "",
      onPagesPageChange: vi.fn(),
      onLandingPagesPageChange: vi.fn(),
      onPagesSortChange: vi.fn(),
      onLandingPagesSortChange: vi.fn(),
      fetchTransitions: vi.fn(),
      fetchPayments: vi.fn(),
      fetchVisitors: vi.fn(),
      fetchJourney: vi.fn(),
      fetchVisitorJourney: vi.fn(),
      ...overrides,
    } as PagesProps;
    render(() => (
      <DashboardStateProvider>
        <Pages {...props} />
      </DashboardStateProvider>
    ));
    return props;
  }

  it("notes the pageview window and switches to landing pages", async () => {
    const props = renderPages();
    expect(screen.getByText(/Displaying pages viewed between/)).toBeInTheDocument();
    expect(screen.getByText("No data for the selected date range")).toBeInTheDocument();
    await userEvent.click(screen.getByText("Landing Pages"));
    expect(props.onPagesModeChange).toHaveBeenCalledWith("landing");
  });

  it("notes the session window for landing pages", () => {
    renderPages({ pagesMode: "landing" });
    expect(
      screen.getByText(/Displaying landing pages for sessions that began between/),
    ).toBeInTheDocument();
  });
});
