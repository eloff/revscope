/// <reference types="solid-js" />
import { For, type JSX, onMount, Show } from "solid-js";
import { formatSessionDuration } from "../../lib/dashboardApi";
import type { VisitorEntry, VisitorOrderBy, VisitorSortState } from "../../lib/visitorsApi";
import { ensureWorldFlagsSpriteStylesheet } from "../../lib/worldFlagsSprite";
import CountryLinkCell, { visitorLocationLabel } from "./CountryLinkCell";
import { formatPaidAt, formatTimeToPurchaseDuration } from "./conversionsLogic";
import { useDashboardState } from "./dashboardPeriodContext";
import { clickableRow, gridNumberCell, gridReferrerCell, gridShell, gridTable } from "./grid.css";
import PaginationBar from "./PaginationBar";
import type { RevenueSourceLinkTarget } from "./revenueSourcesLogic";
import SortableHeaderCell from "./SortableHeaderCell";
import SourceLinkCell from "./SourceLinkCell";

export type VisitorsTableProps = {
  visitors: VisitorEntry[] | undefined;
  loading: boolean;
  emptyLabel: string;
  loadingLabel?: string;
  offset: number;
  limit: number;
  nextOffset?: number;
  sort: VisitorSortState;
  onSortChange: (sort: VisitorSortState) => void;
  onPageChange: (offset: number) => void;
  onVisitorSelect?: (visitor: VisitorEntry) => void;
  onRevenueSourceSelect?: (target: RevenueSourceLinkTarget) => void;
  onCountrySelect?: (countryCode: string) => void;
};

const COLUMNS: {
  key: VisitorOrderBy;
  label: string;
  numeric: boolean;
}[] = [
  { key: "source", label: "Source", numeric: false },
  { key: "last_view_at", label: "Last View At", numeric: true },
  { key: "pages_viewed", label: "Pages Viewed", numeric: true },
  { key: "active_duration", label: "Active Duration", numeric: true },
  { key: "session_age", label: "Session Age", numeric: true },
];

const RIGHT_ALIGNED_COLUMNS = new Set<VisitorOrderBy>([
  "last_view_at",
  "active_duration",
  "session_age",
  "pages_viewed",
]);

const TABLE_COLUMN_COUNT = COLUMNS.length + 1;

function renderVisitorLocation(
  visitor: VisitorEntry,
  onCountrySelect?: (countryCode: string) => void,
): string | JSX.Element {
  const label = visitorLocationLabel(visitor.country, visitor.city);
  if (!label) {
    return "—";
  }
  return (
    <CountryLinkCell country={visitor.country} label={label} onCountrySelect={onCountrySelect} />
  );
}

function nextSortState(current: VisitorSortState, column: VisitorOrderBy): VisitorSortState {
  if (current.orderBy === column) {
    return {
      orderBy: column,
      direction: current.direction === "desc" ? "asc" : "desc",
    };
  }
  return { orderBy: column, direction: "desc" };
}

function visitorSessionAgeSeconds(visitor: VisitorEntry): number {
  return Math.max(0, visitor.last_view_at - visitor.viewed_at);
}

function visitorLabel(visitor: VisitorEntry): string {
  return `visitor ${visitor.visitor_timestamp_id}`;
}

export default function VisitorsTable(props: VisitorsTableProps) {
  const { timeZone } = useDashboardState();

  onMount(() => {
    ensureWorldFlagsSpriteStylesheet();
  });

  const handleSort = (column: VisitorOrderBy) => {
    props.onSortChange(nextSortState(props.sort, column));
  };

  const showInitialLoading = () => props.loading && !props.visitors;

  const countLabel = () => {
    const count = props.visitors?.length ?? 0;
    if (count === 0) {
      return props.emptyLabel;
    }
    const start = props.offset + 1;
    const end = props.offset + count;
    return `Showing ${start.toLocaleString()}-${end.toLocaleString()} visitors`;
  };

  return (
    <div class={gridShell}>
      <table class={gridTable}>
        <thead>
          <tr>
            <th>Location</th>
            <For each={COLUMNS}>
              {(column) => (
                <SortableHeaderCell
                  label={column.label}
                  sortDirection={
                    props.sort.orderBy === column.key ? props.sort.direction : undefined
                  }
                  numeric={RIGHT_ALIGNED_COLUMNS.has(column.key)}
                  onSort={() => handleSort(column.key)}
                />
              )}
            </For>
          </tr>
        </thead>
        <tbody>
          <Show when={!showInitialLoading()}>
            <For each={props.visitors ?? []}>
              {(visitor) => (
                <tr
                  class={props.onVisitorSelect ? clickableRow : undefined}
                  role={props.onVisitorSelect ? "button" : undefined}
                  tabIndex={props.onVisitorSelect ? 0 : undefined}
                  aria-label={
                    props.onVisitorSelect ? `View journey for ${visitorLabel(visitor)}` : undefined
                  }
                  onClick={() => props.onVisitorSelect?.(visitor)}
                  onKeyDown={(event) => {
                    if (props.onVisitorSelect && (event.key === "Enter" || event.key === " ")) {
                      event.preventDefault();
                      props.onVisitorSelect(visitor);
                    }
                  }}
                >
                  <td>{renderVisitorLocation(visitor, props.onCountrySelect)}</td>
                  <td class={gridReferrerCell}>
                    <SourceLinkCell
                      landing={visitor}
                      onRevenueSourceSelect={props.onRevenueSourceSelect}
                    />
                  </td>
                  <td class={gridNumberCell}>{formatPaidAt(visitor.last_view_at, timeZone())}</td>
                  <td class={gridNumberCell}>{visitor.session_page_count.toLocaleString()}</td>
                  <td class={gridNumberCell}>
                    {formatSessionDuration(visitor.session_duration_seconds)}
                  </td>
                  <td class={gridNumberCell}>
                    {formatTimeToPurchaseDuration(visitorSessionAgeSeconds(visitor))}
                  </td>
                </tr>
              )}
            </For>
          </Show>
          <Show when={showInitialLoading()}>
            <tr>
              <td colSpan={TABLE_COLUMN_COUNT}>{props.loadingLabel ?? "Loading…"}</td>
            </tr>
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
