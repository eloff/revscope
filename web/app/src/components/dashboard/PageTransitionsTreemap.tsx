/// <reference types="solid-js" />
import { formatPercentRatio } from "../../lib/dashboardApi";
import type { PageTransitionRow, PageTransitionsResponse } from "../../lib/pagesApi";
import MetricHoverCardContent from "./MetricHoverCardContent";
import type { MetricHoverContent } from "./metricHoverDetails";
import { SOURCE_GROUP_COLORS } from "./revenueSourcesLogic";
import Treemap from "./Treemap";
import type { TreemapGroup, TreemapTile } from "./treemapLogic";

const TREEMAP_TOP_TRANSITIONS = 50;
const EXIT_DATUM_KEY = "__page_transition_exit__";
const OTHER_DATUM_KEY = "__page_transition_other__";
const OUTBOUND_DATUM_KEY_PREFIX = "outbound:";
/** Transitions carry no revenue, so visitors are the only metric. */
const METRICS = [{ value: "traffic", label: "Visitors" }] as const;

type TransitionLeaf = TreemapTile & {
  title: string;
  pathId: number | null;
  visitors: number;
  share: number;
  isExit: boolean;
  isOutbound: boolean;
  isOther: boolean;
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
  const leaves: TransitionLeaf[] = topTransitions.map((transition) => ({
    datumKey: transitionKey(transition),
    label: transitionLabel(transition),
    title: transitionTitle(transition),
    pathId: transition.path_id ?? null,
    value: transition.visitors,
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

function buildSeriesGroups(
  leaves: TransitionLeaf[],
  topGroupName: string,
): TreemapGroup<TransitionLeaf>[] {
  const pageLeaves = leaves.filter((leaf) => !leaf.isExit && !leaf.isOutbound && !leaf.isOther);
  // Outbound destinations are exits that left via a tracked link, so they share
  // the Exit group.
  const exitLeaves = leaves.filter((leaf) => leaf.isExit || leaf.isOutbound);
  const otherLeaves = leaves.filter((leaf) => leaf.isOther);
  return [
    {
      name: topGroupName,
      tiles: pageLeaves,
      color: SOURCE_GROUP_COLORS.campaigns,
    },
    {
      name: "Exit",
      tiles: exitLeaves,
      color: SOURCE_GROUP_COLORS.direct,
    },
    {
      name: "Other",
      tiles: otherLeaves,
      color: SOURCE_GROUP_COLORS.direct,
    },
  ];
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

export default function PageTransitionsTreemap(props: PageTransitionsTreemapProps) {
  return (
    <Treemap
      groups={() => props.data && buildSeriesGroups(buildLeaves(props.data), props.labels.topGroup)}
      metrics={METRICS}
      loading={props.loading}
      loadingLabel={props.labels.loading}
      emptyLabel={props.labels.empty}
      height={360}
      selectable={(leaf) => leaf.pathId !== null}
      onSelect={(leaf) => {
        if (leaf.pathId !== null) {
          props.onPathSelect?.(leaf.pathId);
        }
      }}
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
