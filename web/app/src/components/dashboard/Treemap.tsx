/// <reference types="solid-js" />
import { createEffect, createMemo, type JSX, on, onCleanup, onMount, Show } from "solid-js";
import { rankBy, setRankBy } from "../../lib/dashboardPreferences";
import ApexCharts, { type ApexOptions } from "./apexcharts";
import CursorHoverCard from "./CursorHoverCard";
import { createCursorHover } from "./createCursorHover";
import { message } from "./messages.css";
import SegmentedControl, { type SegmentedControlOption } from "./SegmentedControl";
import { treemapChartArea, treemapMetricToggle, treemapRoot } from "./Treemap.css";
import { type TreemapHoverHandlers, treemapLeafEvents } from "./treemapLeafEvents";
import type { TreemapGroup, TreemapMetric, TreemapTile } from "./treemapLogic";

export const REVENUE_OR_VISITORS: readonly SegmentedControlOption<TreemapMetric>[] = [
  { value: "revenue", label: "Revenue" },
  { value: "traffic", label: "Visitors" },
];

type TreemapProps<T extends TreemapTile> = {
  /** Tile groups sized by `metric`, or undefined until data arrives. */
  groups: (metric: TreemapMetric) => TreemapGroup<T>[] | undefined;
  /**
   * Metrics on offer, preferred first. More than one adds a toggle, which is
   * the dashboard-wide `rankBy` preference, so every such treemap switches together.
   */
  metrics: readonly SegmentedControlOption<TreemapMetric>[];
  /**
   * Set when the tiles come from a top slice of rows ranked by this metric.
   * Finding no value for another metric in it doesn't mean there is none.
   */
  sliceRankedBy?: TreemapMetric;
  loading: boolean;
  loadingLabel: string;
  /**
   * Shown when loaded data has no tile with any area. Without it the treemap
   * takes no space at all then.
   */
  emptyLabel?: string;
  height: number;
  labelFontSize?: string;
  animate?: boolean;
  /** Colors each tile itself instead of shading its group's color, drawing all groups as one series. */
  tileColor?: (tile: T) => string;
  /** Hover card for a tile, anchored at the cursor. */
  hoverCard?: (tile: T) => JSX.Element;
  wideHoverCard?: boolean;
  /** Reports hovered tiles to a hover card the caller owns, instead of `hoverCard`. */
  hover?: TreemapHoverHandlers;
  hoverable?: (tile: T) => boolean;
  selectable?: (tile: T) => boolean;
  onSelect?: (tile: T) => void;
  /** Class for the outer element, which isn't rendered while there's nothing to show. */
  class?: string;
};

/**
 * A treemap that can switch between metrics. It falls back to a metric that
 * has data when the chosen one has none, and leaves zero-area tiles out, so it
 * never draws an empty chart.
 */
export default function Treemap<T extends TreemapTile>(props: TreemapProps<T>) {
  const chosenMetric = () => (props.metrics.length > 1 ? rankBy() : undefined);
  const ownHover = createCursorHover();

  const groupsByMetric = createMemo(() => {
    const byMetric = new Map<TreemapMetric, TreemapGroup<T>[] | undefined>();
    for (const option of props.metrics) {
      byMetric.set(option.value, drawableGroups(props.groups(option.value)));
    }
    return byMetric;
  });
  const isDrawable = (metric: TreemapMetric) => (groupsByMetric().get(metric)?.length ?? 0) > 0;
  const canChoose = (metric: TreemapMetric) =>
    isDrawable(metric) || (props.sliceRankedBy !== undefined && metric !== props.sliceRankedBy);
  const metric = createMemo(() => {
    const chosen = chosenMetric();
    if (chosen !== undefined && isDrawable(chosen)) {
      return chosen;
    }
    return props.metrics.find((option) => isDrawable(option.value))?.value;
  });
  // Rebuilt only when a metric gains or loses data, so a data refresh doesn't
  // recreate the toggle's items and drop its keyboard focus.
  const toggleOptions = createMemo(
    () => props.metrics.map((option) => ({ ...option, disabled: !canChoose(option.value) })),
    undefined,
    {
      equals: (previous, next) =>
        previous.length === next.length &&
        previous.every(
          (option, index) =>
            option.value === next[index]?.value && option.disabled === next[index]?.disabled,
        ),
    },
  );
  const groups = createMemo(() => {
    const current = metric();
    return props.loading || current === undefined ? undefined : groupsByMetric().get(current);
  });
  const showEmptyLabel = () =>
    !props.loading &&
    props.emptyLabel !== undefined &&
    groups() === undefined &&
    groupsByMetric().get(props.metrics[0]?.value ?? "traffic") !== undefined;

  const hoveredTile = createMemo(() => {
    const key = ownHover.key();
    return key === null ? undefined : findTile(groups(), key);
  });
  const hover = (): TreemapHoverHandlers | undefined =>
    props.hoverCard ? { onLeafHover: ownHover.show, onChartMouseMove: ownHover.move } : props.hover;

  const options = createMemo(() => {
    const current = groups();
    return current ? chartOptions(current, props, hover()) : undefined;
  });

  return (
    <>
      <Show when={props.loading || groups() || showEmptyLabel()}>
        <div
          class={props.class ? `${treemapRoot} ${props.class}` : treemapRoot}
          style={{ "min-height": props.loading ? `${props.height}px` : undefined }}
        >
          <Show when={props.loading}>
            <p class={message}>{props.loadingLabel}</p>
          </Show>
          <Show when={showEmptyLabel()}>
            <p class={message}>{props.emptyLabel}</p>
          </Show>
          <Show when={options()}>
            {(current) => (
              <div class={treemapChartArea}>
                <Show when={props.metrics.length > 1}>
                  <div class={treemapMetricToggle}>
                    <SegmentedControl
                      size="small"
                      ariaLabel="Size tiles by"
                      value={metric() ?? "traffic"}
                      options={toggleOptions()}
                      onChange={setRankBy}
                      // A fallback shows a metric `rankBy` isn't set to, and
                      // picking it should still rank everything by it.
                      onReselect={setRankBy}
                    />
                  </div>
                </Show>
                <TreemapChart options={current()} />
              </div>
            )}
          </Show>
        </div>
      </Show>
      <Show when={groups() && props.hoverCard}>
        {(renderCard) => (
          <CursorHoverCard
            hover={ownHover}
            open={hoveredTile() !== undefined}
            wide={props.wideHoverCard}
          >
            <Show when={hoveredTile()} keyed>
              {(tile) => renderCard()(tile)}
            </Show>
          </CursorHoverCard>
        )}
      </Show>
    </>
  );
}

/** Mounted only while there are tiles, so the chart is never drawn empty or hidden. */
function TreemapChart(props: { options: ApexOptions }) {
  let element: HTMLDivElement | undefined;
  let chart: ApexCharts | undefined;

  onMount(() => {
    if (!element) {
      return;
    }
    chart = new ApexCharts(element, props.options);
    chart.render();
  });

  createEffect(
    on(
      () => props.options,
      (options) => chart?.updateOptions(options, true, true),
      { defer: true },
    ),
  );

  onCleanup(() => {
    chart?.destroy();
    chart = undefined;
  });

  return <div ref={element} />;
}

function drawableGroups<T extends TreemapTile>(
  groups: TreemapGroup<T>[] | undefined,
): TreemapGroup<T>[] | undefined {
  return groups
    ?.map((group) => ({ ...group, tiles: group.tiles.filter((tile) => tile.value > 0) }))
    .filter((group) => group.tiles.length > 0);
}

function findTile<T extends TreemapTile>(
  groups: TreemapGroup<T>[] | undefined,
  datumKey: string,
): T | undefined {
  for (const group of groups ?? []) {
    const tile = group.tiles.find((candidate) => candidate.datumKey === datumKey);
    if (tile) {
      return tile;
    }
  }
  return undefined;
}

function chartOptions<T extends TreemapTile>(
  shownGroups: TreemapGroup<T>[],
  props: TreemapProps<T>,
  hover: TreemapHoverHandlers | undefined,
): ApexOptions {
  const tileColor = props.tileColor;
  // ApexCharts picks a distributed tile's color by its index within its own
  // series, so tiles colored one by one must share a single series.
  const groups = tileColor
    ? [{ name: "", color: "", tiles: shownGroups.flatMap((group) => group.tiles) }]
    : shownGroups;
  const labelByDatumKey = new Map<string, string>();
  for (const group of groups) {
    for (const tile of group.tiles) {
      labelByDatumKey.set(tile.datumKey, tile.label);
    }
  }
  return {
    chart: {
      type: "treemap",
      height: props.height,
      toolbar: { show: false },
      animations: { enabled: props.animate ?? true },
      fontFamily: "Inter, Roboto, 'Helvetica Neue', Arial, sans-serif",
      events: treemapLeafEvents({
        seriesLeaves: groups.map((group) => group.tiles),
        hover,
        hoverable: props.hoverable,
        onSelect: props.onSelect,
        selectable: props.selectable,
      }),
    },
    series: groups.map((group) => ({
      name: group.name,
      data: group.tiles.map((tile) => ({ x: tile.datumKey, y: tile.value })),
    })) as ApexOptions["series"],
    colors: tileColor ? groups[0]?.tiles.map(tileColor) : groups.map((group) => group.color),
    legend: { show: false },
    dataLabels: {
      enabled: true,
      // A fixed size with truncation clips long labels instead of shrinking them.
      style: { fontSize: props.labelFontSize ?? "12px" },
      formatter: (text) => labelByDatumKey.get(String(text)) ?? String(text),
    },
    plotOptions: {
      treemap: {
        distributed: tileColor !== undefined,
        enableShades: tileColor === undefined,
        dataLabels: { format: "truncate" },
      },
    },
    tooltip: { enabled: false },
  };
}
