import { render, screen } from "@solidjs/testing-library";
import { describe, expect, it, vi } from "vitest";
import { lastChart, seriesData } from "../../../test/apexchartsMock";
import type { PageRow, PageTransitionRow } from "../../lib/pagesApi";
import type { SearchTermRow, SearchTermsResponse } from "../../lib/searchTermsApi";
import CrawlersTreemap from "./CrawlersTreemap";
import { buildOperatorLeaves } from "./crawlersLogic";
import FunnelDropoffTreemap from "./FunnelDropoffTreemap";
import PagesTreemap, { pageRowKey, pageUrl } from "./PagesTreemap";
import PageTransitionsTreemap from "./PageTransitionsTreemap";
import RevenueSourcesTreemap from "./RevenueSourcesTreemap";
import type { TreemapLeaf } from "./revenueSourcesLogic";
import SearchTermsTreemap from "./SearchTermsTreemap";

vi.mock("./apexcharts", () => import("../../../test/apexchartsMock"));

function page(id: number, attributed_revenue: number, visitors = 10): PageRow {
  return {
    id,
    path: `/p${id}`,
    hostname: "example.com",
    visitors,
    attributed_revenue,
    converting_visitors: 0,
    revenue_per_visitor: 0,
    bounce_rate: 0,
    exit_rate: 0,
    avg_session_duration: 0,
    avg_page_duration: 0,
    avg_scroll_percent: 0,
  } as PageRow;
}

describe("PagesTreemap", () => {
  it("ranks pages by attributed revenue with an Other tile", () => {
    render(() => (
      <PagesTreemap
        data={{ pages: [page(1, 100), page(2, 300), page(3, 300, 50)], offset: 0, limit: 200 }}
        loading={false}
      />
    ));
    const [top, other] = seriesData();
    expect(top?.data).toEqual([
      ["page:3", 300],
      ["page:2", 300],
      ["page:1", 100],
    ]);
    expect(other?.name).toBe("Other");
    const formatter = lastChart().options.dataLabels?.formatter as (text: string) => string;
    expect(formatter("page:2")).toBe("/p2");
  });

  it("selects a page and shows its hover card", async () => {
    const onLeafSelect = vi.fn();
    render(() => (
      <PagesTreemap
        data={{ pages: [page(1, 100)], offset: 0, limit: 200 }}
        loading={false}
        onLeafSelect={onLeafSelect}
      />
    ));
    const chart = lastChart();
    chart.fire("dataPointSelection", { seriesIndex: 0, dataPointIndex: 0 });
    chart.fire("dataPointSelection", { seriesIndex: 1, dataPointIndex: 0 });
    expect(onLeafSelect.mock.calls).toEqual([["page:1"]]);

    chart.fire("dataPointMouseEnter", { seriesIndex: 0, dataPointIndex: 0 });
    expect(await screen.findByText("example.com/p1")).toBeInTheDocument();
    expect(screen.getByText("Attributed revenue")).toBeInTheDocument();
  });

  it("uses its own loading label", () => {
    render(() => (
      <PagesTreemap
        data={undefined}
        loading
        labels={{ loading: "Loading landing pages…", otherTooltipNoun: "landing pages" }}
      />
    ));
    expect(screen.getByText("Loading landing pages…")).toBeInTheDocument();
  });

  it("keys and labels pages", () => {
    expect(pageRowKey({ id: 4 })).toBe("page:4");
    expect(pageUrl({ hostname: "a.io", path: "/x" })).toBe("a.io/x");
  });
});

describe("PageTransitionsTreemap", () => {
  const labels = { loading: "Loading…", empty: "Nowhere next.", topGroup: "Next pages" };
  const transitions: PageTransitionRow[] = [
    { path_id: 2, path: "/pricing", hostname: "example.com", visitors: 30, share: 0.5 },
    { visitors: 20, share: 0.33 },
    { outbound_to: "github.com", visitors: 10, share: 0.17 },
  ];

  it("groups exits and outbound links apart from next pages", () => {
    render(() => (
      <PageTransitionsTreemap
        data={{ total_visitors: 60, transitions }}
        loading={false}
        labels={labels}
      />
    ));
    expect(seriesData()).toEqual([
      { name: "Next pages", data: [["path:2", 30]] },
      {
        name: "Exit",
        data: [
          ["__page_transition_exit__", 20],
          ["outbound:github.com", 10],
        ],
      },
    ]);
  });

  it("drills into a next page but not an exit", () => {
    const onPathSelect = vi.fn();
    render(() => (
      <PageTransitionsTreemap
        data={{ total_visitors: 60, transitions }}
        loading={false}
        labels={labels}
        onPathSelect={onPathSelect}
      />
    ));
    const chart = lastChart();
    chart.fire("dataPointSelection", { seriesIndex: 0, dataPointIndex: 0 });
    chart.fire("dataPointSelection", { seriesIndex: 1, dataPointIndex: 0 });
    expect(onPathSelect.mock.calls).toEqual([[2]]);
  });

  it("explains an outbound exit on hover", async () => {
    render(() => (
      <PageTransitionsTreemap
        data={{ total_visitors: 60, transitions }}
        loading={false}
        labels={labels}
      />
    ));
    lastChart().fire("dataPointMouseEnter", { seriesIndex: 1, dataPointIndex: 1 });
    expect(await screen.findByText(/left this page via an outbound link/)).toBeInTheDocument();
    expect(screen.getByText("17.0%")).toBeInTheDocument();
  });

  it("says when there is nowhere next", () => {
    render(() => (
      <PageTransitionsTreemap
        data={{ total_visitors: 0, transitions: [] }}
        loading={false}
        labels={labels}
      />
    ));
    expect(screen.getByText("Nowhere next.")).toBeInTheDocument();
  });
});

describe("FunnelDropoffTreemap", () => {
  it("rolls destinations past the top 24 into Other", async () => {
    const transitions = Array.from({ length: 26 }, (_, index) => ({
      path_id: index,
      path: `/d${index}`,
      hostname: "x.io",
      visitors: 100 - index,
      share: 0.01,
    }));
    render(() => <FunnelDropoffTreemap transitions={transitions} />);
    const all = seriesData().flatMap((series) => series.data);
    expect(all).toHaveLength(25);
    expect(all.at(-1)).toEqual(["__funnel_dropoff_other__", 76 + 75]);
  });
});

describe("CrawlersTreemap", () => {
  const leaves = buildOperatorLeaves([
    { useragent_id: 1, source: 0, verified: true, operator: "openai", views: 30 },
    { useragent_id: 2, source: 0, verified: false, views: 10 },
  ] as never);

  it("draws one tile per operator in its color", () => {
    render(() => <CrawlersTreemap leaves={leaves} loading={false} />);
    expect(seriesData()[0]?.data).toEqual([
      ["OpenAI", 30],
      ["Unverified", 10],
    ]);
    expect(lastChart().options.colors).toEqual(leaves.map((leaf) => leaf.color));
  });

  it("selects an operator and shows views on hover", async () => {
    const onLeafSelect = vi.fn();
    render(() => <CrawlersTreemap leaves={leaves} loading={false} onLeafSelect={onLeafSelect} />);
    lastChart().fire("dataPointSelection", { seriesIndex: 0, dataPointIndex: 0 });
    expect(onLeafSelect).toHaveBeenCalledWith("OpenAI");
    lastChart().fire("dataPointMouseEnter", { seriesIndex: 0, dataPointIndex: 1 });
    expect(await screen.findByText("25.0%")).toBeInTheDocument();
  });

  it("says when the selected categories have no views", () => {
    render(() => <CrawlersTreemap leaves={[]} loading={false} />);
    expect(screen.getByText("No crawler views for the selected categories.")).toBeInTheDocument();
  });
});

describe("RevenueSourcesTreemap", () => {
  const leaf = (group: TreemapLeaf["group"], key: string, revenue: number): TreemapLeaf => ({
    group,
    label: key,
    tooltip: key,
    datumKey: key,
    revenue,
    visitors: 1,
  });

  it("draws a series per source group in group order", () => {
    const leaves = new Map<TreemapLeaf["group"], TreemapLeaf[]>([
      ["direct", [leaf("direct", "d", 50)]],
      ["search", [leaf("search", "google", 200)]],
    ]);
    render(() => (
      <RevenueSourcesTreemap
        leaves={leaves}
        recordByKey={new Map()}
        bestHighlights={{} as never}
        loading={false}
        error=""
      />
    ));
    expect(seriesData()).toEqual([
      { name: "Search", data: [["google", 200]] },
      { name: "Direct", data: [["d", 50]] },
    ]);
  });

  it("shows a load error", () => {
    render(() => (
      <RevenueSourcesTreemap
        leaves={undefined}
        recordByKey={new Map()}
        bestHighlights={{} as never}
        loading={false}
        error="Sources failed to load."
      />
    ));
    expect(screen.getByText("Sources failed to load.")).toBeInTheDocument();
  });
});

describe("SearchTermsTreemap", () => {
  function term(query: string, estimated_revenue: number): SearchTermRow {
    return {
      query,
      estimated_revenue,
      clicks: 1,
      impressions: 10,
      landing_paths: [],
    } as unknown as SearchTermRow;
  }

  it("leaves out zero-revenue terms and adds the remaining channel revenue as Other", () => {
    const data = {
      current: {
        search_channel_revenue: 1000,
        summary: { clicks: 0, impressions: 0 },
        terms: [term("crm", 600), term("free crm", 0)],
      },
      offset: 0,
      limit: 100,
    } as unknown as SearchTermsResponse;
    render(() => <SearchTermsTreemap data={data} loading={false} />);
    expect(seriesData().flatMap((series) => series.data)).toEqual([
      ["term:crm", 600],
      ["__search_terms_other__", 400],
    ]);
  });
});
