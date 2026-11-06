/// <reference types="solid-js" />
import { createEffect, createSignal, For, Show } from "solid-js";
import type {
  CrawlerDetailResponse,
  CrawlerRow,
  CrawlerSource,
  CrawlersOrderBy,
  CrawlersSortState,
} from "../../lib/crawlersApi";
import CrawlerDetailPanel from "./CrawlerDetailPanel";
import { crawlerUseragentCell } from "./CrawlersGrid.css";
import {
  CRAWLER_CATEGORY_COLORS,
  CRAWLER_CATEGORY_LABELS,
  type CrawlerCategory,
  categoryForRow,
  crawlerRowKey,
  nextCrawlersSortState,
  operatorLabelForRow,
  operatorTrustTitle,
} from "./crawlersLogic";
import { createRowHighlight } from "./createRowHighlight";
import {
  clickableRow,
  gridExpandedCell,
  gridExpandedRow,
  gridGroupBadge,
  gridNumberCell,
  gridRowHighlighted,
  gridShell,
  gridSubtableShell,
  gridTable,
} from "./grid.css";
import PaginationBar from "./PaginationBar";
import SortableHeaderCell from "./SortableHeaderCell";

type CrawlersGridProps = {
  rows: CrawlerRow[];
  loading: boolean;
  offset: number;
  limit: number;
  nextOffset?: number;
  sort: CrawlersSortState;
  onSortChange: (sort: CrawlersSortState) => void;
  onPageChange: (offset: number) => void;
  scrollTarget?: { key: string; id: number } | null;
  onScrollTargetHandled?: () => void;
  fetchDetail: (
    useragentId: number,
    source: CrawlerSource,
    verified: boolean,
  ) => Promise<CrawlerDetailResponse>;
};

const GRID_HEADERS: { key: CrawlersOrderBy; label: string; numeric: boolean }[] = [
  { key: "views", label: "Views", numeric: true },
  { key: "operator", label: "Operator", numeric: false },
  { key: "category", label: "Category", numeric: false },
  { key: "useragent", label: "User agent", numeric: false },
];

type ExpandedDetail = {
  key: string;
  data: CrawlerDetailResponse | undefined;
  loading: boolean;
  error: string;
};

function categoryBadgeStyle(category: CrawlerCategory): {
  background: string;
  color: string;
} {
  return {
    background: CRAWLER_CATEGORY_COLORS[category],
    color: "#ffffff",
  };
}

type CrawlerTableRowProps = {
  row: CrawlerRow;
  highlighted: boolean;
  expanded: boolean;
  onToggle: (row: CrawlerRow) => void;
};

function CrawlerTableRow(props: CrawlerTableRowProps) {
  const rowKey = () => crawlerRowKey(props.row);
  const category = () => categoryForRow(props.row);
  const rowClass = () =>
    [clickableRow, props.highlighted ? gridRowHighlighted : undefined].filter(Boolean).join(" ");
  return (
    <tr
      data-record-key={rowKey()}
      class={rowClass()}
      tabIndex={0}
      aria-expanded={props.expanded}
      aria-label={`${props.expanded ? "Hide" : "Show"} details for ${props.row.useragent}`}
      onClick={() => props.onToggle(props.row)}
      onKeyDown={(event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          props.onToggle(props.row);
        }
      }}
    >
      <td class={gridNumberCell}>{props.row.views.toLocaleString()}</td>
      <td title={operatorTrustTitle(props.row)}>{operatorLabelForRow(props.row)}</td>
      <td>
        <span class={gridGroupBadge} style={categoryBadgeStyle(category())}>
          {CRAWLER_CATEGORY_LABELS[category()]}
        </span>
      </td>
      <td class={crawlerUseragentCell} title={props.row.useragent}>
        {props.row.useragent}
      </td>
    </tr>
  );
}

export default function CrawlersGrid(props: CrawlersGridProps) {
  const [expanded, setExpanded] = createSignal<ExpandedDetail | null>(null);
  const { highlightKey, scrollToRow } = createRowHighlight();
  let tbodyElement: HTMLTableSectionElement | undefined;
  let detailRequestId = 0;

  const handleSort = (column: CrawlersOrderBy) => {
    props.onSortChange(nextCrawlersSortState(props.sort, column));
  };

  const loadDetail = async (row: CrawlerRow) => {
    const key = crawlerRowKey(row);
    detailRequestId += 1;
    const requestId = detailRequestId;
    setExpanded({ key, data: undefined, loading: true, error: "" });
    try {
      const data = await props.fetchDetail(row.useragent_id, row.source, row.verified);
      if (requestId === detailRequestId) {
        setExpanded({ key, data, loading: false, error: "" });
      }
    } catch (loadError) {
      if (requestId === detailRequestId) {
        setExpanded({
          key,
          data: undefined,
          loading: false,
          error: loadError instanceof Error ? loadError.message : "Unable to load crawler details.",
        });
      }
    }
  };

  const toggleRow = (row: CrawlerRow) => {
    const key = crawlerRowKey(row);
    if (expanded()?.key === key) {
      detailRequestId += 1;
      setExpanded(null);
      return;
    }
    void loadDetail(row);
  };

  createEffect<CrawlerRow[]>((previousRows) => {
    const currentRows = props.rows;
    if (currentRows !== previousRows) {
      detailRequestId += 1;
      setExpanded(null);
    }
    return currentRows;
  });

  createEffect(() => {
    const target = props.scrollTarget;
    if (!target || props.loading) {
      return;
    }
    const tbody = tbodyElement;
    if (!tbody) {
      return;
    }
    scrollToRow(tbody, target.key, () => props.onScrollTargetHandled?.());
  });

  const countLabel = () => {
    const count = props.rows.length;
    if (count === 0) {
      return "No crawlers for this period";
    }
    const start = props.offset + 1;
    const end = props.offset + count;
    return `Showing ${start.toLocaleString()}-${end.toLocaleString()} crawler${
      count === 1 ? "" : "s"
    }`;
  };

  return (
    <div class={gridShell}>
      <table class={gridTable}>
        <thead>
          <tr>
            <For each={GRID_HEADERS}>
              {(column) => (
                <SortableHeaderCell
                  label={column.label}
                  sortDirection={
                    props.sort.column === column.key ? props.sort.direction : undefined
                  }
                  numeric={column.numeric}
                  onSort={() => handleSort(column.key)}
                />
              )}
            </For>
          </tr>
        </thead>
        <tbody ref={tbodyElement}>
          <Show
            when={!props.loading}
            fallback={
              <tr>
                <td colSpan={GRID_HEADERS.length}>Loading crawlers…</td>
              </tr>
            }
          >
            <Show
              when={props.rows.length > 0}
              fallback={
                <tr>
                  <td colSpan={GRID_HEADERS.length}>No crawlers for this period.</td>
                </tr>
              }
            >
              <For each={props.rows}>
                {(row) => {
                  const rowKey = crawlerRowKey(row);
                  return (
                    <>
                      <CrawlerTableRow
                        row={row}
                        highlighted={highlightKey() === rowKey}
                        expanded={expanded()?.key === rowKey}
                        onToggle={toggleRow}
                      />
                      <Show when={expanded()?.key === rowKey}>
                        <tr class={gridExpandedRow}>
                          <td class={gridExpandedCell} colSpan={GRID_HEADERS.length}>
                            <div class={gridSubtableShell}>
                              <CrawlerDetailPanel
                                data={expanded()?.data}
                                loading={expanded()?.loading ?? false}
                                error={expanded()?.error ?? ""}
                              />
                            </div>
                          </td>
                        </tr>
                      </Show>
                    </>
                  );
                }}
              </For>
            </Show>
          </Show>
        </tbody>
      </table>
      <PaginationBar
        label={countLabel()}
        loading={props.loading}
        atStart={props.offset <= 0}
        atEnd={props.nextOffset === undefined}
        onPrevious={() => props.onPageChange(Math.max(0, props.offset - props.limit))}
        onNext={() => {
          if (props.nextOffset !== undefined) {
            props.onPageChange(props.nextOffset);
          }
        }}
      />
    </div>
  );
}
