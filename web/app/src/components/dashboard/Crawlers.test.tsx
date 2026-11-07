import { fireEvent, render, screen, within } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { charts, lastChart, seriesData } from "../../../test/apexchartsMock";
import {
  type CrawlerDetailResponse,
  type CrawlerRow,
  type CrawlersResponse,
  DEFAULT_CRAWLERS_SORT,
} from "../../lib/crawlersApi";
import {
  disabledCrawlerCategories,
  setDisabledCrawlerCategories,
} from "../../lib/dashboardPreferences";
import CrawlerDetailPanel from "./CrawlerDetailPanel";
import Crawlers, { type CrawlersProps } from "./Crawlers";
import { DashboardStateProvider } from "./dashboardPeriodContext";

vi.mock("./apexcharts", () => import("../../../test/apexchartsMock"));

afterEach(() => setDisabledCrawlerCategories([]));

const gptBot: CrawlerRow = {
  useragent_id: 1,
  source: 0,
  verified: true,
  useragent: "GPTBot/1.0",
  operator: "openai",
  category: "ai_crawler",
  views: 900,
};
const unknownBot: CrawlerRow = {
  useragent_id: 2,
  source: 0,
  verified: false,
  useragent: "curl/8",
  category: "headless",
  views: 100,
};

const data: CrawlersResponse = {
  rows: [gptBot, unknownBot],
  category_totals: {
    ai_crawler: 900,
    ai_search: 0,
    ai_assistant: 0,
    ai_browser: 0,
    search_engine: 0,
    archive: 0,
    headless: 100,
  },
  operators: [
    { source: 0, verified: true, operator: "openai", category: "ai_crawler", views: 900 },
    { source: 0, verified: false, category: "headless", views: 100 },
  ],
  offset: 0,
  limit: 100,
};

const detail: CrawlerDetailResponse = {
  granularity: "day",
  series: [
    { start: 1_700_000_000, views: 400 },
    { start: 1_700_086_400, views: 500 },
  ],
  pages: [
    { path_id: 1, path: "/a", hostname: "x.io", views: 10, last_3_days: [] },
    { path_id: 2, path: "/b", hostname: "x.io", views: 30, last_3_days: [1, 2] },
  ],
  last_3_days_start: 1_700_000_000,
};

function renderCrawlers(overrides: Partial<CrawlersProps> = {}) {
  const props: CrawlersProps = {
    data,
    loading: false,
    error: "",
    sort: DEFAULT_CRAWLERS_SORT,
    onSortChange: vi.fn(),
    onPageChange: vi.fn(),
    onFilterChange: vi.fn(),
    fetchDetail: vi.fn(async () => detail),
    ...overrides,
  };
  const result = render(() => (
    <DashboardStateProvider>
      <Crawlers {...props} />
    </DashboardStateProvider>
  ));
  return { props, ...result };
}

const categoryButton = (label: string) =>
  screen
    .getAllByRole("button")
    .find((button) => button.firstElementChild?.textContent === label) as HTMLElement;

describe("Crawlers", () => {
  it("shows category totals, the operator treemap and the crawler grid", () => {
    renderCrawlers();
    expect(categoryButton("AI crawler")).toHaveTextContent("900");
    expect(screen.getByText("OpenAI")).toBeInTheDocument();
    expect(screen.getByText("Unknown")).toBeInTheDocument();
    expect(screen.getByText("Showing 1-2 crawlers")).toBeInTheDocument();
    expect(screen.getByText(/Displaying crawler views between/)).toBeInTheDocument();
  });

  it("hides a category, or shows only it with Ctrl+click", async () => {
    const { props } = renderCrawlers();
    await userEvent.click(categoryButton("Headless browser"));
    expect(disabledCrawlerCategories()).toEqual(["headless"]);
    expect(categoryButton("Headless browser")).toHaveAttribute("aria-pressed", "false");
    expect(props.onFilterChange).toHaveBeenCalledTimes(1);
    const treemap = charts.find((chart) => chart.options.chart?.type === "treemap");
    expect(treemap && seriesData(treemap)[0]?.data).toHaveLength(1);

    fireEvent.click(categoryButton("AI search"), { ctrlKey: true });
    expect(disabledCrawlerCategories()).not.toContain("ai_search");
    expect(disabledCrawlerCategories()).toHaveLength(6);
  });

  it("expands a crawler to its views over time and pages", async () => {
    const { props } = renderCrawlers();
    await userEvent.click(screen.getByRole("row", { name: "Show details for GPTBot/1.0" }));
    expect(props.fetchDetail).toHaveBeenCalledWith(1, 0, true);
    const pagesTable = (await screen.findByText("x.io/b")).closest("table") as HTMLElement;
    const rows = within(pagesTable).getAllByRole("row").slice(1);
    expect(rows.map((row) => row.lastElementChild?.textContent)).toEqual(["x.io/b", "x.io/a"]);
    expect(screen.getByRole("row", { name: "Hide details for GPTBot/1.0" })).toHaveAttribute(
      "aria-expanded",
      "true",
    );

    await userEvent.click(screen.getByRole("row", { name: "Hide details for GPTBot/1.0" }));
    expect(screen.queryByText("x.io/b")).toBeNull();
  });

  it("shows why details failed to load", async () => {
    renderCrawlers({
      fetchDetail: async () => {
        throw new Error("Detail timed out.");
      },
    });
    await userEvent.click(screen.getByRole("row", { name: "Show details for curl/8" }));
    expect(await screen.findByText("Detail timed out.")).toBeInTheDocument();
  });

  it("jumps to the treemap's ranking when a tile's crawler isn't on this page", () => {
    const { props } = renderCrawlers({
      data: { ...data, rows: [unknownBot] },
      sort: { column: "useragent", direction: "asc" },
    });
    const treemap = charts.find((chart) => chart.options.chart?.type === "treemap");
    treemap?.fire("dataPointSelection", { seriesIndex: 0, dataPointIndex: 0 });
    expect(props.onSortChange).toHaveBeenCalledWith(DEFAULT_CRAWLERS_SORT);
  });

  it("shows the tab error without the grid", () => {
    renderCrawlers({ error: "Cloudflare token expired." });
    expect(screen.getByText("Cloudflare token expired.")).toBeInTheDocument();
    expect(screen.queryByRole("table")).toBeNull();
  });
});

describe("CrawlerDetailPanel", () => {
  function renderPanel(props: Parameters<typeof CrawlerDetailPanel>[0]) {
    return render(() => (
      <DashboardStateProvider>
        <CrawlerDetailPanel {...props} />
      </DashboardStateProvider>
    ));
  }

  it("plots views over time", () => {
    renderPanel({ data: detail, loading: false, error: "" });
    const viewsChart = charts.find((chart) => chart.options.chart?.type === "line");
    expect(viewsChart?.options.series).toEqual([{ name: "Views", data: [400, 500] }]);
  });

  it("sorts pages by name, then back by views", async () => {
    renderPanel({ data: detail, loading: false, error: "" });
    const pageNames = () => screen.getAllByTitle(/^x\.io\//).map((cell) => cell.textContent);
    await userEvent.click(screen.getByRole("button", { name: "Page" }));
    expect(pageNames()).toEqual(["x.io/a", "x.io/b"]);
    await userEvent.click(screen.getByRole("button", { name: /Views/ }));
    expect(pageNames()).toEqual(["x.io/b", "x.io/a"]);
  });

  it("says when there are no views or pages", () => {
    renderPanel({ data: { ...detail, series: [], pages: [] }, loading: false, error: "" });
    expect(screen.getByText("No views over this period.")).toBeInTheDocument();
    expect(screen.getByText("No pages for this crawler.")).toBeInTheDocument();
    expect(lastChart()).toBeDefined();
  });
});
