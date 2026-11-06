/// <reference types="solid-js" />
import { createEffect, createMemo, onCleanup, onMount, Show } from "solid-js";
import ApexCharts, { type ApexOptions } from "./apexcharts";
import CursorHoverCard from "./CursorHoverCard";
import { chartMount, chartMountHidden } from "./chart.css";
import { createCursorHover } from "./createCursorHover";
import MetricHoverCardContent from "./MetricHoverCardContent";
import { errorMessage, message } from "./messages.css";
import { buildBestSourceHoverContent } from "./revenueSourceHoverDetails";
import {
  type BestSourceHighlights,
  bestSourceRowHighlights,
  type RevenueSourceRecord,
  SOURCE_GROUP_COLORS,
  SOURCE_GROUP_LABELS,
  SOURCE_GROUP_ORDER,
  type SourceGroup,
  sourceRowHasBestHighlight,
  type TreemapLeaf,
} from "./revenueSourcesLogic";
import { type TreemapHoverHandlers, treemapLeafEvents } from "./treemapLeafEvents";

type RevenueSourcesTreemapProps = {
  leaves: Map<SourceGroup, TreemapLeaf[]> | undefined;
  recordByKey: Map<string, RevenueSourceRecord>;
  bestHighlights: BestSourceHighlights;
  loading: boolean;
  error: string;
  onLeafSelect?: (datumKey: string) => void;
};

/** Fixed treemap tile label size; use truncate format so long URLs are clipped, not scaled down. */
const TREEMAP_LABEL_FONT_SIZE = "12px";

type TreemapSeries = {
  name: string;
  data: { x: string; y: number }[];
};

type TreemapChartHandlers = TreemapHoverHandlers & {
  onLeafSelect?: (datumKey: string) => void;
};

function rowHasBestHighlight(
  record: RevenueSourceRecord,
  highlights: BestSourceHighlights,
): boolean {
  return sourceRowHasBestHighlight(record.key, highlights);
}

function buildLabelByDatumKey(leaves: Map<SourceGroup, TreemapLeaf[]>): Map<string, string> {
  const labelByDatumKey = new Map<string, string>();
  for (const group of SOURCE_GROUP_ORDER) {
    for (const leaf of leaves.get(group) ?? []) {
      labelByDatumKey.set(leaf.datumKey, leaf.label);
    }
  }
  return labelByDatumKey;
}

function buildSeries(leaves: Map<SourceGroup, TreemapLeaf[]>): {
  series: TreemapSeries[];
  colors: string[];
} {
  const series: TreemapSeries[] = [];
  const colors: string[] = [];
  for (const group of SOURCE_GROUP_ORDER) {
    const groupLeaves = leaves.get(group) ?? [];
    if (groupLeaves.length === 0) {
      continue;
    }
    series.push({
      name: SOURCE_GROUP_LABELS[group],
      data: groupLeaves.map((leaf) => ({
        x: leaf.datumKey,
        y: leaf.revenue,
      })),
    });
    colors.push(SOURCE_GROUP_COLORS[group]);
  }
  return { series, colors };
}

function treemapHasLeaves(
  leaves: Map<SourceGroup, TreemapLeaf[]> | undefined,
): leaves is Map<SourceGroup, TreemapLeaf[]> {
  if (!leaves) {
    return false;
  }
  for (const groupLeaves of leaves.values()) {
    for (const leaf of groupLeaves) {
      if (leaf.revenue > 0) {
        return true;
      }
    }
  }
  return false;
}

/** One row per treemap series, in the same order as `buildSeries`. */
function buildSeriesLeaves(leaves: Map<SourceGroup, TreemapLeaf[]>): TreemapLeaf[][] {
  const rows: TreemapLeaf[][] = [];
  for (const group of SOURCE_GROUP_ORDER) {
    const groupLeaves = leaves.get(group) ?? [];
    if (groupLeaves.length > 0) {
      rows.push(groupLeaves);
    }
  }
  return rows;
}

function buildOptions(
  leaves: Map<SourceGroup, TreemapLeaf[]>,
  labelByDatumKey: Map<string, string>,
  handlers?: TreemapChartHandlers,
): ApexOptions {
  const { series, colors } = buildSeries(leaves);
  const seriesLeaves = buildSeriesLeaves(leaves);
  return {
    chart: {
      type: "treemap",
      height: 460,
      toolbar: { show: false },
      fontFamily: "Inter, Roboto, 'Helvetica Neue', Arial, sans-serif",
      events: treemapLeafEvents({
        seriesLeaves: seriesLeaves,
        hover: handlers,
        onSelect: handlers?.onLeafSelect
          ? (leaf) => handlers.onLeafSelect?.(leaf.datumKey)
          : undefined,
      }),
    },
    series: series as ApexOptions["series"],
    colors,
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

export default function RevenueSourcesTreemap(props: RevenueSourcesTreemapProps) {
  let chartElement: HTMLDivElement | undefined;
  let chart: ApexCharts | undefined;
  const hover = createCursorHover();

  const hoveredRecord = createMemo(() => {
    const key = hover.key();
    return key ? props.recordByKey.get(key) : undefined;
  });

  const hoverContent = createMemo(() => {
    const record = hoveredRecord();
    if (!record) {
      return undefined;
    }
    const isStarred = rowHasBestHighlight(record, props.bestHighlights);
    return buildBestSourceHoverContent(
      record,
      isStarred ? bestSourceRowHighlights(record, props.bestHighlights) : undefined,
    );
  });

  onMount(() => {
    if (!chartElement) {
      return;
    }
    chart = new ApexCharts(chartElement, {
      chart: { type: "treemap", height: 460 },
      series: [],
    });
    chart.render();
  });

  const showTreemap = () => props.loading || treemapHasLeaves(props.leaves);

  createEffect(() => {
    if (!chart || props.loading || props.error) {
      return;
    }
    const leaves = props.leaves;
    if (!treemapHasLeaves(leaves)) {
      return;
    }
    chart.updateOptions(
      buildOptions(leaves, buildLabelByDatumKey(leaves), {
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
          <p class={message}>Loading treemap…</p>
        </Show>
        <Show when={!props.loading && props.error}>
          <p class={errorMessage}>{props.error}</p>
        </Show>
        <div
          ref={chartElement}
          style={{
            display:
              props.loading || props.error || !treemapHasLeaves(props.leaves) ? "none" : "block",
          }}
        />
      </div>
      <CursorHoverCard hover={hover} open={hoveredRecord() !== undefined} wide>
        <Show when={hoveredRecord()}>
          {(record) => (
            <MetricHoverCardContent
              title={record().label}
              content={hoverContent()}
              statLayout="threeColumn"
            />
          )}
        </Show>
      </CursorHoverCard>
    </>
  );
}
