import { render, screen } from "@solidjs/testing-library";
import type { ApexOptions } from "apexcharts";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { lastChart } from "../../../test/apexchartsMock";
import { testBucket, testOverview } from "../../../test/dashboardFixtures";
import { setChartVisibility } from "../../lib/dashboardPreferences";
import type { ImportedRange } from "../../lib/importedCoverageApi";
import { createDefaultChartVisibility } from "./chartMetrics";
import { DashboardStateProvider } from "./dashboardPeriodContext";
import OverviewChart from "./OverviewChart";

type YAxes = Extract<NonNullable<ApexOptions["yaxis"]>, unknown[]>;

vi.mock("./apexcharts", () => import("../../../test/apexchartsMock"));

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
  props: { loading?: boolean; error?: string; importedRanges?: ImportedRange[] } = {},
) {
  return render(() => (
    <DashboardStateProvider>
      <OverviewChart
        data={data}
        loading={props.loading ?? false}
        error={props.error ?? ""}
        importedRanges={props.importedRanges}
      />
    </DashboardStateProvider>
  ));
}

type Series = { name: string; type: string; data: (number | null)[] };
const series = () => lastChart().options.series as unknown as Series[];

describe("OverviewChart", () => {
  it("draws the visitors line over stacked default revenue columns", () => {
    renderChart();
    expect(series().map((entry) => [entry.name, entry.type])).toEqual([
      ["Visitors", "line"],
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

  it("leaves today's visitors off the line until the day is over", () => {
    renderChart();
    expect(series()[0]?.data).toEqual([40, null]);
  });

  it("follows the chart preferences", () => {
    renderChart();
    setChartVisibility({
      ...createDefaultChartVisibility(),
      lineMetric: "bounce_rate",
      barEnabled: false,
    });
    expect(series()).toEqual([{ name: "Bounce rate", type: "line", data: [0.25, 0] }]);
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
    const custom = (lastChart().options.tooltip as { custom: (args: unknown) => string }).custom;
    const html = custom({
      dataPointIndex: 0,
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
    expect(tooltip.textContent).toContain("Visitors: 40 visitors");
    expect(tooltip.textContent).toContain("Recurring subscriptions: $30 (3)");
    expect(tooltip.textContent).toContain("Total revenue: $42");
  });

  it("shows loading and errors instead of the chart", () => {
    renderChart({ error: "Overview failed." });
    expect(screen.getByText("Overview failed.")).toBeInTheDocument();
    expect(lastChart().history).toHaveLength(1);
  });
});
