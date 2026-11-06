/// <reference types="solid-js" />
import { createEffect, createMemo, onCleanup, onMount, Show } from "solid-js";
import { formatPercentRatio } from "../../lib/dashboardApi";
import type { FunnelDropoffRow } from "../../lib/funnelsApi";
import ApexCharts, { type ApexOptions } from "./apexcharts";
import CursorHoverCard from "./CursorHoverCard";
import { createCursorHover } from "./createCursorHover";
import { funnelDropoffChartMount, funnelDropoffTreemapMount } from "./FunnelDropoffTreemap.css";
import MetricHoverCardContent from "./MetricHoverCardContent";
import type { MetricHoverContent } from "./metricHoverDetails";
import { SOURCE_GROUP_COLORS } from "./revenueSourcesLogic";
import { type TreemapHoverHandlers, treemapLeafEvents } from "./treemapLeafEvents";

const TOP_DESTINATIONS = 24;
const EXIT_KEY = "__funnel_dropoff_exit__";
const OTHER_KEY = "__funnel_dropoff_other__";
const OUTBOUND_KEY_PREFIX = "outbound:";

type DropoffLeaf = {
  datumKey: string;
  label: string;
  title: string;
  visitors: number;
  share: number;
  isExit: boolean;
  isOutbound: boolean;
  isOther: boolean;
};

type FunnelDropoffTreemapProps = {
  transitions: FunnelDropoffRow[];
};

function destinationTitle(destination: FunnelDropoffRow): string {
  if (destination.path_id === undefined) {
    return destination.outbound_to ?? "Exit";
  }
  return `${destination.hostname ?? ""}${destination.path ?? ""}`;
}

function destinationLabel(destination: FunnelDropoffRow): string {
  if (destination.path_id === undefined) {
    return destination.outbound_to ?? "Exit";
  }
  return destination.path ?? destinationTitle(destination);
}

function destinationKey(destination: FunnelDropoffRow): string {
  if (destination.path_id !== undefined) {
    return `path:${destination.path_id}`;
  }
  if (destination.outbound_to !== undefined) {
    return `${OUTBOUND_KEY_PREFIX}${destination.outbound_to}`;
  }
  return EXIT_KEY;
}

function compareDropoff(left: FunnelDropoffRow, right: FunnelDropoffRow): number {
  if (right.visitors !== left.visitors) {
    return right.visitors - left.visitors;
  }
  return destinationTitle(left).localeCompare(destinationTitle(right));
}

function buildLeaves(transitions: FunnelDropoffRow[]): DropoffLeaf[] {
  const ranked = [...transitions].sort(compareDropoff);
  const top = ranked.slice(0, TOP_DESTINATIONS);
  const rest = ranked.slice(TOP_DESTINATIONS);
  const leaves: DropoffLeaf[] = top.map((destination) => ({
    datumKey: destinationKey(destination),
    label: destinationLabel(destination),
    title: destinationTitle(destination),
    visitors: destination.visitors,
    share: destination.share,
    isExit: destination.path_id === undefined && destination.outbound_to === undefined,
    isOutbound: destination.path_id === undefined && destination.outbound_to !== undefined,
    isOther: false,
  }));
  if (rest.length > 0) {
    const visitors = rest.reduce((total, row) => total + row.visitors, 0);
    const share = rest.reduce((total, row) => total + row.share, 0);
    leaves.push({
      datumKey: OTHER_KEY,
      label: "Other",
      title: `Other ${rest.length.toLocaleString()} destinations`,
      visitors,
      share,
      isExit: false,
      isOutbound: false,
      isOther: true,
    });
  }
  return leaves;
}

function buildLabelByDatumKey(leaves: DropoffLeaf[]): Map<string, string> {
  const labelByDatumKey = new Map<string, string>();
  for (const leaf of leaves) {
    labelByDatumKey.set(leaf.datumKey, leaf.label);
  }
  return labelByDatumKey;
}

function buildLeafByDatumKey(leaves: DropoffLeaf[]): Map<string, DropoffLeaf> {
  const leafByDatumKey = new Map<string, DropoffLeaf>();
  for (const leaf of leaves) {
    leafByDatumKey.set(leaf.datumKey, leaf);
  }
  return leafByDatumKey;
}

type SeriesGroup = {
  name: string;
  color: string;
  leaves: DropoffLeaf[];
};

function buildGroups(leaves: DropoffLeaf[]): SeriesGroup[] {
  return [
    {
      name: "Destinations",
      color: SOURCE_GROUP_COLORS.campaigns,
      leaves: leaves.filter((leaf) => !leaf.isExit && !leaf.isOutbound && !leaf.isOther),
    },
    {
      name: "Exit",
      color: SOURCE_GROUP_COLORS.direct,
      // Outbound destinations are exits that left via a tracked link, so they
      // share the Exit group.
      leaves: leaves.filter((leaf) => leaf.isExit || leaf.isOutbound),
    },
    {
      name: "Other",
      color: SOURCE_GROUP_COLORS.referrers,
      leaves: leaves.filter((leaf) => leaf.isOther),
    },
  ].filter((group) => group.leaves.length > 0);
}

function buildHoverContent(leaf: DropoffLeaf): MetricHoverContent {
  return {
    stats: [
      {
        label: "Visitors",
        value: leaf.visitors.toLocaleString(),
      },
      {
        label: "Share",
        value: formatPercentRatio(leaf.share, 1),
      },
    ],
  };
}

function buildOptions(
  leaves: DropoffLeaf[],
  labelByDatumKey: Map<string, string>,
  handlers?: TreemapHoverHandlers,
): ApexOptions {
  const groups = buildGroups(leaves);
  const seriesLeaves = groups.map((group) => group.leaves);
  return {
    chart: {
      type: "treemap",
      height: 220,
      toolbar: { show: false },
      animations: { enabled: false },
      fontFamily: "Inter, Roboto, 'Helvetica Neue', Arial, sans-serif",
      events: treemapLeafEvents({
        seriesLeaves: seriesLeaves,
        hover: handlers,
      }),
    },
    series: groups.map((group) => ({
      name: group.name,
      data: group.leaves.map((leaf) => ({
        x: leaf.datumKey,
        y: Math.max(0, leaf.visitors),
      })),
    })) as ApexOptions["series"],
    colors: groups.map((group) => group.color),
    legend: { show: false },
    dataLabels: {
      enabled: true,
      style: { fontSize: "11px" },
      formatter: (text) => labelByDatumKey.get(String(text)) ?? String(text),
    },
    plotOptions: {
      treemap: {
        distributed: false,
        enableShades: true,
        dataLabels: { format: "truncate" },
      },
    },
    tooltip: { enabled: false },
  };
}

export default function FunnelDropoffTreemap(props: FunnelDropoffTreemapProps) {
  let chartElement: HTMLDivElement | undefined;
  let chart: ApexCharts | undefined;
  const hover = createCursorHover();

  const leaves = createMemo(() => buildLeaves(props.transitions));
  const leafByKey = createMemo(() => buildLeafByDatumKey(leaves()));
  const hoveredLeaf = createMemo(() => {
    const key = hover.key();
    return key ? leafByKey().get(key) : undefined;
  });
  const hoverContent = createMemo(() => {
    const leaf = hoveredLeaf();
    return leaf ? buildHoverContent(leaf) : undefined;
  });

  onMount(() => {
    if (!chartElement) {
      return;
    }
    chart = new ApexCharts(chartElement, {
      chart: { type: "treemap", height: 220 },
      series: [],
    });
    chart.render();
  });

  createEffect(() => {
    if (!chart) {
      return;
    }
    const currentLeaves = leaves();
    chart.updateOptions(
      buildOptions(currentLeaves, buildLabelByDatumKey(currentLeaves), {
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
      <div class={funnelDropoffChartMount}>
        <div ref={chartElement} class={funnelDropoffTreemapMount} />
      </div>
      <CursorHoverCard hover={hover} open={hoveredLeaf() !== undefined} wide>
        <Show when={hoveredLeaf()}>
          {(leaf) => (
            <MetricHoverCardContent
              title={leaf().title}
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
