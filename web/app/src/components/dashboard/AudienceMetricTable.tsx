import { createMemo, For, Show } from "solid-js";
import { CITIES_PAGE_SIZE } from "../../lib/audienceApi";
import { formatRevenueAmount } from "../../lib/dashboardApi";
import {
  type AudienceBreakdownColumn,
  type AudienceBreakdownHighlights,
  type AudienceBreakdownRow,
  type AudienceBreakdownSortState,
  isIsoCountryCode,
  sortAudienceBreakdownRows,
} from "./audienceLogic";
import CountryFlag from "./CountryFlag";
import { nextColumnSort } from "./columnSort";
import { formatWilsonScoreInterval } from "./conversionConfidence";
import {
  gridFilterActionCell,
  gridFilterButton,
  gridFilterButtonActive,
  gridIconLabelCell,
  gridNumberCell,
  gridRowHighlighted,
  gridShell,
  gridTable,
} from "./grid.css";
import HighlightedValue from "./HighlightedValue";
import { gridFilterIcon } from "./icons.css";
import PaginationBar from "./PaginationBar";
import SortableHeaderCell from "./SortableHeaderCell";

type AudienceTableValueColumn = "visitors" | "revenue" | "conversion";

export type AudienceTableColumn = {
  key: AudienceTableValueColumn;
  label: string;
  info?: { title: string; description: string };
};

export const AUDIENCE_VALUE_COLUMNS: AudienceTableColumn[] = [
  { key: "visitors", label: "Visitors" },
  { key: "revenue", label: "Revenue" },
];

/** Country table only — conversion rate is too noisy for other audience grains. */
export const COUNTRY_VALUE_COLUMNS: AudienceTableColumn[] = [
  ...AUDIENCE_VALUE_COLUMNS,
  { key: "conversion", label: "Conversion rate" },
];

function formatAudienceTableCell(
  row: AudienceBreakdownRow,
  column: AudienceTableColumn["key"],
): string {
  switch (column) {
    case "visitors":
      return row.visitors.toLocaleString();
    case "revenue":
      return formatRevenueAmount(row.revenue);
    case "conversion":
      return formatWilsonScoreInterval(row.conversionCi ?? null, 2);
  }
}

/** A trailing per-row action that adds the row as a global segment filter. */
export type AudienceRowFilter = {
  /** Rows without a filterable value get an empty cell. */
  canFilter: (row: AudienceBreakdownRow) => boolean;
  isActive: (row: AudienceBreakdownRow) => boolean;
  onFilter: (row: AudienceBreakdownRow) => void;
};

interface AudienceMetricTableProps {
  rows: AudienceBreakdownRow[];
  sort: AudienceBreakdownSortState;
  columns: AudienceTableColumn[];
  highlights?: AudienceBreakdownHighlights;
  loading: boolean;
  nameColumnLabel: string;
  countSingularLabel: string;
  countPluralLabel: string;
  emptyLabel: string;
  showCountryFlags?: boolean;
  iconClass?: (key: string) => string;
  rowFilter?: AudienceRowFilter;
  onSort: (sort: AudienceBreakdownSortState) => void;
  /** When set, hovering a row shows a cursor-anchored metrics hover card. */
  onRowHover?: (key: string, clientX: number, clientY: number) => void;
  onRowLeave?: () => void;
  highlightedKey?: string | null;
  tbodyRef?: (element: HTMLTableSectionElement) => void;
  /** Rows are already in display order from the server; skip client-side sort. */
  serverSorted?: boolean;
  offset?: number;
  limit?: number;
  nextOffset?: number;
  onPageChange?: (offset: number) => void;
}

/** A sortable audience breakdown grid: one row per country, city, browser, and so on. */
export default function AudienceMetricTable(props: AudienceMetricTableProps) {
  const sortedRows = createMemo(() =>
    props.serverSorted ? props.rows : sortAudienceBreakdownRows(props.rows, props.sort),
  );
  const sortDirectionFor = (column: AudienceBreakdownColumn) =>
    props.sort.column === column ? props.sort.direction : undefined;
  const handleSort = (column: AudienceBreakdownColumn) => {
    props.onSort(nextColumnSort(props.sort, column));
  };
  const isHighlightedCell = (row: AudienceBreakdownRow, column: AudienceTableColumn["key"]) =>
    (column === "revenue" || column === "conversion") && props.highlights?.[column] === row.key;
  const paginationEnabled = () => props.onPageChange !== undefined;
  const offset = () => props.offset ?? 0;
  const limit = () => props.limit ?? CITIES_PAGE_SIZE;
  const footerLabel = () => {
    if (props.loading) {
      return "";
    }
    const count = props.rows.length;
    if (count === 0) {
      return props.emptyLabel;
    }
    if (paginationEnabled()) {
      const start = offset() + 1;
      const end = offset() + count;
      return `Showing ${start.toLocaleString()}-${end.toLocaleString()} ${
        count === 1 ? props.countSingularLabel : props.countPluralLabel
      }`;
    }
    return `${count.toLocaleString()} ${
      count === 1 ? props.countSingularLabel : props.countPluralLabel
    }`;
  };

  return (
    <div class={gridShell}>
      <table class={gridTable}>
        <thead>
          <tr>
            <SortableHeaderCell
              label={props.nameColumnLabel}
              sortDirection={sortDirectionFor("name")}
              onSort={() => handleSort("name")}
            />
            <For each={props.columns}>
              {(column) => (
                <SortableHeaderCell
                  label={column.label}
                  sortDirection={sortDirectionFor(column.key)}
                  numeric
                  info={column.info}
                  onSort={() => handleSort(column.key)}
                />
              )}
            </For>
            <Show when={props.rowFilter}>
              <th class={gridFilterActionCell} aria-label="Filter" />
            </Show>
          </tr>
        </thead>
        <tbody ref={props.tbodyRef}>
          <Show when={!props.loading}>
            <For each={sortedRows()}>
              {(row) => (
                <tr
                  data-record-key={row.key}
                  class={props.highlightedKey === row.key ? gridRowHighlighted : undefined}
                  {...(() => {
                    const onRowHover = props.onRowHover;
                    return onRowHover
                      ? {
                          onMouseMove: (event: MouseEvent) =>
                            onRowHover(row.key, event.clientX, event.clientY),
                          onMouseLeave: () => props.onRowLeave?.(),
                        }
                      : {};
                  })()}
                >
                  <td>
                    <Show
                      when={props.iconClass}
                      fallback={
                        <Show
                          when={
                            (props.showCountryFlags && isIsoCountryCode(row.key)) ||
                            (row.flagCountryCode && isIsoCountryCode(row.flagCountryCode))
                          }
                          fallback={row.name}
                        >
                          <span class={gridIconLabelCell}>
                            <CountryFlag code={row.flagCountryCode ?? row.key} />
                            <span>{row.name}</span>
                          </span>
                        </Show>
                      }
                    >
                      {(iconClass) => (
                        <span class={gridIconLabelCell}>
                          <span class={iconClass()(row.key)} aria-hidden="true" />
                          <span>{row.name}</span>
                        </span>
                      )}
                    </Show>
                  </td>
                  <For each={props.columns}>
                    {(column) => (
                      <td class={gridNumberCell}>
                        <HighlightedValue positive={isHighlightedCell(row, column.key)}>
                          {formatAudienceTableCell(row, column.key)}
                        </HighlightedValue>
                      </td>
                    )}
                  </For>
                  <Show when={props.rowFilter}>
                    {(rowFilter) => {
                      const active = () => rowFilter().isActive(row);
                      const label = () =>
                        active() ? `Filtering by ${row.name}` : `Filter by ${row.name}`;
                      return (
                        <td class={gridFilterActionCell}>
                          <Show when={rowFilter().canFilter(row)}>
                            <button
                              type="button"
                              class={
                                active()
                                  ? `${gridFilterButton} ${gridFilterButtonActive}`
                                  : gridFilterButton
                              }
                              title={label()}
                              aria-label={label()}
                              aria-pressed={active()}
                              onClick={(event) => {
                                event.stopPropagation();
                                rowFilter().onFilter(row);
                              }}
                            >
                              <span class={gridFilterIcon} aria-hidden="true" />
                            </button>
                          </Show>
                        </td>
                      );
                    }}
                  </Show>
                </tr>
              )}
            </For>
          </Show>
          <Show when={props.loading}>
            <tr>
              <td colSpan={1 + props.columns.length + (props.rowFilter ? 1 : 0)}>Loading…</td>
            </tr>
          </Show>
        </tbody>
      </table>
      <PaginationBar
        label={footerLabel()}
        loading={props.loading}
        showButtons={paginationEnabled()}
        atStart={offset() <= 0}
        atEnd={props.nextOffset === undefined}
        onPrevious={() => props.onPageChange?.(Math.max(0, offset() - limit()))}
        onNext={() => {
          if (props.nextOffset !== undefined) {
            props.onPageChange?.(props.nextOffset);
          }
        }}
      />
    </div>
  );
}
