/// <reference types="solid-js" />

import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import {
  CRAWLER_LAST_3_DAYS_BUCKET_SECONDS,
  CRAWLER_LAST_3_DAYS_BUCKETS,
  type CrawlerDetailResponse,
  type CrawlerPageRow,
  last3DaysValues,
} from "../../lib/crawlersApi";
import { formatBucketLabel } from "../../lib/dateRange";
import ApexCharts, { type ApexOptions } from "./apexcharts";
import {
  crawlerDetailChartMount,
  crawlerPageSparklineCell,
  crawlerPageSparklineHeader,
  crawlerPageViewsCell,
} from "./CrawlerDetailPanel.css";
import { nextColumnSort } from "./columnSort";
import { useDashboardState } from "./dashboardPeriodContext";
import GridColumnInfoIcon from "./GridColumnInfoIcon";
import { gridHeaderLabel, gridNumberCell, gridSortHeaderCell, gridTable } from "./grid.css";
import { message } from "./messages.css";
import SortableHeaderCell from "./SortableHeaderCell";
import Sparkline from "./Sparkline";

type PageSortColumn = "page" | "views";
type PageSortState = { column: PageSortColumn; direction: "asc" | "desc" };

const DEFAULT_PAGE_SORT: PageSortState = { column: "views", direction: "desc" };

function sparklineLabels(start: number, buckets: number, timeZone: string): string[] {
  return Array.from({ length: buckets }, (_, index) =>
    formatBucketLabel(start + index * CRAWLER_LAST_3_DAYS_BUCKET_SECONDS, "hour", timeZone),
  );
}

type CrawlerDetailPanelProps = {
  data: CrawlerDetailResponse | undefined;
  loading: boolean;
  error: string;
};

function pageLabel(row: CrawlerPageRow): string {
  return `${row.hostname}${row.path}`;
}

function nextPageSort(sort: PageSortState, column: PageSortColumn): PageSortState {
  return nextColumnSort(sort, column, column === "views" ? "desc" : "asc");
}

function sortPages(pages: CrawlerPageRow[], sort: PageSortState): CrawlerPageRow[] {
  const ranked = [...pages];
  ranked.sort((left, right) => {
    const direction = sort.direction === "asc" ? 1 : -1;
    if (sort.column === "page") {
      const comparison = pageLabel(left).localeCompare(pageLabel(right));
      if (comparison !== 0) {
        return comparison * direction;
      }
    } else if (left.views !== right.views) {
      return (left.views - right.views) * direction;
    }
    if (right.views !== left.views) {
      return right.views - left.views;
    }
    return pageLabel(left).localeCompare(pageLabel(right));
  });
  return ranked;
}

function buildChartOptions(data: CrawlerDetailResponse, timeZone: string): ApexOptions {
  return {
    chart: {
      type: "line",
      height: 260,
      toolbar: { show: false },
      fontFamily: "Inter, Roboto, 'Helvetica Neue', Arial, sans-serif",
    },
    stroke: {
      width: 2,
      curve: "smooth",
    },
    dataLabels: { enabled: false },
    xaxis: {
      categories: data.series.map((point) =>
        formatBucketLabel(point.start, data.granularity, timeZone),
      ),
      labels: {
        rotate: -35,
        hideOverlappingLabels: true,
      },
    },
    yaxis: {
      min: 0,
      labels: {
        formatter: (value) => Math.round(value).toLocaleString(),
      },
    },
    colors: ["#0891b2"],
    legend: { show: false },
    series: [
      {
        name: "Views",
        data: data.series.map((point) => point.views),
      },
    ],
  };
}

export default function CrawlerDetailPanel(props: CrawlerDetailPanelProps) {
  const { timeZone } = useDashboardState();
  const [pageSort, setPageSort] = createSignal<PageSortState>(DEFAULT_PAGE_SORT);
  let chartElement: HTMLDivElement | undefined;
  let chart: ApexCharts | undefined;

  const pages = createMemo(() => (props.data ? sortPages(props.data.pages, pageSort()) : []));
  const hasSeries = () => (props.data?.series.length ?? 0) > 0;
  const sparkCategories = createMemo(() => {
    const data = props.data;
    if (!data) {
      return [];
    }
    return sparklineLabels(data.last_3_days_start, CRAWLER_LAST_3_DAYS_BUCKETS, timeZone());
  });

  onMount(() => {
    if (!chartElement) {
      return;
    }
    chart = new ApexCharts(chartElement, {
      chart: { type: "line", height: 260 },
      series: [],
    });
    chart.render();
  });

  createEffect(() => {
    if (!chart || props.loading || !props.data) {
      return;
    }
    chart.updateOptions(buildChartOptions(props.data, timeZone()), true, true);
  });

  onCleanup(() => {
    chart?.destroy();
    chart = undefined;
  });

  const showChart = () => !props.loading && !props.error && hasSeries();

  return (
    <>
      <Show when={props.error}>
        <p class={message}>{props.error}</p>
      </Show>
      <Show when={!props.error && props.loading}>
        <p class={message}>Loading crawler details…</p>
      </Show>
      <Show when={!props.error && !props.loading && props.data && !hasSeries()}>
        <p class={message}>No views over this period.</p>
      </Show>
      <div class={crawlerDetailChartMount} style={{ display: showChart() ? "block" : "none" }}>
        <div ref={chartElement} />
      </div>
      <Show when={!props.error && !props.loading && props.data}>
        <table class={gridTable}>
          <thead>
            <tr>
              <th class={crawlerPageSparklineHeader}>
                <span class={gridSortHeaderCell}>
                  <span class={gridHeaderLabel}>Last 3 days</span>
                  <GridColumnInfoIcon
                    title="Last 3 days"
                    description="View counts in 4-hour buckets over the 3 days ending at this crawler's latest view in the selected range."
                  />
                </span>
              </th>
              <SortableHeaderCell
                label="Views"
                sortDirection={pageSort().column === "views" ? pageSort().direction : undefined}
                numeric
                class={crawlerPageViewsCell}
                onSort={() => setPageSort((current) => nextPageSort(current, "views"))}
              />
              <SortableHeaderCell
                label="Page"
                sortDirection={pageSort().column === "page" ? pageSort().direction : undefined}
                onSort={() => setPageSort((current) => nextPageSort(current, "page"))}
              />
            </tr>
          </thead>
          <tbody>
            <Show
              when={pages().length > 0}
              fallback={
                <tr>
                  <td colSpan={3}>No pages for this crawler.</td>
                </tr>
              }
            >
              <For each={pages()}>
                {(row) => (
                  <tr>
                    <td class={crawlerPageSparklineCell}>
                      <Sparkline
                        values={last3DaysValues(row.last_3_days)}
                        categories={sparkCategories()}
                      />
                    </td>
                    <td class={`${gridNumberCell} ${crawlerPageViewsCell}`}>
                      {row.views.toLocaleString()}
                    </td>
                    <td title={pageLabel(row)}>{pageLabel(row)}</td>
                  </tr>
                )}
              </For>
            </Show>
          </tbody>
        </table>
      </Show>
    </>
  );
}
