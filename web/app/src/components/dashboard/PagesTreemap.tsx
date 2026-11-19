/// <reference types="solid-js" />
import { Show } from "solid-js";
import type { PageRow, PagesResponse } from "../../lib/pagesApi";
import MetricHoverCardContent from "./MetricHoverCardContent";
import { buildPageHoverContent, type PagesVariant } from "./pageHoverDetails";
import { pageRowHighlights } from "./pagesLogic";
import { SOURCE_GROUP_COLORS } from "./revenueSourcesLogic";
import Treemap, { REVENUE_OR_VISITORS } from "./Treemap";
import {
  compareByMetric,
  type TreemapGroup,
  type TreemapMetric,
  type TreemapTile,
} from "./treemapLogic";

const TREEMAP_TOP_PAGES = 50;
const OTHER_DATUM_KEY = "__pages_other__";

/** A page's tile, or the Other tile (no `page`) for pages outside the top. */
type PageTile = TreemapTile & { page?: PageRow };

type PagesTreemapProps = {
  data: PagesResponse | undefined;
  variant?: PagesVariant;
  loading: boolean;
  /** The metric `data` is ranked by. */
  rankedBy?: TreemapMetric;
  onLeafSelect?: (datumKey: string) => void;
  topGroupColor?: string;
  loadingLabel?: string;
};

/** Stable key shared between the treemap leaf and the grid row for a page. */
export function pageRowKeyFromId(id: number): string {
  return `page:${id}`;
}

/** Stable key shared between the treemap leaf and the grid row for a page. */
export function pageRowKey(page: Pick<PageRow, "id">): string {
  return pageRowKeyFromId(page.id);
}

/** Display URL for a page (hostname + path). */
export function pageUrl(page: Pick<PageRow, "path" | "hostname">): string {
  return `${page.hostname}${page.path}`;
}

function pageValue(page: PageRow, metric: TreemapMetric): number {
  return metric === "revenue" ? page.attributed_revenue : page.visitors;
}

/**
 * The treemap's data is the first page of pages ranked by the ranking metric,
 * then the other one, so with no revenue it holds the most visited pages too.
 * Other sums only the rest of that page.
 */
function buildPageGroups(
  pages: PageRow[],
  metric: TreemapMetric,
  topGroupColor: string,
): TreemapGroup<PageTile>[] {
  const ranked = [...pages].sort(
    compareByMetric(metric, pageValue, (left, right) =>
      pageUrl(left).localeCompare(pageUrl(right)),
    ),
  );
  const otherPages = ranked.slice(TREEMAP_TOP_PAGES);
  return [
    {
      name: "Top pages",
      color: topGroupColor,
      tiles: ranked.slice(0, TREEMAP_TOP_PAGES).map((page) => ({
        datumKey: pageRowKey(page),
        label: page.path,
        value: pageValue(page, metric),
        page,
      })),
    },
    {
      name: "Other",
      color: SOURCE_GROUP_COLORS.direct,
      tiles: [
        {
          datumKey: OTHER_DATUM_KEY,
          label: "Other",
          value: otherPages.reduce((total, page) => total + pageValue(page, metric), 0),
        },
      ],
    },
  ];
}

export default function PagesTreemap(props: PagesTreemapProps) {
  const variant = () => props.variant ?? "pages";
  const isPage = (tile: PageTile) => tile.page !== undefined;

  return (
    <Treemap
      groups={(metric) =>
        props.data
          ? buildPageGroups(
              props.data.pages,
              metric,
              props.topGroupColor ?? SOURCE_GROUP_COLORS.social,
            )
          : undefined
      }
      metrics={REVENUE_OR_VISITORS}
      sliceRankedBy={props.rankedBy}
      loading={props.loading}
      loadingLabel={props.loadingLabel ?? "Loading pages treemap…"}
      height={420}
      hoverable={isPage}
      selectable={isPage}
      onSelect={(tile) => props.onLeafSelect?.(tile.datumKey)}
      wideHoverCard
      hoverCard={(tile) => (
        <Show when={tile.page}>
          {(page) => (
            <MetricHoverCardContent
              title={pageUrl(page())}
              content={buildPageHoverContent(
                page(),
                variant(),
                pageRowHighlights(page(), variant()),
              )}
              statLayout="twoColumn"
              showPeriodComparisonNote={false}
            />
          )}
        </Show>
      )}
    />
  );
}
