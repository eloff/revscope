/// <reference types="solid-js" />
import { audienceBreakdownTreemap } from "./AudienceBreakdownTreemap.css";
import { type AudienceBreakdownRow, buildAudienceBreakdownTreemapLeaves } from "./audienceLogic";
import { SOURCE_GROUP_COLORS } from "./revenueSourcesLogic";
import Treemap, { REVENUE_OR_VISITORS } from "./Treemap";
import type { TreemapHoverHandlers } from "./treemapLeafEvents";
import type { TreemapMetric } from "./treemapLogic";

type AudienceBreakdownTreemapProps = TreemapHoverHandlers & {
  rows: AudienceBreakdownRow[];
  topGroupColor: string;
  loading: boolean;
  /** Set when the rows are the top of a longer list ranked by this metric, as cities are. */
  sliceRankedBy?: TreemapMetric;
  onLeafSelect?: (datumKey: string) => void;
};

export default function AudienceBreakdownTreemap(props: AudienceBreakdownTreemapProps) {
  return (
    <Treemap
      class={audienceBreakdownTreemap}
      groups={(metric) => {
        const leaves = buildAudienceBreakdownTreemapLeaves(props.rows, metric);
        return [
          {
            name: "Top",
            color: props.topGroupColor,
            tiles: leaves.filter((leaf) => !leaf.isOther),
          },
          {
            name: "Other",
            color: SOURCE_GROUP_COLORS.direct,
            tiles: leaves.filter((leaf) => leaf.isOther),
          },
        ];
      }}
      metrics={REVENUE_OR_VISITORS}
      sliceRankedBy={props.sliceRankedBy}
      loading={props.loading}
      loadingLabel="Loading breakdown treemap…"
      height={420}
      hover={{ onLeafHover: props.onLeafHover, onChartMouseMove: props.onChartMouseMove }}
      hoverable={(leaf) => !leaf.isOther}
      selectable={(leaf) => !leaf.isOther}
      onSelect={(leaf) => props.onLeafSelect?.(leaf.datumKey)}
    />
  );
}
