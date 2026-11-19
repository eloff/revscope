import { render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { batch, createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import { charts, lastChart, seriesData } from "../../../test/apexchartsMock";
import { rankBy, setRankBy } from "../../lib/dashboardPreferences";
import Treemap, { REVENUE_OR_VISITORS } from "./Treemap";
import type { TreemapMetric } from "./treemapLogic";

vi.mock("./apexcharts", () => import("../../../test/apexchartsMock"));

afterEach(() => setRankBy("revenue"));

type Row = { key: string; revenue: number; visitors: number };

function renderTreemap(
  rows: () => Row[] | undefined,
  options: { emptyLabel?: string; sliceRankedBy?: () => TreemapMetric | undefined } = {},
) {
  return render(() => (
    <Treemap
      groups={(metric: TreemapMetric) => {
        const current = rows();
        return (
          current && [
            {
              name: "Top",
              color: "#123456",
              tiles: current.map((row) => ({
                datumKey: row.key,
                label: row.key,
                value: metric === "revenue" ? row.revenue : row.visitors,
              })),
            },
          ]
        );
      }}
      metrics={REVENUE_OR_VISITORS}
      loading={rows() === undefined}
      loadingLabel="Loading…"
      emptyLabel={options.emptyLabel}
      sliceRankedBy={options.sliceRankedBy?.()}
      height={420}
    />
  ));
}

describe("Treemap", () => {
  it("sizes tiles by revenue, leaving zero-area tiles out", () => {
    renderTreemap(() => [
      { key: "a", revenue: 500, visitors: 1 },
      { key: "b", revenue: 0, visitors: 9 },
    ]);
    expect(seriesData()).toEqual([{ name: "Top", data: [["a", 500]] }]);
  });

  it("switches to visitors and back with its toggle", async () => {
    renderTreemap(() => [
      { key: "a", revenue: 500, visitors: 1 },
      { key: "b", revenue: 0, visitors: 9 },
    ]);
    await userEvent.click(screen.getByText("Visitors"));
    expect(seriesData()[0]?.data).toEqual([
      ["a", 1],
      ["b", 9],
    ]);
    await userEvent.click(screen.getByText("Revenue"));
    expect(seriesData()[0]?.data).toEqual([["a", 500]]);
  });

  it("falls back to visitors when there is no revenue", () => {
    renderTreemap(() => [{ key: "a", revenue: 0, visitors: 4 }]);
    expect(seriesData()[0]?.data).toEqual([["a", 4]]);
    expect(screen.getByText("Revenue").closest("[data-disabled]")).not.toBeNull();
  });

  it("switches every toggling treemap together", async () => {
    const rows = () => [
      { key: "a", revenue: 500, visitors: 1 },
      { key: "b", revenue: 0, visitors: 9 },
    ];
    renderTreemap(rows);
    renderTreemap(rows);
    await userEvent.click(screen.getAllByText("Visitors")[0] as HTMLElement);
    const visitors = [
      {
        name: "Top",
        data: [
          ["a", 1],
          ["b", 9],
        ],
      },
    ];
    expect(charts.map((chart) => seriesData(chart))).toEqual([visitors, visitors]);
    expect(rankBy()).toBe("traffic");
  });

  it("ranks everything by the fallback when it's picked", async () => {
    renderTreemap(() => [{ key: "a", revenue: 0, visitors: 4 }]);
    expect(rankBy()).toBe("revenue");
    await userEvent.click(screen.getByText("Visitors"));
    expect(rankBy()).toBe("traffic");
  });

  it("keeps the other metric on offer for a ranked slice that lacks it", async () => {
    setRankBy("traffic");
    const [rows, setRows] = createSignal<Row[]>([{ key: "a", revenue: 0, visitors: 4 }]);
    const [rankedBy, setRankedBy] = createSignal<TreemapMetric>("traffic");
    renderTreemap(rows, { sliceRankedBy: rankedBy });
    const revenueDisabled = () => screen.getByText("Revenue").closest("[data-disabled]") !== null;
    expect(revenueDisabled()).toBe(false);
    await userEvent.click(screen.getByText("Revenue"));
    // Still on offer while its slice loads, then judged by the slice ranked by it.
    expect(revenueDisabled()).toBe(false);
    batch(() => {
      setRows([{ key: "b", revenue: 0, visitors: 2 }]);
      setRankedBy("revenue");
    });
    expect(revenueDisabled()).toBe(true);
  });

  it("judges a slice by its own ranking when it mounts after the ranking changed", () => {
    renderTreemap(() => [{ key: "a", revenue: 0, visitors: 4 }], {
      sliceRankedBy: () => "traffic",
    });
    expect(rankBy()).toBe("revenue");
    expect(screen.getByText("Revenue").closest("[data-disabled]")).toBeNull();
  });

  it("takes no space when no tile has any area", () => {
    const { container } = renderTreemap(() => [{ key: "a", revenue: 0, visitors: 0 }]);
    expect(container).toBeEmptyDOMElement();
    expect(charts).toHaveLength(0);
  });

  it("says why it's empty when given a label", () => {
    renderTreemap(() => [], { emptyLabel: "Nothing here." });
    expect(screen.getByText("Nothing here.")).toBeInTheDocument();
    expect(screen.queryByText("Visitors")).toBeNull();
    expect(charts).toHaveLength(0);
  });

  it("draws tiles colored one by one as a single series, so each keeps its color", () => {
    render(() => (
      <Treemap
        groups={() => [
          { name: "A", color: "", tiles: [{ datumKey: "a", label: "a", value: 2 }] },
          { name: "B", color: "", tiles: [{ datumKey: "b", label: "b", value: 1 }] },
        ]}
        metrics={[{ value: "traffic", label: "Views" }]}
        loading={false}
        loadingLabel=""
        height={420}
        tileColor={(tile) => (tile.datumKey === "a" ? "#aaa" : "#bbb")}
      />
    ));
    expect(seriesData().map((series) => series.data)).toEqual([
      [
        ["a", 2],
        ["b", 1],
      ],
    ]);
    expect(lastChart().options.colors).toEqual(["#aaa", "#bbb"]);
  });

  it("draws once data arrives, and removes the chart when it empties", () => {
    const [rows, setRows] = createSignal<Row[] | undefined>(undefined);
    const { container } = renderTreemap(rows);
    expect(screen.getByText("Loading…")).toBeInTheDocument();
    expect(charts).toHaveLength(0);

    setRows([{ key: "a", revenue: 1, visitors: 1 }]);
    const chart = lastChart();
    expect(chart.rendered).toBe(true);

    setRows([{ key: "a", revenue: 2, visitors: 1 }]);
    expect(chart.history).toHaveLength(2);

    setRows([]);
    expect(chart.destroyed).toBe(true);
    expect(container).toBeEmptyDOMElement();
  });
});
