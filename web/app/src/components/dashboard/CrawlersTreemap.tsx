/// <reference types="solid-js" />
import { formatPercentRatio } from "../../lib/dashboardApi";
import { CRAWLERS_TREEMAP_TOP_OPERATORS, type CrawlerOperatorLeaf } from "./crawlersLogic";
import MetricHoverCardContent from "./MetricHoverCardContent";
import type { MetricHoverContent } from "./metricHoverDetails";
import Treemap from "./Treemap";

/** Crawlers earn no revenue, so views are the only metric. */
const METRICS = [{ value: "traffic", label: "Views" }] as const;

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

export default function CrawlersTreemap(props: CrawlersTreemapProps) {
  return (
    <Treemap
      groups={() =>
        props.leaves && [
          {
            name: "Operators",
            color: "",
            tiles: props.leaves.map((leaf) => ({ ...leaf, value: leaf.views })),
          },
        ]
      }
      metrics={METRICS}
      loading={props.loading}
      loadingLabel="Loading crawlers treemap…"
      emptyLabel="No crawler views for the selected categories."
      height={420}
      tileColor={(leaf) => leaf.color}
      selectable={(leaf) => !leaf.isOther}
      onSelect={(leaf) => props.onLeafSelect?.(leaf.datumKey)}
      wideHoverCard
      hoverCard={(leaf) => (
        <MetricHoverCardContent
          title={leaf.label}
          content={buildHoverContent(leaf)}
          statLayout="twoColumn"
          showPeriodComparisonNote={false}
        />
      )}
    />
  );
}
