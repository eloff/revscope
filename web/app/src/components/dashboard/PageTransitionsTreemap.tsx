/// <reference types="solid-js" />
import { createEffect, createMemo, onCleanup, onMount, Show } from "solid-js";
import { formatPercentRatio } from "../../lib/dashboardApi";
import type { PageTransitionRow, PageTransitionsResponse } from "../../lib/pagesApi";
import ApexCharts, { type ApexOptions } from "./apexcharts";
import CursorHoverCard from "./CursorHoverCard";
import { chartMount } from "./chart.css";
import { createCursorHover } from "./createCursorHover";
import MetricHoverCardContent from "./MetricHoverCardContent";
import { message } from "./messages.css";
import type { MetricHoverContent } from "./metricHoverDetails";
import { SOURCE_GROUP_COLORS } from "./revenueSourcesLogic";
import { type TreemapHoverHandlers, treemapLeafEvents } from "./treemapLeafEvents";

const TREEMAP_TOP_TRANSITIONS = 50;
const EXIT_DATUM_KEY = "__page_transition_exit__";
const OTHER_DATUM_KEY = "__page_transition_other__";
const OUTBOUND_DATUM_KEY_PREFIX = "outbound:";
const TREEMAP_LABEL_FONT_SIZE = "12px";

type TransitionLeaf = {
  datumKey: string;
  label: string;
  title: string;
  pathId: number | null;
  visitors: number;
  share: number;
  isExit: boolean;
  isOutbound: boolean;
  isOther: boolean;
};

type TreemapSeries = {
  name: string;
  data: { x: string; y: number }[];
};

type SeriesGroup = {
  name: string;
  leaves: TransitionLeaf[];
  color: string;
};

type TreemapChartHandlers = TreemapHoverHandlers & {
  onLeafSelect?: (leaf: TransitionLeaf) => void;
};

type PageTransitionsTreemapProps = {
  data: PageTransitionsResponse | undefined;
  loading: boolean;
  labels: {
    loading: string;
    empty: string;
    topGroup: string;
  };
  onPathSelect?: (pathId: number) => void;
};

function transitionTitle(transition: PageTransitionRow): string {
  if (transition.path_id === undefined) {
    return transition.outbound_to ?? "Exit";
  }
  return `${transition.hostname ?? ""}${transition.path ?? ""}`;
}

function transitionLabel(transition: PageTransitionRow): string {
  if (transition.path_id === undefined) {
    return transition.outbound_to ?? "Exit";
  }
  return transition.path ?? transitionTitle(transition);
}

function transitionKey(transition: PageTransitionRow): string {
  if (transition.path_id !== undefined) {
    return `path:${transition.path_id}`;
  }
  if (transition.outbound_to !== undefined) {
    return `${OUTBOUND_DATUM_KEY_PREFIX}${transition.outbound_to}`;
  }
  return EXIT_DATUM_KEY;
}

function compareTransitionRank(left: PageTransitionRow, right: PageTransitionRow): number {
  if (right.visitors !== left.visitors) {
    return right.visitors - left.visitors;
  }
  return transitionTitle(left).localeCompare(transitionTitle(right));
}

function buildLeaves(response: PageTransitionsResponse): TransitionLeaf[] {
  const ranked = [...response.transitions].sort(compareTransitionRank);
  const topTransitions = ranked.slice(0, TREEMAP_TOP_TRANSITIONS);
  const otherTransitions = ranked.slice(TREEMAP_TOP_TRANSITIONS);
  const leaves = topTransitions.map((transition) => ({
    datumKey: transitionKey(transition),
    label: transitionLabel(transition),
    title: transitionTitle(transition),
    pathId: transition.path_id ?? null,
    visitors: transition.visitors,
    share: transition.share,
    isExit: transition.path_id === undefined && transition.outbound_to === undefined,
    isOutbound: transition.path_id === undefined && transition.outbound_to !== undefined,
    isOther: false,
  }));
  if (otherTransitions.length > 0) {
    const visitors = otherTransitions.reduce((total, transition) => total + transition.visitors, 0);
    const share = otherTransitions.reduce((total, transition) => total + transition.share, 0);
    leaves.push({
      datumKey: OTHER_DATUM_KEY,
      label: "Other",
      title: `Other ${otherTransitions.length.toLocaleString()} destinations`,
      pathId: null,
      visitors,
      share,
      isExit: false,
      isOutbound: false,
      isOther: true,
    });
  }
  return leaves;
}

function buildLabelByDatumKey(leaves: TransitionLeaf[]): Map<string, string> {
  const labelByDatumKey = new Map<string, string>();
  for (const leaf of leaves) {
    labelByDatumKey.set(leaf.datumKey, leaf.label);
  }
  return labelByDatumKey;
}

function buildLeafByDatumKey(leaves: TransitionLeaf[]): Map<string, TransitionLeaf> {
  const leafByDatumKey = new Map<string, TransitionLeaf>();
  for (const leaf of leaves) {
    leafByDatumKey.set(leaf.datumKey, leaf);
  }
  return leafByDatumKey;
}

function buildSeriesGroups(leaves: TransitionLeaf[], topGroupName: string): SeriesGroup[] {
  const pageLeaves = leaves.filter((leaf) => !leaf.isExit && !leaf.isOutbound && !leaf.isOther);
  // Outbound destinations are exits that left via a tracked link, so they share
  // the Exit group.
  const exitLeaves = leaves.filter((leaf) => leaf.isExit || leaf.isOutbound);
  const otherLeaves = leaves.filter((leaf) => leaf.isOther);
  return [
    {
      name: topGroupName,
      leaves: pageLeaves,
      color: SOURCE_GROUP_COLORS.campaigns,
    },
    {
      name: "Exit",
      leaves: exitLeaves,
      color: SOURCE_GROUP_COLORS.direct,
    },
    {
      name: "Other",
      leaves: otherLeaves,
      color: SOURCE_GROUP_COLORS.direct,
    },
  ].filter((group) => group.leaves.length > 0);
}

function buildHoverContent(leaf: TransitionLeaf): MetricHoverContent {
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
    description: leaf.isOutbound
      ? "Visitors who left this page via an outbound link to this destination."
      : leaf.isExit
        ? "Visitors who didn't return to the site after viewing this page."
        : "Visitors whose next page view went to this destination.",
  };
}

function buildOptions(
  leaves: TransitionLeaf[],
  labelByDatumKey: Map<string, string>,
  topGroupName: string,
  handlers?: TreemapChartHandlers,
): ApexOptions {
  const groups = buildSeriesGroups(leaves, topGroupName);
  const series: TreemapSeries[] = groups.map((group) => ({
    name: group.name,
    data: group.leaves.map((leaf) => ({
      x: leaf.datumKey,
      y: Math.max(0, leaf.visitors),
    })),
  }));
  const seriesLeaves = groups.map((group) => group.leaves);
  return {
    chart: {
      type: "treemap",
      height: 360,
      toolbar: { show: false },
      fontFamily: "Inter, Roboto, 'Helvetica Neue', Arial, sans-serif",
      events: treemapLeafEvents({
        seriesLeaves: seriesLeaves,
        hover: handlers,
        onSelect: handlers?.onLeafSelect,
        selectable: (leaf) => leaf.pathId !== null,
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

export default function PageTransitionsTreemap(props: PageTransitionsTreemapProps) {
  let chartElement: HTMLDivElement | undefined;
  let chart: ApexCharts | undefined;
  const hover = createCursorHover();

  const leaves = createMemo(() => (props.data ? buildLeaves(props.data) : undefined));
  const leafByKey = createMemo(() => {
    const current = leaves();
    return current ? buildLeafByDatumKey(current) : new Map<string, TransitionLeaf>();
  });
  const hoveredLeaf = createMemo(() => {
    const key = hover.key();
    return key ? leafByKey().get(key) : undefined;
  });
  const hoverContent = createMemo(() => {
    const leaf = hoveredLeaf();
    return leaf ? buildHoverContent(leaf) : undefined;
  });
  const hasLeaves = () => (leaves()?.length ?? 0) > 0;

  onMount(() => {
    if (!chartElement) {
      return;
    }
    chart = new ApexCharts(chartElement, {
      chart: { type: "treemap", height: 360 },
      series: [],
    });
    chart.render();
  });

  createEffect(() => {
    if (!chart || props.loading) {
      return;
    }
    const currentLeaves = leaves();
    if (!currentLeaves) {
      return;
    }
    chart.updateOptions(
      buildOptions(currentLeaves, buildLabelByDatumKey(currentLeaves), props.labels.topGroup, {
        onLeafSelect: (leaf) => {
          if (leaf.pathId !== null) {
            props.onPathSelect?.(leaf.pathId);
          }
        },
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
          <p class={message}>{props.labels.loading}</p>
        </Show>
        <Show when={!props.loading && props.data && !hasLeaves()}>
          <p class={message}>{props.labels.empty}</p>
        </Show>
        <div
          ref={chartElement}
          style={{
            display: props.loading || !props.data || !hasLeaves() ? "none" : "block",
          }}
        />
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
