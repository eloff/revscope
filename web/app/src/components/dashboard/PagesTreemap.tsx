/// <reference types="solid-js" />
import { createEffect, createMemo, onCleanup, onMount, Show } from "solid-js";
import type { PageRow, PagesResponse } from "../../lib/pagesApi";
import ApexCharts, { type ApexOptions } from "./apexcharts";
import CursorHoverCard from "./CursorHoverCard";
import { chartMount, chartMountHidden } from "./chart.css";
import { createCursorHover } from "./createCursorHover";
import MetricHoverCardContent from "./MetricHoverCardContent";
import { message } from "./messages.css";
import { buildPageHoverContent, type PagesVariant } from "./pageHoverDetails";
import { pageRowHighlights } from "./pagesLogic";
import { SOURCE_GROUP_COLORS } from "./revenueSourcesLogic";
import { type TreemapHoverHandlers, treemapLeafEvents } from "./treemapLeafEvents";

const TREEMAP_TOP_PAGES = 50;
const OTHER_DATUM_KEY = "__pages_other__";
const OTHER_LABEL = "Other";
const TREEMAP_LABEL_FONT_SIZE = "12px";

type PageTreemapLeaf = {
  label: string;
  tooltip: string;
  datumKey: string;
  attributed_revenue: number;
  visitors: number | null;
  isOther: boolean;
};

type TreemapSeries = {
  name: string;
  data: { x: string; y: number }[];
};

type SeriesGroup = {
  name: string;
  leaves: PageTreemapLeaf[];
  color: string;
};

type TreemapChartHandlers = TreemapHoverHandlers & {
  onLeafSelect?: (datumKey: string) => void;
};

type PagesTreemapProps = {
  data: PagesResponse | undefined;
  variant?: PagesVariant;
  loading: boolean;
  onLeafSelect?: (datumKey: string) => void;
  topGroupColor?: string;
  labels?: {
    loading: string;
    otherTooltipNoun: string;
  };
};

/** Stable key shared between the treemap leaf and the grid row for a page. */
export function pageRowKeyFromId(id: number): string {
  return `page:${id}`;
}

/** Stable key shared between the treemap leaf and the grid row for a page. */
export function pageRowKey(page: Pick<PageRow, "id">): string {
  return pageRowKeyFromId(page.id);
}

/** Display URL for a page (hostname + path). */
export function pageUrl(page: Pick<PageRow, "path" | "hostname">): string {
  return `${page.hostname}${page.path}`;
}

function comparePagesByTreemapRank(left: PageRow, right: PageRow): number {
  if (right.attributed_revenue !== left.attributed_revenue) {
    return right.attributed_revenue - left.attributed_revenue;
  }
  if (right.visitors !== left.visitors) {
    return right.visitors - left.visitors;
  }
  return pageUrl(left).localeCompare(pageUrl(right));
}

function buildPageByKey(pages: PageRow[]): Map<string, PageRow> {
  const map = new Map<string, PageRow>();
  for (const page of pages) {
    map.set(pageRowKey(page), page);
  }
  return map;
}

function buildPageTreemapLeaves(pages: PageRow[], otherTooltipNoun: string): PageTreemapLeaf[] {
  const ranked = [...pages].sort(comparePagesByTreemapRank);
  const topPages = ranked.slice(0, TREEMAP_TOP_PAGES);
  const otherPages = ranked.slice(TREEMAP_TOP_PAGES);
  const otherAttributedRevenue = otherPages.reduce(
    (total, page) => total + page.attributed_revenue,
    0,
  );
  const leaves: PageTreemapLeaf[] = topPages.map((page) => ({
    label: page.path,
    tooltip: pageUrl(page),
    datumKey: pageRowKey(page),
    attributed_revenue: page.attributed_revenue,
    visitors: page.visitors,
    isOther: false,
  }));
  leaves.push({
    label: OTHER_LABEL,
    tooltip: `Attributed revenue from ${otherPages.length.toLocaleString()} ${otherTooltipNoun} outside the top ${TREEMAP_TOP_PAGES}`,
    datumKey: OTHER_DATUM_KEY,
    attributed_revenue: otherAttributedRevenue,
    visitors: null,
    isOther: true,
  });
  return leaves;
}

function buildLabelByDatumKey(leaves: PageTreemapLeaf[]): Map<string, string> {
  const labelByDatumKey = new Map<string, string>();
  for (const leaf of leaves) {
    labelByDatumKey.set(leaf.datumKey, leaf.label);
  }
  return labelByDatumKey;
}

function buildSeriesGroups(
  leaves: PageTreemapLeaf[],
  topGroupName: string,
  topGroupColor: string,
): SeriesGroup[] {
  const pageLeaves = leaves.filter((leaf) => !leaf.isOther);
  const otherLeaves = leaves.filter((leaf) => leaf.isOther);
  return [
    {
      name: topGroupName,
      leaves: pageLeaves,
      color: topGroupColor,
    },
    {
      name: OTHER_LABEL,
      leaves: otherLeaves,
      color: SOURCE_GROUP_COLORS.direct,
    },
  ].filter((group) => group.leaves.length > 0);
}

function buildOptions(
  leaves: PageTreemapLeaf[],
  labelByDatumKey: Map<string, string>,
  topGroupName: string,
  topGroupColor: string,
  handlers?: TreemapChartHandlers,
): ApexOptions {
  const groups = buildSeriesGroups(leaves, topGroupName, topGroupColor);
  const series: TreemapSeries[] = groups.map((group) => ({
    name: group.name,
    data: group.leaves.map((leaf) => ({
      x: leaf.datumKey,
      y: Math.max(0, leaf.attributed_revenue),
    })),
  }));
  const seriesLeaves = groups.map((group) => group.leaves);
  return {
    chart: {
      type: "treemap",
      height: 420,
      toolbar: { show: false },
      fontFamily: "Inter, Roboto, 'Helvetica Neue', Arial, sans-serif",
      events: treemapLeafEvents({
        seriesLeaves: seriesLeaves,
        hover: handlers,
        hoverable: (leaf) => !leaf.isOther,
        onSelect: handlers?.onLeafSelect
          ? (leaf) => handlers.onLeafSelect?.(leaf.datumKey)
          : undefined,
        selectable: (leaf) => !leaf.isOther,
      }),
    },
    series: series as ApexOptions["series"],
    colors: groups.map((group) => group.color),
    legend: {
      show: false,
    },
    dataLabels: {
      enabled: true,
      style: { fontSize: TREEMAP_LABEL_FONT_SIZE },
      formatter: (text) => labelByDatumKey.get(String(text)) ?? String(text),
    },
    plotOptions: {
      treemap: {
        distributed: false,
        enableShades: true,
        dataLabels: {
          format: "truncate",
        },
      },
    },
    tooltip: {
      enabled: false,
    },
  };
}

export default function PagesTreemap(props: PagesTreemapProps) {
  let chartElement: HTMLDivElement | undefined;
  let chart: ApexCharts | undefined;
  const hover = createCursorHover();
  const variant = () => props.variant ?? "pages";
  const labels = () =>
    props.labels ?? {
      loading: "Loading pages treemap…",
      otherTooltipNoun: "pages",
    };

  const pageByKey = createMemo(() =>
    props.data ? buildPageByKey(props.data.pages) : new Map<string, PageRow>(),
  );

  const leaves = createMemo(() =>
    props.data ? buildPageTreemapLeaves(props.data.pages, labels().otherTooltipNoun) : undefined,
  );

  const hoveredPage = createMemo(() => {
    const key = hover.key();
    return key ? pageByKey().get(key) : undefined;
  });

  const hoverContent = createMemo(() => {
    const page = hoveredPage();
    if (!page) {
      return undefined;
    }
    return buildPageHoverContent(page, variant(), pageRowHighlights(page, variant()));
  });

  onMount(() => {
    if (!chartElement) {
      return;
    }
    chart = new ApexCharts(chartElement, {
      chart: { type: "treemap", height: 420 },
      series: [],
    });
    chart.render();
  });

  const showTreemap = () => props.loading || (leaves()?.length ?? 0) > 0;

  createEffect(() => {
    if (!chart || props.loading) {
      return;
    }
    const currentLeaves = leaves();
    if (!currentLeaves || currentLeaves.length === 0) {
      return;
    }
    chart.updateOptions(
      buildOptions(
        currentLeaves,
        buildLabelByDatumKey(currentLeaves),
        "Top by revenue",
        props.topGroupColor ?? SOURCE_GROUP_COLORS.social,
        {
          onLeafSelect: props.onLeafSelect,
          onLeafHover: hover.show,
          onChartMouseMove: hover.move,
        },
      ),
      true,
      true,
    );
  });

  onCleanup(() => {
    chart?.destroy();
    chart = undefined;
  });

  return (
    <>
      <div class={showTreemap() ? chartMount : `${chartMount} ${chartMountHidden}`}>
        <Show when={props.loading}>
          <p class={message}>{labels().loading}</p>
        </Show>
        <div
          ref={chartElement}
          style={{
            display: props.loading || (leaves()?.length ?? 0) === 0 ? "none" : "block",
          }}
        />
      </div>
      <CursorHoverCard hover={hover} open={hoveredPage() !== undefined} wide>
        <Show when={hoveredPage()}>
          {(page) => (
            <MetricHoverCardContent
              title={pageUrl(page())}
              content={hoverContent()}
              statLayout="twoColumn"
              showPeriodComparisonNote={false}
            />
          )}
        </Show>
      </CursorHoverCard>
    </>
  );
}
