import { render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { describe, expect, it, vi } from "vitest";
import { lastChart, seriesData } from "../../../test/apexchartsMock";
import AudienceBreakdownTreemap from "./AudienceBreakdownTreemap";
import type { AudienceBreakdownRow } from "./audienceLogic";

vi.mock("./apexcharts", () => import("../../../test/apexchartsMock"));

function row(key: string, revenue: number): AudienceBreakdownRow {
  return {
    key,
    name: `Name ${key}`,
    visitors: 1,
    convertingVisitors: 0,
    revenue,
    revenuePerVisitor: 0,
  };
}

describe("AudienceBreakdownTreemap", () => {
  it("draws revenue tiles labelled by name, without zero-revenue rows", () => {
    render(() => (
      <AudienceBreakdownTreemap
        rows={[row("chrome", 500), row("safari", 300), row("lynx", 0)]}
        topGroupColor="#123456"
        otherTooltipNoun="browsers"
        loading={false}
      />
    ));
    const chart = lastChart();
    expect(chart.rendered).toBe(true);
    expect(seriesData(chart)).toEqual([
      {
        name: "Top by revenue",
        data: [
          ["chrome", 500],
          ["safari", 300],
        ],
      },
    ]);
    expect(chart.options.colors).toEqual(["#123456"]);
    const formatter = chart.options.dataLabels?.formatter as (text: string) => string;
    expect(formatter("chrome")).toBe("Name chrome");
  });

  it("reports selection and hover for real tiles only", () => {
    const rows = Array.from({ length: 52 }, (_, index) => row(`r${index}`, 1000 - index));
    const onLeafSelect = vi.fn();
    const onLeafHover = vi.fn();
    render(() => (
      <AudienceBreakdownTreemap
        rows={rows}
        topGroupColor="#000"
        otherTooltipNoun="cities"
        loading={false}
        onLeafSelect={onLeafSelect}
        onLeafHover={onLeafHover}
      />
    ));
    const chart = lastChart();
    expect(seriesData(chart).map((series) => series.name)).toEqual(["Top by revenue", "Other"]);

    chart.fire("dataPointSelection", { seriesIndex: 0, dataPointIndex: 1 });
    chart.fire("dataPointSelection", { seriesIndex: 1, dataPointIndex: 0 });
    expect(onLeafSelect.mock.calls).toEqual([["r1"]]);

    chart.fire(
      "dataPointMouseEnter",
      { seriesIndex: 0, dataPointIndex: 0 },
      { clientX: 5, clientY: 6 },
    );
    chart.fire("dataPointMouseLeave");
    expect(onLeafHover.mock.calls).toEqual([
      ["r0", 5, 6],
      [null, 0, 0],
    ]);
  });

  it("waits for data and hides itself when there is none", () => {
    const [loading, setLoading] = createSignal(true);
    const [rows, setRows] = createSignal<AudienceBreakdownRow[]>([]);
    const { container } = render(() => (
      <AudienceBreakdownTreemap
        rows={rows()}
        topGroupColor="#000"
        otherTooltipNoun="x"
        loading={loading()}
      />
    ));
    expect(screen.getByText("Loading breakdown treemap…")).toBeInTheDocument();
    expect(lastChart().history).toHaveLength(1);

    setLoading(false);
    expect(lastChart().history).toHaveLength(1);
    expect(container.firstElementChild?.className).toMatch(/Hidden/);

    setRows([row("a", 1)]);
    expect(lastChart().history).toHaveLength(2);
  });

  it("destroys the chart on unmount", () => {
    const { unmount } = render(() => (
      <AudienceBreakdownTreemap
        rows={[]}
        topGroupColor="#000"
        otherTooltipNoun="x"
        loading={false}
      />
    ));
    const chart = lastChart();
    unmount();
    expect(chart.destroyed).toBe(true);
  });
});
