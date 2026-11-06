/// <reference types="solid-js" />
import { createEffect, createMemo, onCleanup, onMount, Show } from "solid-js";
import type {
  SearchTermRow,
  SearchTermsPeriod,
  SearchTermsResponse,
} from "../../lib/searchTermsApi";
import ApexCharts, { type ApexOptions } from "./apexcharts";
import CursorHoverCard from "./CursorHoverCard";
import { chartMount, chartMountHidden } from "./chart.css";
import { createCursorHover } from "./createCursorHover";
import MetricHoverCardContent from "./MetricHoverCardContent";
import { message } from "./messages.css";
import { SOURCE_GROUP_COLORS } from "./revenueSourcesLogic";
import {
  buildSearchTermHoverContent,
  buildSearchTermOtherHoverContent,
  indexSearchTermsByQuery,
} from "./searchTermHoverDetails";
import {
  SEARCH_TERMS_OTHER_DATUM_KEY,
  searchTermRowHighlights,
  searchTermRowKey,
} from "./searchTermsLogic";
import { type TreemapHoverHandlers, treemapLeafEvents } from "./treemapLeafEvents";

const TREEMAP_TOP_TERMS = 50;
const OTHER_LABEL = "Other";
const TREEMAP_LABEL_FONT_SIZE = "12px";

type SearchTermTreemapLeaf = {
  label: string;
  datumKey: string;
  revenue: number;
  isOther: boolean;
};

type TreemapSeries = {
  name: string;
  data: { x: string; y: number }[];
};

type SeriesGroup = {
  name: string;
  leaves: SearchTermTreemapLeaf[];
  color: string;
};

type TreemapChartHandlers = TreemapHoverHandlers & {
  onLeafSelect?: (datumKey: string) => void;
};

type SearchTermsTreemapProps = {
  data: SearchTermsResponse | undefined;
  loading: boolean;
  onLeafSelect?: (datumKey: string) => void;
};

function compareTermsByTreemapRank(left: SearchTermRow, right: SearchTermRow): number {
  if (right.estimated_revenue !== left.estimated_revenue) {
    return right.estimated_revenue - left.estimated_revenue;
  }
  if (right.clicks !== left.clicks) {
    return right.clicks - left.clicks;
  }
  return left.query.localeCompare(right.query);
}

function topTreemapQueries(period: SearchTermsPeriod): Set<string> {
  return new Set(
    [...period.terms]
      .sort(compareTermsByTreemapRank)
      .slice(0, TREEMAP_TOP_TERMS)
      .map((term) => term.query),
  );
}

function buildSearchTermTreemapLeaves(period: SearchTermsPeriod): SearchTermTreemapLeaf[] {
  const topTerms = [...period.terms].sort(compareTermsByTreemapRank).slice(0, TREEMAP_TOP_TERMS);
  const topRevenue = topTerms.reduce((total, term) => total + term.estimated_revenue, 0);
  const otherRevenue = period.search_channel_revenue - topRevenue;
  // Zero-revenue tiles have no area; omitting them lets the chart collapse
  // when a period has search terms but no estimated revenue.
  const leaves: SearchTermTreemapLeaf[] = topTerms
    .filter((term) => term.estimated_revenue > 0)
    .map((term) => ({
      label: term.query,
      datumKey: searchTermRowKey(term),
      revenue: term.estimated_revenue,
      isOther: false,
    }));
  if (otherRevenue > 0) {
    leaves.push({
      label: OTHER_LABEL,
      datumKey: SEARCH_TERMS_OTHER_DATUM_KEY,
      revenue: otherRevenue,
      isOther: true,
    });
  }
  return leaves;
}

function buildLabelByDatumKey(leaves: SearchTermTreemapLeaf[]): Map<string, string> {
  const labelByDatumKey = new Map<string, string>();
  for (const leaf of leaves) {
    labelByDatumKey.set(leaf.datumKey, leaf.label);
  }
  return labelByDatumKey;
}

function buildSeriesGroups(leaves: SearchTermTreemapLeaf[]): SeriesGroup[] {
  const termLeaves = leaves.filter((leaf) => !leaf.isOther);
  const otherLeaves = leaves.filter((leaf) => leaf.isOther);
  return [
    {
      name: "Top by revenue",
      leaves: termLeaves,
      color: SOURCE_GROUP_COLORS.search,
    },
    {
      name: OTHER_LABEL,
      leaves: otherLeaves,
      color: SOURCE_GROUP_COLORS.referrers,
    },
  ].filter((group) => group.leaves.length > 0);
}

function buildOptions(
  leaves: SearchTermTreemapLeaf[],
  labelByDatumKey: Map<string, string>,
  handlers?: TreemapChartHandlers,
): ApexOptions {
  const groups = buildSeriesGroups(leaves);
  const series: TreemapSeries[] = groups.map((group) => ({
    name: group.name,
    data: group.leaves.map((leaf) => ({
      x: leaf.datumKey,
      y: Math.max(0, leaf.revenue),
    })),
  }));
  const seriesLeaves = groups.map((group) => group.leaves);
  const onLeafSelect = handlers?.onLeafSelect;
  return {
    chart: {
      type: "treemap",
      height: 420,
      toolbar: { show: false },
      fontFamily: "Inter, Roboto, 'Helvetica Neue', Arial, sans-serif",
      events: treemapLeafEvents({
        seriesLeaves: seriesLeaves,
        hover: handlers,
        onSelect: onLeafSelect ? (leaf) => onLeafSelect(leaf.datumKey) : undefined,
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

export default function SearchTermsTreemap(props: SearchTermsTreemapProps) {
  let chartElement: HTMLDivElement | undefined;
  let chart: ApexCharts | undefined;
  const hover = createCursorHover();

  const leaves = createMemo(() =>
    props.data ? buildSearchTermTreemapLeaves(props.data.current) : undefined,
  );

  const termByKey = createMemo(() => {
    const response = props.data;
    if (!response) {
      return new Map<string, SearchTermRow>();
    }
    return indexSearchTermsByQuery(response.current.terms);
  });

  const topQueries = createMemo(() =>
    props.data ? topTreemapQueries(props.data.current) : new Set<string>(),
  );

  const hoveredTitle = createMemo(() => {
    const key = hover.key();
    if (!key) {
      return undefined;
    }
    if (key === SEARCH_TERMS_OTHER_DATUM_KEY) {
      return OTHER_LABEL;
    }
    return termByKey().get(key)?.query;
  });

  const hoverContent = createMemo(() => {
    const key = hover.key();
    const response = props.data;
    if (!key || !response) {
      return undefined;
    }
    if (key === SEARCH_TERMS_OTHER_DATUM_KEY) {
      return buildSearchTermOtherHoverContent(response.current, topQueries(), TREEMAP_TOP_TERMS);
    }
    const term = termByKey().get(key);
    if (!term) {
      return undefined;
    }
    return buildSearchTermHoverContent(term, searchTermRowHighlights(term));
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
      buildOptions(currentLeaves, buildLabelByDatumKey(currentLeaves), {
        onLeafSelect: props.onLeafSelect,
        onLeafHover: hover.show,
        onChartMouseMove: hover.move,
      }),
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
          <p class={message}>Loading search terms treemap…</p>
        </Show>
        <div
          ref={chartElement}
          style={{
            display: props.loading || (leaves()?.length ?? 0) === 0 ? "none" : "block",
          }}
        />
      </div>
      <CursorHoverCard hover={hover} open={hoveredTitle() !== undefined}>
        <Show when={hoveredTitle()}>
          {(title) => (
            <MetricHoverCardContent
              title={title()}
              content={hoverContent()}
              statLayout="threeColumn"
            />
          )}
        </Show>
      </CursorHoverCard>
    </>
  );
}
