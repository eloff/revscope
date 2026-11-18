/// <reference types="solid-js" />
import type {
  SearchTermRow,
  SearchTermsPeriod,
  SearchTermsResponse,
} from "../../lib/searchTermsApi";
import MetricHoverCardContent from "./MetricHoverCardContent";
import { SOURCE_GROUP_COLORS } from "./revenueSourcesLogic";
import {
  buildSearchTermHoverContent,
  buildSearchTermOtherHoverContent,
} from "./searchTermHoverDetails";
import {
  SEARCH_TERMS_OTHER_DATUM_KEY,
  searchTermRowHighlights,
  searchTermRowKey,
} from "./searchTermsLogic";
import Treemap from "./Treemap";
import {
  compareByMetric,
  type TreemapGroup,
  type TreemapMetric,
  type TreemapTile,
} from "./treemapLogic";

const TREEMAP_TOP_TERMS = 50;
const OTHER_LABEL = "Other";

const METRICS = [
  { value: "revenue", label: "Revenue" },
  { value: "traffic", label: "Clicks" },
] as const;

/** A term's tile, or the Other tile (no `term`) for the rest of the channel. */
type SearchTermTile = TreemapTile & {
  term?: SearchTermRow;
  /** Queries with their own tile, which the Other tile's hover card leaves out. */
  topQueries: Set<string>;
};

type SearchTermsTreemapProps = {
  data: SearchTermsResponse | undefined;
  loading: boolean;
  onLeafSelect?: (datumKey: string) => void;
};

function termValue(term: SearchTermRow, metric: TreemapMetric): number {
  return metric === "revenue" ? term.estimated_revenue : term.clicks;
}

/** Terms are paged, so Other is the channel total less the top terms rather than a sum of the rest. */
function buildSearchTermGroups(
  period: SearchTermsPeriod,
  metric: TreemapMetric,
): TreemapGroup<SearchTermTile>[] {
  const topTerms = [...period.terms]
    .sort(
      compareByMetric(metric, termValue, (left, right) => left.query.localeCompare(right.query)),
    )
    .slice(0, TREEMAP_TOP_TERMS);
  const topQueries = new Set(topTerms.map((term) => term.query));
  const topTotal = topTerms.reduce((total, term) => total + termValue(term, metric), 0);
  const channelTotal = metric === "revenue" ? period.search_channel_revenue : period.summary.clicks;
  return [
    {
      name: "Top terms",
      color: SOURCE_GROUP_COLORS.search,
      tiles: topTerms.map((term) => ({
        datumKey: searchTermRowKey(term),
        label: term.query,
        value: termValue(term, metric),
        term,
        topQueries,
      })),
    },
    {
      name: OTHER_LABEL,
      color: SOURCE_GROUP_COLORS.referrers,
      tiles: [
        {
          datumKey: SEARCH_TERMS_OTHER_DATUM_KEY,
          label: OTHER_LABEL,
          value: channelTotal - topTotal,
          topQueries,
        },
      ],
    },
  ];
}

export default function SearchTermsTreemap(props: SearchTermsTreemapProps) {
  return (
    <Treemap
      groups={(metric) =>
        props.data ? buildSearchTermGroups(props.data.current, metric) : undefined
      }
      metrics={METRICS}
      loading={props.loading}
      loadingLabel="Loading search terms treemap…"
      height={420}
      selectable={(tile) => tile.term !== undefined}
      onSelect={(tile) => props.onLeafSelect?.(tile.datumKey)}
      hoverCard={(tile) => (
        <MetricHoverCardContent
          title={tile.term?.query ?? OTHER_LABEL}
          content={
            tile.term
              ? buildSearchTermHoverContent(tile.term, searchTermRowHighlights(tile.term))
              : props.data &&
                buildSearchTermOtherHoverContent(
                  props.data.current,
                  tile.topQueries,
                  TREEMAP_TOP_TERMS,
                )
          }
          statLayout="threeColumn"
        />
      )}
    />
  );
}
