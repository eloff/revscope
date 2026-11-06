/// <reference types="solid-js" />
import { createEffect, createSignal, For, Show } from "solid-js";
import { formatPercentRatio, formatRevenueAmount } from "../../lib/dashboardApi";
import type {
  SearchTermLandingPath,
  SearchTermRow as SearchTermRowData,
  SearchTermsOrderBy,
  SearchTermsResponse,
  SearchTermsSortState,
} from "../../lib/searchTermsApi";
import { SEARCH_TERMS_PAGE_SIZE } from "../../lib/searchTermsApi";
import { createRowHighlight } from "./createRowHighlight";
import {
  gridLandingPathsCell,
  gridLandingPathsText,
  gridNumberCell,
  gridReferrerCell,
  gridRowHighlighted,
  gridShell,
  gridSummaryRow,
  gridTable,
} from "./grid.css";
import HighlightedValue from "./HighlightedValue";
import MetricHoverCard from "./MetricHoverCard";
import { message } from "./messages.css";
import PaginationBar from "./PaginationBar";
import SearchTermsTreemap from "./SearchTermsTreemap";
import SortableHeaderCell from "./SortableHeaderCell";
import { buildSearchTermHoverContent } from "./searchTermHoverDetails";
import {
  clickThroughRate,
  DEFAULT_SEARCH_TERMS_SORT,
  isSearchTermsTreemapSort,
  mobileShare,
  nextSearchTermsSortState,
  searchTermRowHighlights,
  searchTermRowKey,
} from "./searchTermsLogic";
import ViewDetailsButton from "./ViewDetailsButton";

export type SearchTermsProps = {
  data: SearchTermsResponse | undefined;
  treemapData: SearchTermsResponse | undefined;
  loading: boolean;
  treemapLoading: boolean;
  error: string;
  sort: SearchTermsSortState;
  onSortChange: (sort: SearchTermsSortState) => void;
  onPageChange: (offset: number) => void;
  onLandingPathSelect?: (pathId: number) => void;
};

const COLUMNS: {
  key: SearchTermsOrderBy;
  label: string;
  numeric: boolean;
  tooltip?: string;
}[] = [
  { key: "query", label: "Query", numeric: false },
  { key: "landing_paths", label: "Landing paths", numeric: false },
  {
    key: "position",
    label: "Position",
    numeric: true,
    tooltip: "The average ranking in the search results over the period for the query",
  },
  {
    key: "impressions",
    label: "Impressions",
    numeric: true,
    tooltip: "The number of people who saw your site in the search results",
  },
  {
    key: "clicks",
    label: "Clicks",
    numeric: true,
    tooltip: "The number of people who clicked on your site from the search results",
  },
  {
    key: "ctr",
    label: "CTR",
    numeric: true,
    tooltip: "The click through rate (clicks / impressions)",
  },
  {
    key: "mobile",
    label: "% Mobile",
    numeric: true,
    tooltip: "The percentage of clicks that came from mobile devices and tablets",
  },
  {
    key: "estimated_revenue",
    label: "Est. revenue",
    numeric: true,
    tooltip:
      "Estimated from total Google search attributed revenue because exact attribution is not possible",
  },
];

type SearchTermRowProps = {
  row: SearchTermRowData;
  highlighted: boolean;
  onLandingPathSelect?: (pathId: number) => void;
};

function SearchTermRow(props: SearchTermRowProps) {
  const hoverContent = () =>
    buildSearchTermHoverContent(props.row, searchTermRowHighlights(props.row));

  const renderLandingPaths = () => {
    const visiblePaths = props.row.landing_paths.slice(0, 2);
    if (visiblePaths.length === 0) {
      return "-";
    }
    return (
      <For each={visiblePaths}>
        {(entry, index) => (
          <>
            <ViewDetailsButton
              title={`View details for ${entry.path} in Landing Pages`}
              onClick={(event) => {
                event.stopPropagation();
                props.onLandingPathSelect?.(entry.path_id);
              }}
            >
              {entry.path}
            </ViewDetailsButton>
            <Show when={index() < visiblePaths.length - 1}>, </Show>
          </>
        )}
      </For>
    );
  };

  const cells = (
    <>
      <td class={gridReferrerCell}>{props.row.query}</td>
      <td class={gridLandingPathsCell} title={landingPathsTooltip(props.row.landing_paths)}>
        <HighlightedValue negative={props.row.highest_cannibalization} alignStart>
          <span class={gridLandingPathsText}>{renderLandingPaths()}</span>
        </HighlightedValue>
      </td>
      <td class={gridNumberCell}>{props.row.position.toFixed(1)}</td>
      <td class={gridNumberCell}>{props.row.impressions.toLocaleString()}</td>
      <td class={gridNumberCell}>{props.row.clicks.toLocaleString()}</td>
      <td class={gridNumberCell}>
        <HighlightedValue negative={props.row.highest_lost_clicks}>
          {formatPercentRatio(clickThroughRate(props.row), 2)}
        </HighlightedValue>
      </td>
      <td class={gridNumberCell}>{formatPercentRatio(mobileShare(props.row), 1)}</td>
      <td class={gridNumberCell}>{formatRevenueAmount(props.row.estimated_revenue)}</td>
    </>
  );

  const rowClass = () => (props.highlighted ? gridRowHighlighted : undefined);

  return (
    <MetricHoverCard
      title={props.row.query}
      content={hoverContent()}
      statLayout="threeColumn"
      onLandingPathSelect={props.onLandingPathSelect}
      renderTrigger={(mergeProps) => (
        <tr
          {...mergeProps({
            "data-record-key": searchTermRowKey(props.row),
            class: rowClass(),
          })}
        >
          {cells}
        </tr>
      )}
    />
  );
}

function landingPathsTooltip(landingPaths: SearchTermLandingPath[]): string | undefined {
  if (landingPaths.length <= 2) {
    return undefined;
  }
  const totalClicks = landingPaths.reduce((sum, entry) => sum + entry.clicks, 0);
  return landingPaths
    .map((entry) => {
      const share = totalClicks > 0 ? entry.clicks / totalClicks : 0;
      return `${entry.path} (${entry.clicks.toLocaleString()}, ${formatPercentRatio(share, 1)})`;
    })
    .join("\n");
}

export default function SearchTerms(props: SearchTermsProps) {
  let tbodyElement: HTMLTableSectionElement | undefined;
  const { highlightKey, scrollToRow } = createRowHighlight();
  const [scrollTarget, setScrollTarget] = createSignal<{
    key: string;
    id: number;
  } | null>(null);

  const requestGridScroll = (key: string) => {
    setScrollTarget({ key, id: Date.now() });
  };

  const handleSort = (column: SearchTermsOrderBy) => {
    props.onSortChange(nextSearchTermsSortState(props.sort, column));
  };

  const handleTreemapSelect = (key: string) => {
    const onCurrentPage = rows().some((row) => searchTermRowKey(row) === key);
    if (!onCurrentPage) {
      if (!isSearchTermsTreemapSort(props.sort)) {
        props.onSortChange(DEFAULT_SEARCH_TERMS_SORT);
      } else if (offset() !== 0) {
        props.onPageChange(0);
      }
    }
    requestGridScroll(key);
  };

  const showInitialLoading = () => props.loading && !props.data;
  const offset = () => props.data?.offset ?? 0;
  const limit = () => props.data?.limit ?? SEARCH_TERMS_PAGE_SIZE;
  const rows = () => props.data?.current.terms ?? [];

  createEffect(() => {
    const target = scrollTarget();
    const tableRowsData = rows();
    if (!target || !tbodyElement || showInitialLoading() || tableRowsData.length === 0) {
      return;
    }
    scrollToRow(tbodyElement, target.key, (found) => {
      if (found) {
        setScrollTarget(null);
      }
    });
  });

  const countLabel = () => {
    const count = rows().length;
    if (count === 0) {
      return "No search terms for this period";
    }
    const start = offset() + 1;
    const end = offset() + count;
    return `Showing ${start.toLocaleString()}-${end.toLocaleString()} search terms`;
  };

  return (
    <Show when={!props.error} fallback={<p class={message}>{props.error}</p>}>
      <div class={gridShell}>
        <SearchTermsTreemap
          data={props.treemapData}
          loading={props.treemapLoading && !props.treemapData}
          onLeafSelect={handleTreemapSelect}
        />
        <table class={gridTable}>
          <thead>
            <tr>
              <For each={COLUMNS}>
                {(column) => (
                  <SortableHeaderCell
                    label={column.label}
                    sortDirection={
                      props.sort.orderBy === column.key ? props.sort.direction : undefined
                    }
                    numeric={column.numeric}
                    tooltip={column.tooltip}
                    onSort={() => handleSort(column.key)}
                  />
                )}
              </For>
            </tr>
            <Show when={props.data?.current.terms.length ? props.data.current : undefined}>
              {(data) => (
                <tr class={gridSummaryRow}>
                  <th scope="row">Avg / total</th>
                  <td></td>
                  <td class={gridNumberCell}>{data().summary.position.toFixed(1)}</td>
                  <td class={gridNumberCell}>{data().summary.impressions.toLocaleString()}</td>
                  <td class={gridNumberCell}>{data().summary.clicks.toLocaleString()}</td>
                  <td class={gridNumberCell}>{formatPercentRatio(data().summary.ctr, 2)}</td>
                  <td class={gridNumberCell}>
                    {formatPercentRatio(data().summary.mobile_share, 1)}
                  </td>
                  <td class={gridNumberCell}>
                    {formatRevenueAmount(data().summary.estimated_revenue)}
                  </td>
                </tr>
              )}
            </Show>
          </thead>
          <tbody ref={tbodyElement}>
            <Show when={!showInitialLoading()}>
              <For each={rows()}>
                {(row) => (
                  <SearchTermRow
                    row={row}
                    highlighted={highlightKey() === searchTermRowKey(row)}
                    onLandingPathSelect={props.onLandingPathSelect}
                  />
                )}
              </For>
            </Show>
            <Show when={showInitialLoading()}>
              <tr>
                <td colSpan="8">Loading…</td>
              </tr>
            </Show>
          </tbody>
        </table>
        <PaginationBar
          label={countLabel()}
          loading={props.loading}
          atStart={offset() <= 0}
          atEnd={props.data?.next_offset === undefined}
          onPrevious={() => props.onPageChange(Math.max(0, offset() - limit()))}
          onNext={() => {
            if (props.data?.next_offset !== undefined) {
              props.onPageChange(props.data.next_offset);
            }
          }}
        />
      </div>
    </Show>
  );
}
