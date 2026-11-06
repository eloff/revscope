import type { ApexOptions } from "./apexcharts";

type ChartEvents = NonNullable<NonNullable<ApexOptions["chart"]>["events"]>;

export type TreemapHoverHandlers = {
  onLeafHover?: (key: string | null, clientX: number, clientY: number) => void;
  onChartMouseMove?: (clientX: number, clientY: number) => void;
};

/**
 * ApexCharts treemap events that resolve the leaf under the pointer and report
 * it by key. Leaves that `hoverable` or `selectable` reject are ignored.
 */
export function treemapLeafEvents<L extends { datumKey: string }>(options: {
  seriesLeaves: L[][];
  hover?: TreemapHoverHandlers;
  hoverable?: (leaf: L) => boolean;
  onSelect?: (leaf: L) => void;
  selectable?: (leaf: L) => boolean;
}): ChartEvents {
  const leafAt = (config: { seriesIndex: number; dataPointIndex: number } | undefined) =>
    config === undefined
      ? undefined
      : options.seriesLeaves[config.seriesIndex]?.[config.dataPointIndex];
  const clear = () => options.hover?.onLeafHover?.(null, 0, 0);
  const onSelect = options.onSelect;
  return {
    ...(onSelect
      ? {
          dataPointSelection(_event, _chartContext, config) {
            const leaf = leafAt(config);
            if (leaf && (options.selectable?.(leaf) ?? true)) {
              onSelect(leaf);
            }
          },
        }
      : {}),
    dataPointMouseEnter(event, _chartContext, config) {
      const leaf = leafAt(config);
      if (leaf && (options.hoverable?.(leaf) ?? true)) {
        options.hover?.onLeafHover?.(leaf.datumKey, event.clientX, event.clientY);
      }
    },
    dataPointMouseLeave: clear,
    mouseMove(event) {
      options.hover?.onChartMouseMove?.(event.clientX, event.clientY);
    },
    mouseLeave: clear,
  };
}
