/// <reference types="solid-js" />
import { formatPercentRatio } from "../../lib/dashboardApi";
import type { FunnelDropoffRow } from "../../lib/funnelsApi";
import MetricHoverCardContent from "./MetricHoverCardContent";
import type { MetricHoverContent } from "./metricHoverDetails";
import { SOURCE_GROUP_COLORS } from "./revenueSourcesLogic";
import Treemap from "./Treemap";
import type { TreemapGroup, TreemapTile } from "./treemapLogic";

const TOP_DESTINATIONS = 24;
const EXIT_KEY = "__funnel_dropoff_exit__";
const OTHER_KEY = "__funnel_dropoff_other__";
const OUTBOUND_KEY_PREFIX = "outbound:";

/** Drop-offs carry no revenue, so visitors are the only metric. */
const METRICS = [{ value: "traffic", label: "Visitors" }] as const;

type DropoffLeaf = TreemapTile & {
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
    value: destination.visitors,
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
      value: visitors,
      visitors,
      share,
      isExit: false,
      isOutbound: false,
      isOther: true,
    });
  }
  return leaves;
}

function buildGroups(leaves: DropoffLeaf[]): TreemapGroup<DropoffLeaf>[] {
  return [
    {
      name: "Destinations",
      color: SOURCE_GROUP_COLORS.campaigns,
      tiles: leaves.filter((leaf) => !leaf.isExit && !leaf.isOutbound && !leaf.isOther),
    },
    {
      name: "Exit",
      color: SOURCE_GROUP_COLORS.direct,
      // Outbound destinations are exits that left via a tracked link, so they
      // share the Exit group.
      tiles: leaves.filter((leaf) => leaf.isExit || leaf.isOutbound),
    },
    {
      name: "Other",
      color: SOURCE_GROUP_COLORS.referrers,
      tiles: leaves.filter((leaf) => leaf.isOther),
    },
  ];
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

export default function FunnelDropoffTreemap(props: FunnelDropoffTreemapProps) {
  return (
    <Treemap
      groups={() => buildGroups(buildLeaves(props.transitions))}
      metrics={METRICS}
      loading={false}
      loadingLabel=""
      emptyLabel="No next-page transitions for this step."
      height={220}
      labelFontSize="11px"
      animate={false}
      wideHoverCard
      hoverCard={(leaf) => (
        <MetricHoverCardContent
          title={leaf.title}
          content={buildHoverContent(leaf)}
          statLayout="twoColumn"
          showPeriodComparisonNote={false}
        />
      )}
    />
  );
}
