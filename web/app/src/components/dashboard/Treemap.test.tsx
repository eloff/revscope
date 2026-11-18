import { render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { createSignal } from "solid-js";
import { describe, expect, it, vi } from "vitest";
import { charts, lastChart, seriesData } from "../../../test/apexchartsMock";
import Treemap, { REVENUE_OR_VISITORS } from "./Treemap";
import type { TreemapMetric } from "./treemapLogic";

vi.mock("./apexcharts", () => import("../../../test/apexchartsMock"));

type Row = { key: string; revenue: number; visitors: number };

function renderTreemap(rows: () => Row[] | undefined, options: { emptyLabel?: string } = {}) {
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
