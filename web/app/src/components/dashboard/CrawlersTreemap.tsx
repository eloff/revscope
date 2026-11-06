/// <reference types="solid-js" />
import { createEffect, createMemo, onCleanup, onMount, Show } from "solid-js";
import { formatPercentRatio } from "../../lib/dashboardApi";
import ApexCharts, { type ApexOptions } from "./apexcharts";
import CursorHoverCard from "./CursorHoverCard";
import { chartMount } from "./chart.css";
import { CRAWLERS_TREEMAP_TOP_OPERATORS, type CrawlerOperatorLeaf } from "./crawlersLogic";
import { createCursorHover } from "./createCursorHover";
import MetricHoverCardContent from "./MetricHoverCardContent";
import { message } from "./messages.css";
import type { MetricHoverContent } from "./metricHoverDetails";
import { type TreemapHoverHandlers, treemapLeafEvents } from "./treemapLeafEvents";

const TREEMAP_LABEL_FONT_SIZE = "12px";

type CrawlersTreemapProps = {
  leaves: CrawlerOperatorLeaf[] | undefined;
  loading: boolean;
  onLeafSelect?: (key: string) => void;
};

function buildHoverContent(leaf: CrawlerOperatorLeaf): MetricHoverContent {
  return {
    ...(leaf.isOther
      ? {
          description: `${leaf.otherCount.toLocaleString()} operators outside the top ${CRAWLERS_TREEMAP_TOP_OPERATORS}`,
        }
      : {}),
    stats: [
      {
        label: "Views",
        value: leaf.views.toLocaleString(),
      },
      {
        label: "Share",
        value: formatPercentRatio(leaf.share, 1),
      },
    ],
  };
}

function buildOptions(
  leaves: CrawlerOperatorLeaf[],
  handlers?: TreemapHoverHandlers & {
    onLeafSelect?: (leaf: CrawlerOperatorLeaf) => void;
  },
): ApexOptions {
  return {
    chart: {
      type: "treemap",
      height: 420,
      toolbar: { show: false },
      fontFamily: "Inter, Roboto, 'Helvetica Neue', Arial, sans-serif",
      events: treemapLeafEvents({
        seriesLeaves: [leaves],
        hover: handlers,
        onSelect: handlers?.onLeafSelect,
        selectable: (leaf) => !leaf.isOther,
      }),
    },
    series: [
      {
        data: leaves.map((leaf) => ({
          x: leaf.datumKey,
          y: Math.max(0, leaf.views),
        })),
      },
    ] as ApexOptions["series"],
    colors: leaves.map((leaf) => leaf.color),
    legend: {
      show: false,
    },
    dataLabels: {
      enabled: true,
      style: { fontSize: TREEMAP_LABEL_FONT_SIZE },
      formatter: (text) => {
        const leaf = leaves.find((entry) => entry.datumKey === String(text));
        return leaf?.label ?? String(text);
      },
    },
    plotOptions: {
      treemap: {
        distributed: true,
        enableShades: false,
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

export default function CrawlersTreemap(props: CrawlersTreemapProps) {
  let chartElement: HTMLDivElement | undefined;
  let chart: ApexCharts | undefined;
  const hover = createCursorHover();

  const leafByKey = createMemo(() => {
    const map = new Map<string, CrawlerOperatorLeaf>();
    for (const leaf of props.leaves ?? []) {
      map.set(leaf.datumKey, leaf);
    }
    return map;
  });
  const hoveredLeaf = createMemo(() => {
    const key = hover.key();
    return key ? leafByKey().get(key) : undefined;
  });
  const hoverContent = createMemo(() => {
    const leaf = hoveredLeaf();
    return leaf ? buildHoverContent(leaf) : undefined;
  });
  const hasLeaves = () => (props.leaves?.length ?? 0) > 0;

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

  createEffect(() => {
    if (!chart || props.loading) {
      return;
    }
    const leaves = props.leaves;
    if (!leaves) {
      return;
    }
    chart.updateOptions(
      buildOptions(leaves, {
        onLeafSelect: (leaf) => props.onLeafSelect?.(leaf.datumKey),
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
      <div class={chartMount}>
        <Show when={props.loading}>
          <p class={message}>Loading crawlers treemap…</p>
        </Show>
        <Show when={!props.loading && props.leaves && !hasLeaves()}>
          <p class={message}>No crawler views for the selected categories.</p>
        </Show>
        <div
          ref={chartElement}
          style={{
            display: props.loading || !props.leaves || !hasLeaves() ? "none" : "block",
          }}
        />
      </div>
      <CursorHoverCard hover={hover} open={hoveredLeaf() !== undefined} wide>
        <Show when={hoveredLeaf()}>
          {(leaf) => (
            <MetricHoverCardContent
              title={leaf().label}
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
