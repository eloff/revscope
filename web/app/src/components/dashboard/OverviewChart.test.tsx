import { render, screen } from "@solidjs/testing-library";
import type { ApexOptions } from "apexcharts";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { lastChart } from "../../../test/apexchartsMock";
import { testBucket, testOverview } from "../../../test/dashboardFixtures";
import type { OverviewResponse } from "../../lib/dashboardApi";
import { setChartVisibility } from "../../lib/dashboardPreferences";
import type { ImportedRange } from "../../lib/importedCoverageApi";
import { createDefaultChartVisibility } from "./chartMetrics";
import { DashboardStateProvider } from "./dashboardPeriodContext";
import OverviewChart from "./OverviewChart";

type YAxes = Extract<NonNullable<ApexOptions["yaxis"]>, unknown[]>;

vi.mock("./apexcharts", () => import("../../../test/apexchartsMock"));

const HOUR = 3_600;
const DAY = 86_400;
const TODAY = Date.parse("2026-03-15T00:00:00Z") / 1000;

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(new Date("2026-03-15T12:00:00Z"));
});

afterEach(() => {
  vi.useRealTimers();
  setChartVisibility(createDefaultChartVisibility());
});

const data = testOverview({
  granularity: "day",
  buckets: [
    testBucket({
      start: TODAY - DAY,
      visitors: 40,
      bounced_sessions: 10,
      recurring_subscriptions: 30_000,
      recurring_subscriptions_count: 3,
      new_subscriptions: 10_000,
      other_revenue: 5_000,
      churn_mrr: 2_000,
      refunds: -1_000,
    }),
    testBucket({ start: TODAY, visitors: 7, recurring_subscriptions: 1_000 }),
  ],
});

function renderChart(
  props: {
    data?: OverviewResponse;
    loading?: boolean;
    error?: string;
    importedRanges?: ImportedRange[];
    rangeEnd?: number;
  } = {},
) {
  return render(() => (
    <DashboardStateProvider>
      <OverviewChart
        data={props.data ?? data}
        loading={props.loading ?? false}
        error={props.error ?? ""}
        importedRanges={props.importedRanges}
        rangeEnd={props.rangeEnd}
      />
    </DashboardStateProvider>
  ));
}

type Series = { name: string; type: string; data: (number | null)[] };
const series = () => lastChart().options.series as unknown as Series[];

/** The line as drawn: its solid series, and the dashed one into the bucket in progress. */
function drawnLine(): { solid?: (number | null)[]; dashed?: (number | null)[] } {
  const dashes = lastChart().options.stroke?.dashArray as number[];
  const lines = series()
    .map((entry, index) => ({ ...entry, dash: dashes[index] ?? 0 }))
    .filter((entry) => entry.type === "line");
  return {
    solid: lines.find((entry) => entry.dash === 0)?.data,
    dashed: lines.find((entry) => entry.dash > 0)?.data,
  };
}

function tooltipText(dataPointIndex: number): string {
  const custom = (lastChart().options.tooltip as { custom: (args: unknown) => string }).custom;
  const html = custom({
    dataPointIndex,
    w: {
      globals: {
        series: series().map((entry) => entry.data),
        seriesNames: series().map((entry) => entry.name),
        colors: lastChart().options.colors,
      },
    },
  });
  const tooltip = document.createElement("div");
  tooltip.innerHTML = html;
  return tooltip.textContent ?? "";
}

describe("OverviewChart", () => {
  it("draws the visitors line over stacked default revenue columns", () => {
    renderChart();
    expect(series().map((entry) => [entry.name, entry.type])).toEqual([
      ["Visitors", "line"],
      ["Visitors (so far)", "line"],
      ["Recurring subscriptions", "column"],
      ["New subscriptions", "column"],
      ["New revenue", "column"],
    ]);
    expect(lastChart().options.chart?.stacked).toBe(true);
    expect((lastChart().options.yaxis as YAxes).map((axis) => axis.title?.text)).toEqual([
      "Visitors",
      "Revenue",
    ]);
  });

  it("adds no shading without imports", () => {
    renderChart();
    expect(lastChart().options.annotations?.xaxis).toEqual([]);
  });

  it("shades the buckets an import covers, naming them by unique categories", () => {
    renderChart({
      importedRanges: [{ start_date: "2026-03-14", end_date: "2026-03-15", source_name: "Acme" }],
    });
    const options = lastChart().options;
    const categories = options.xaxis?.categories as string[];
    expect(new Set(categories).size).toBe(categories.length);
    expect(options.annotations?.xaxis).toMatchObject([
      {
        x: categories[0],
        x2: categories[1],
        label: { text: "Imported from Google Analytics" },
      },
    ]);
  });

  it("dashes the line into today until the day is over", () => {
    renderChart();
    expect(drawnLine()).toEqual({ solid: [40, null], dashed: [40, 7] });
    const [lineAxis] = lastChart().options.yaxis as YAxes;
    expect(lineAxis?.seriesName).toEqual(["Visitors", "Visitors (so far)"]);
    expect(tooltipText(1)).toContain("(so far)");
    expect(tooltipText(1)).toContain("Visitors: 7 visitors");
    expect(tooltipText(1)).not.toContain("Visitors (so far):");
    expect(tooltipText(0)).not.toContain("(so far)");
  });

  it("dashes into the current hour and leaves the hours to come off the line", () => {
    const noon = TODAY + 12 * HOUR;
    renderChart({
      data: testOverview({
        granularity: "hour",
        buckets: [
          testBucket({ start: noon - HOUR, visitors: 5 }),
          testBucket({ start: noon, visitors: 2 }),
          testBucket({ start: noon + HOUR }),
        ],
      }),
    });
    expect(drawnLine()).toEqual({ solid: [5, null, null], dashed: [5, 2, null] });
  });

  it("plots a day without visitors as zero", () => {
    renderChart({
      data: testOverview({
        granularity: "day",
        buckets: [
          testBucket({ start: TODAY - 2 * DAY, visitors: 3 }),
          testBucket({ start: TODAY - DAY }),
          testBucket({ start: TODAY, visitors: 1 }),
        ],
      }),
    });
    expect(drawnLine()).toEqual({ solid: [3, 0, null], dashed: [null, 0, 1] });
  });

  it("marks a range that is all today", () => {
    renderChart({
      data: testOverview({
        granularity: "day",
        buckets: [testBucket({ start: TODAY, visitors: 7 })],
      }),
    });
    expect(drawnLine()).toEqual({ solid: [null], dashed: [7] });
    expect(tooltipText(0)).toContain("(so far)");
  });

  it("draws a range that has ended as complete", () => {
    renderChart({ rangeEnd: TODAY });
    expect(drawnLine()).toEqual({ solid: [40, 7], dashed: undefined });
    expect(tooltipText(1)).not.toContain("(so far)");
  });

  it("marks nothing in data left over from an earlier range", () => {
    renderChart({
      data: testOverview({
        granularity: "hour",
        buckets: [
          testBucket({ start: TODAY - 2 * HOUR, visitors: 5 }),
          testBucket({ start: TODAY - HOUR, visitors: 2 }),
        ],
      }),
    });
    expect(drawnLine()).toEqual({ solid: [5, 2], dashed: undefined });
  });

  it("follows the chart preferences", () => {
    renderChart();
    setChartVisibility({
      ...createDefaultChartVisibility(),
      lineMetric: "bounce_rate",
      barEnabled: false,
    });
    expect(series()).toEqual([
      { name: "Bounce rate", type: "line", data: [0.25, null] },
      { name: "Bounce rate (so far)", type: "line", data: [0.25, 0] },
    ]);
    expect(lastChart().options.chart?.stacked).toBe(false);

    setChartVisibility({
      lineMetric: null,
      barEnabled: true,
      barSeries: {
        mrr: false,
        new_subscriptions_mrr: false,
        new_revenue: false,
        churn_mrr: true,
        refunds_and_chargebacks: true,
      },
    });
    expect(series().map((entry) => entry.name)).toEqual(["Churned MRR", "Refunds", "Chargebacks"]);
    // Churn is drawn below zero; an empty bucket negates to -0, which plots as 0.
    expect(series()[0]?.data.map((value) => (value ?? 0) + 0)).toEqual([-2_000, 0]);
    const [revenueAxis] = lastChart().options.yaxis as YAxes;
    expect(revenueAxis?.min).toBeLessThan(0);
  });

  it("builds a tooltip with payment counts and the revenue total", () => {
    renderChart();
    const text = tooltipText(0);
    expect(text).toContain("Visitors: 40 visitors");
    expect(text).toContain("Recurring subscriptions: $30 (3)");
    expect(text).toContain("Total revenue: $42");
  });

  it("shows loading and errors instead of the chart", () => {
    renderChart({ error: "Overview failed." });
    expect(screen.getByText("Overview failed.")).toBeInTheDocument();
    expect(lastChart().history).toHaveLength(1);
  });
});
