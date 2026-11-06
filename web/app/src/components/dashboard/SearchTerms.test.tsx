import { render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { lastChart } from "../../../test/apexchartsMock";
import {
  DEFAULT_SEARCH_TERMS_SORT,
  type SearchTermRow,
  type SearchTermsResponse,
} from "../../lib/searchTermsApi";
import { DashboardStateProvider } from "./dashboardPeriodContext";
import SearchTerms, { type SearchTermsProps } from "./SearchTerms";

vi.mock("./apexcharts", () => import("../../../test/apexchartsMock"));

const none = { ratio: null };

function term(query: string, overrides: Partial<SearchTermRow> = {}): SearchTermRow {
  return {
    query,
    landing_paths: [],
    position: 3.26,
    impressions: 1000,
    clicks: 50,
    mobile_clicks: 10,
    detail_clicks: 40,
    estimated_revenue: 20_000,
    ctr: 0.05,
    mobile_share: 0.25,
    expected_ctr: 0,
    lost_clicks: 0,
    lost_revenue: 0,
    highest_lost_clicks: false,
    highest_cannibalization: false,
    position_change: none,
    impressions_change: none,
    clicks_change: none,
    ctr_change: none,
    mobile_share_change: none,
    estimated_revenue_change: none,
    ...overrides,
  };
}

function response(
  terms: SearchTermRow[],
  overrides: Partial<SearchTermsResponse> = {},
): SearchTermsResponse {
  return {
    current: {
      search_channel_revenue: 50_000,
      summary: {
        position: 4.5,
        impressions: 5000,
        clicks: 120,
        ctr: 0.024,
        mobile_share: 0.3,
        estimated_revenue: 50_000,
        position_change: none,
        impressions_change: none,
        clicks_change: none,
        ctr_change: none,
        mobile_share_change: none,
        estimated_revenue_change: none,
      },
      terms,
    },
    offset: 0,
    limit: 100,
    ...overrides,
  };
}

const crm = term("crm", {
  landing_paths: [
    { path_id: 1, path: "/crm", clicks: 30 },
    { path_id: 2, path: "/blog/crm", clicks: 15 },
    { path_id: 3, path: "/pricing", clicks: 5 },
  ],
  highest_cannibalization: true,
});

function renderTerms(overrides: Partial<SearchTermsProps> = {}) {
  const data = response([crm, term("free crm")]);
  const props: SearchTermsProps = {
    data,
    treemapData: data,
    loading: false,
    treemapLoading: false,
    error: "",
    sort: DEFAULT_SEARCH_TERMS_SORT,
    onSortChange: vi.fn(),
    onPageChange: vi.fn(),
    ...overrides,
  };
  const result = render(() => (
    <DashboardStateProvider>
      <SearchTerms {...props} />
    </DashboardStateProvider>
  ));
  return { props, ...result };
}

describe("SearchTerms", () => {
  it("shows the totals row and each term's metrics", () => {
    renderTerms();
    expect(screen.getByRole("rowheader", { name: "Avg / total" }).parentElement).toHaveTextContent(
      `4.5${(5000).toLocaleString()}1202.40%30.0%$50`,
    );
    expect(screen.getAllByText("3.3").length).toBeGreaterThan(0);
    expect(screen.getAllByText("25.0%").length).toBeGreaterThan(0);
    expect(screen.getByText("Showing 1-2 search terms")).toBeInTheDocument();
  });

  it("lists two landing paths, with the rest in the tooltip", async () => {
    const onLandingPathSelect = vi.fn();
    renderTerms({ onLandingPathSelect });
    expect(
      screen.queryByRole("button", { name: "View details for /pricing in Landing Pages" }),
    ).toBeNull();
    const cell = screen
      .getByRole("button", { name: "View details for /crm in Landing Pages" })
      .closest("td");
    expect(cell?.title).toBe("/crm (30, 60.0%)\n/blog/crm (15, 30.0%)\n/pricing (5, 10.0%)");
    await userEvent.click(
      screen.getByRole("button", { name: "View details for /blog/crm in Landing Pages" }),
    );
    expect(onLandingPathSelect).toHaveBeenCalledWith(2);
  });

  it("asks the parent to sort", async () => {
    const { props } = renderTerms();
    await userEvent.click(screen.getByRole("button", { name: "Clicks" }));
    expect(props.onSortChange).toHaveBeenCalledWith({ orderBy: "clicks", direction: "desc" });
  });

  it("highlights the row for a treemap tile on this page", async () => {
    const { container } = renderTerms();
    lastChart().fire("dataPointSelection", { seriesIndex: 0, dataPointIndex: 0 });
    await vi.waitFor(() =>
      expect(container.querySelector('tr[data-record-key="term:crm"]')?.className).toMatch(
        /Highlighted/,
      ),
    );
  });

  it("returns to the treemap's ranking for a tile not on this page", () => {
    const pageTwo = response([term("other")], { offset: 100 });
    const { props } = renderTerms({
      data: pageTwo,
      treemapData: response([crm]),
      sort: { orderBy: "clicks", direction: "asc" },
    });
    lastChart().fire("dataPointSelection", { seriesIndex: 0, dataPointIndex: 0 });
    expect(props.onSortChange).toHaveBeenCalledWith(DEFAULT_SEARCH_TERMS_SORT);
  });

  it("goes back to the first page when already on the treemap's ranking", () => {
    const { props } = renderTerms({
      data: response([term("other")], { offset: 100 }),
      treemapData: response([crm]),
    });
    lastChart().fire("dataPointSelection", { seriesIndex: 0, dataPointIndex: 0 });
    expect(props.onPageChange).toHaveBeenCalledWith(0);
  });

  it("shows the tab error", () => {
    renderTerms({ error: "Search Console is not connected." });
    expect(screen.getByText("Search Console is not connected.")).toBeInTheDocument();
  });
});
