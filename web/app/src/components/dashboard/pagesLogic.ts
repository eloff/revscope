import type { PageRow } from "../../lib/pagesApi";
import {
  DEFAULT_PAGES_SORT,
  PAGES_SORT_COLUMNS,
  type PagesOrderBy,
  type PagesSortDirection,
  type PagesSortState,
} from "../../lib/pagesApi";
import {
  buildHoverHighlights,
  type HighlightMetric,
  type HoverHighlight,
} from "./metricHoverDetails";
import type { PagesVariant } from "./pageHoverDetails";

export type { PagesOrderBy, PagesSortDirection, PagesSortState };
export { DEFAULT_PAGES_SORT, PAGES_SORT_COLUMNS };

export type PagesSortColumn = PagesOrderBy;

/** True when the table sort matches the ranking the pages treemap uses. */
export function isPagesTreemapSort(sort: PagesSortState): boolean {
  return (
    sort.column === DEFAULT_PAGES_SORT.column && sort.direction === DEFAULT_PAGES_SORT.direction
  );
}

export function pageRowHasConversions(row: Pick<PageRow, "converting_visitors">): boolean {
  return row.converting_visitors > 0;
}

/** Hover card highlight lines for a page row flagged as best or worst in a category. */
export function pageRowHighlights(row: PageRow, variant: PagesVariant): HoverHighlight[] {
  const metrics: HighlightMetric[] = [];
  if (row.best_visitors_uplift) {
    metrics.push("visitorsUplift");
  }
  if (row.best_conversion_uplift) {
    metrics.push("conversionUplift");
  }
  if (variant === "pages" && row.best_exit_uplift) {
    metrics.push("exitUplift");
  }
  if (variant === "pages" && row.highest_exit_rate) {
    metrics.push("highestExitRate");
  }
  if (variant === "landingPages" && row.best_bounce_uplift) {
    metrics.push("bounceUplift");
  }
  if (variant === "landingPages" && row.highest_bounce_rate) {
    metrics.push("highestBounceRate");
  }
  if (row.best_session_uplift) {
    metrics.push("sessionUplift");
  }
  return buildHoverHighlights(metrics);
}

export type PagesColumnConfig = {
  key: PagesSortColumn;
  label: string;
  numeric: boolean;
  info?: { title: string; description: string };
};

export function pagesColumnsForVariant(variant: PagesVariant): PagesColumnConfig[] {
  const engagementColumn: PagesColumnConfig =
    variant === "landingPages"
      ? { key: "bounce", label: "Bounce rate", numeric: true }
      : {
          key: "exit",
          label: "Exit rate",
          numeric: true,
          info: {
            title: "Exit rate",
            description:
              "Share of visitors whose session ended on this page (their last pageview).",
          },
        };

  return [
    { key: "url", label: "URL", numeric: false },
    { key: "visitors", label: "Visitors", numeric: true },
    {
      key: "attributed_revenue",
      label: variant === "landingPages" ? "Revenue" : "Attr. revenue",
      numeric: true,
      info: {
        title: variant === "landingPages" ? "Revenue" : "Attributed revenue",
        description:
          variant === "landingPages"
            ? "Revenue from successful payments attributed to sessions that started on this landing page."
            : "Revenue weighted by this page's share of total session time before the payment. Only pageviews that occurred before payment are included.",
      },
    },
    {
      key: "conversion",
      label: "Conversion rate",
      numeric: true,
      info: {
        title: "Conversion rate",
        description:
          variant === "landingPages"
            ? "Share of visitors whose session started on this landing page and later made a purchase."
            : "Share of visitors who made a purchase after viewing this page during their session.",
      },
    },
    engagementColumn,
    variant === "landingPages"
      ? { key: "session_duration", label: "Avg session", numeric: true }
      : { key: "page_duration", label: "Avg duration", numeric: true },
    { key: "scroll", label: "Avg scroll", numeric: true },
  ];
}
