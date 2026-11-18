/// <reference types="solid-js" />
import MetricHoverCardContent from "./MetricHoverCardContent";
import { buildBestSourceHoverContent } from "./revenueSourceHoverDetails";
import {
  type BestSourceHighlights,
  bestSourceRowHighlights,
  SOURCE_GROUP_COLORS,
  SOURCE_GROUP_LABELS,
  SOURCE_GROUP_ORDER,
  type SourceGroup,
  sourceRowHasBestHighlight,
  type TreemapLeaf,
} from "./revenueSourcesLogic";
import Treemap, { REVENUE_OR_VISITORS } from "./Treemap";
import type { TreemapMetric } from "./treemapLogic";

type RevenueSourcesTreemapProps = {
  /** Leaves by source group for a metric, or undefined until data arrives. */
  leaves: (metric: TreemapMetric) => Map<SourceGroup, TreemapLeaf[]> | undefined;
  bestHighlights: BestSourceHighlights;
  loading: boolean;
  onLeafSelect?: (datumKey: string) => void;
  /** Whether a tile has a grid row to go to; a Visitors-mode Other tile may not. */
  selectable?: (datumKey: string) => boolean;
};

export default function RevenueSourcesTreemap(props: RevenueSourcesTreemapProps) {
  return (
    <Treemap
      groups={(metric) => {
        const leaves = props.leaves(metric);
        return leaves
          ? SOURCE_GROUP_ORDER.map((group) => ({
              name: SOURCE_GROUP_LABELS[group],
              color: SOURCE_GROUP_COLORS[group],
              tiles: leaves.get(group) ?? [],
            }))
          : undefined;
      }}
      metrics={REVENUE_OR_VISITORS}
      loading={props.loading}
      loadingLabel="Loading treemap…"
      height={460}
      selectable={(leaf) => props.selectable?.(leaf.datumKey) ?? true}
      onSelect={(leaf) => props.onLeafSelect?.(leaf.datumKey)}
      wideHoverCard
      hoverCard={(leaf) => (
        <MetricHoverCardContent
          title={leaf.record.label}
          content={buildBestSourceHoverContent(
            leaf.record,
            sourceRowHasBestHighlight(leaf.record.key, props.bestHighlights)
              ? bestSourceRowHighlights(leaf.record, props.bestHighlights)
              : undefined,
          )}
          statLayout="threeColumn"
        />
      )}
    />
  );
}
