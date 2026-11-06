/// <reference types="solid-js" />

import { createEffect, createMemo, onCleanup, onMount, Show } from "solid-js";
import {
  audienceBreakdownTreemap,
  audienceBreakdownTreemapHidden,
} from "./AudienceBreakdownTreemap.css";
import ApexCharts, { type ApexOptions } from "./apexcharts";
import {
  type AudienceBreakdownRow,
  type AudienceBreakdownTreemapLeaf,
  buildAudienceBreakdownTreemapLeaves,
} from "./audienceLogic";
import { chartMount } from "./chart.css";
import { message } from "./messages.css";
import { SOURCE_GROUP_COLORS } from "./revenueSourcesLogic";
import { type TreemapHoverHandlers, treemapLeafEvents } from "./treemapLeafEvents";

const TREEMAP_LABEL_FONT_SIZE = "12px";
const OTHER_LABEL = "Other";

type TreemapSeries = {
  name: string;
  data: { x: string; y: number }[];
};

type SeriesGroup = {
  name: string;
  leaves: AudienceBreakdownTreemapLeaf[];
  color: string;
};

type TreemapChartHandlers = TreemapHoverHandlers & {
  onLeafSelect?: (datumKey: string) => void;
};

type AudienceBreakdownTreemapProps = TreemapHoverHandlers & {
  rows: AudienceBreakdownRow[];
  topGroupColor: string;
  otherTooltipNoun: string;
  loading: boolean;
  onLeafSelect?: (datumKey: string) => void;
};

function buildLabelByDatumKey(leaves: AudienceBreakdownTreemapLeaf[]): Map<string, string> {
  const labelByDatumKey = new Map<string, string>();
  for (const leaf of leaves) {
    labelByDatumKey.set(leaf.datumKey, leaf.label);
  }
  return labelByDatumKey;
}

function buildSeriesGroups(
  leaves: AudienceBreakdownTreemapLeaf[],
  topGroupColor: string,
): SeriesGroup[] {
  const topLeaves = leaves.filter((leaf) => !leaf.isOther);
  const otherLeaves = leaves.filter((leaf) => leaf.isOther);
  return [
    {
      name: "Top by revenue",
      leaves: topLeaves,
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
  leaves: AudienceBreakdownTreemapLeaf[],
  labelByDatumKey: Map<string, string>,
  topGroupColor: string,
  handlers?: TreemapChartHandlers,
): ApexOptions {
  const groups = buildSeriesGroups(leaves, topGroupColor);
  const series: TreemapSeries[] = groups.map((group) => ({
    name: group.name,
    data: group.leaves.map((leaf) => ({
      x: leaf.datumKey,
      y: Math.max(0, leaf.revenue),
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

export default function AudienceBreakdownTreemap(props: AudienceBreakdownTreemapProps) {
  let chartElement: HTMLDivElement | undefined;
  let chart: ApexCharts | undefined;

  const leaves = createMemo(() =>
    buildAudienceBreakdownTreemapLeaves(props.rows, props.otherTooltipNoun),
  );
  const showTreemap = () => props.loading || leaves().length > 0;

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
    const currentLeaves = leaves();
    if (currentLeaves.length === 0) {
      return;
    }
    chart.updateOptions(
      buildOptions(currentLeaves, buildLabelByDatumKey(currentLeaves), props.topGroupColor, {
        onLeafSelect: props.onLeafSelect,
        onLeafHover: props.onLeafHover,
        onChartMouseMove: props.onChartMouseMove,
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
    <div
      class={
        showTreemap()
          ? audienceBreakdownTreemap
          : `${audienceBreakdownTreemap} ${audienceBreakdownTreemapHidden}`
      }
    >
      <div class={chartMount}>
        <Show when={props.loading}>
          <p class={message}>Loading breakdown treemap…</p>
        </Show>
        <div
          ref={chartElement}
          style={{
            display: props.loading || leaves().length === 0 ? "none" : "block",
          }}
        />
      </div>
    </div>
  );
}
