import type { ApexOptions } from "apexcharts";
import { beforeEach } from "vitest";

/**
 * Stands in for `src/components/dashboard/apexcharts.ts`; jsdom can't lay out
 * SVG, so tests assert on the options a component hands the chart instead.
 *
 *   vi.mock("./apexcharts", () => import("../../../test/apexchartsMock"));
 */
export class FakeApexCharts {
  element: Element;
  /** Every options object passed in, in order: the constructor's first. */
  history: ApexOptions[];
  rendered = false;
  destroyed = false;

  constructor(element: Element, options: ApexOptions) {
    this.element = element;
    this.history = [options];
    charts.push(this);
  }

  get options(): ApexOptions {
    return this.history.at(-1) as ApexOptions;
  }

  render(): Promise<void> {
    this.rendered = true;
    return Promise.resolve();
  }

  updateOptions(options: ApexOptions): Promise<void> {
    this.history.push(options);
    return Promise.resolve();
  }

  updateSeries(series: ApexOptions["series"]): Promise<void> {
    this.history.push({ ...this.options, series });
    return Promise.resolve();
  }

  destroy(): void {
    this.destroyed = true;
  }

  /** Calls a `chart.events` handler from the latest options, as ApexCharts would. */
  fire(
    name: keyof NonNullable<NonNullable<ApexOptions["chart"]>["events"]>,
    config?: { seriesIndex: number; dataPointIndex: number },
    event: Partial<MouseEvent> = { clientX: 10, clientY: 20 },
  ): void {
    const handler = this.options.chart?.events?.[name] as
      | ((event: unknown, context: unknown, config: unknown) => void)
      | undefined;
    handler?.(event, this, config);
  }
}

export const charts: FakeApexCharts[] = [];

export function lastChart(): FakeApexCharts {
  const chart = charts.at(-1);
  if (!chart) {
    throw new Error("No chart was created");
  }
  return chart;
}

/** `[x, y]` pairs per series of the latest options. */
export function seriesData(chart = lastChart()): { name?: string; data: [unknown, unknown][] }[] {
  return ((chart.options.series ?? []) as { name?: string; data: unknown[] }[]).map((series) => ({
    name: series.name,
    data: series.data.map((point) =>
      Array.isArray(point)
        ? (point as [unknown, unknown])
        : [(point as { x: unknown }).x, (point as { y: unknown }).y],
    ),
  }));
}

beforeEach(() => {
  charts.length = 0;
});

export default FakeApexCharts;
export type { ApexOptions };
