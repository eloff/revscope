/// <reference types="solid-js" />
import { For, Show } from "solid-js";
import { formatPercentRatio, formatRevenueAmount } from "../../lib/dashboardApi";
import {
  EVENTS_PAGE_SIZE,
  type EventRow,
  type EventsOrderBy,
  type EventsResponse,
  type EventsSortState,
} from "../../lib/eventsApi";
import type { VisitorSessionFilter } from "../../lib/visitorsApi";
import { nextColumnSort } from "./columnSort";
import { formatWilsonScoreInterval } from "./conversionConfidence";
import {
  gridFilterActionCell,
  gridFilterButton,
  gridFilterButtonActive,
  gridIconLabelCell,
  gridLandingPathsCell,
  gridLandingPathsText,
  gridNumberCell,
  gridShell,
  gridTable,
} from "./grid.css";
import { gridEventIcon, gridFilterIcon, gridGoalIcon, gridOutboundLinkIcon } from "./icons.css";
import MetricHoverCard from "./MetricHoverCard";
import { message } from "./messages.css";
import type { MetricHoverContent } from "./metricHoverDetails";
import PaginationBar from "./PaginationBar";
import SortableHeaderCell from "./SortableHeaderCell";
import ViewDetailsButton from "./ViewDetailsButton";

export type EventsProps = {
  data: EventsResponse | undefined;
  loading: boolean;
  error: string;
  sort: EventsSortState;
  onSortChange: (sort: EventsSortState) => void;
  onPageChange: (offset: number) => void;
  onPathSelect?: (pathId: number) => void;
  onVisitorsSelect?: (filter: VisitorSessionFilter) => void;
  /** Adds this row's event to the global event segment filter when provided. */
  onEventFilter?: (eventNameId: number, eventName: string) => void;
  /** Keys ({@link String} of `event_names.id`) currently filtered on. */
  activeEventFilterKeys?: Set<string>;
};

type SortColumn = EventsOrderBy;

type ColumnConfig = {
  key: SortColumn;
  label: string;
  numeric: boolean;
  info?: { title: string; description: string };
};

const COLUMNS: ColumnConfig[] = [
  { key: "event", label: "Event", numeric: false },
  { key: "page", label: "Page", numeric: false },
  { key: "visitors", label: "Visitors", numeric: true },
  {
    key: "revenue",
    label: "Revenue",
    numeric: true,
    info: {
      title: "Revenue",
      description:
        "Successful first-time subscription and single-payment revenue from visitors who triggered this event before purchasing. Payments may occur after the selected period.",
    },
  },
  { key: "conversions", label: "Conversions", numeric: true },
  {
    key: "conversion",
    label: "Conversion rate",
    numeric: true,
    info: {
      title: "Conversion rate",
      description:
        "Distinct converting visitors divided by distinct visitors who triggered this event. Shown as a 95% Wilson confidence interval.",
    },
  },
];

function pageUrl(row: EventRow): string {
  return `${row.hostname}${row.path}`;
}

function nextSortState(sort: EventsSortState, column: SortColumn): EventsSortState {
  return nextColumnSort(sort, column, column === "event" || column === "page" ? "asc" : "desc");
}

function buildEventHoverContent(row: EventRow): MetricHoverContent {
  return {
    stats: [
      {
        label: "Visitors",
        value: row.visitors.toLocaleString(),
        changeRatio: row.visitors_change.ratio,
      },
      {
        label: "Revenue",
        value: formatRevenueAmount(row.revenue),
        changeRatio: row.revenue_change.ratio,
      },
      {
        label: "Conversions",
        value: row.conversions.toLocaleString(),
        changeRatio: row.conversions_change.ratio,
      },
      {
        label: "Conversion rate",
        value: formatPercentRatio(row.conversion_rate, 2),
        changeRatio: row.conversion_rate_change.ratio,
      },
      {
        label: "95% CI",
        value: formatWilsonScoreInterval(row.conversion_ci, 2),
      },
    ],
  };
}

type EventTableRowProps = {
  row: EventRow;
  onPathSelect?: (pathId: number) => void;
  onVisitorsSelect?: (filter: VisitorSessionFilter) => void;
  onEventFilter?: (eventNameId: number, eventName: string) => void;
  activeEventFilterKeys?: Set<string>;
};

const OUTBOUND_CLICK_EVENT_NAME = "revscope-outbound-click";

function eventIconClass(row: EventRow): string {
  if (row.event_name === OUTBOUND_CLICK_EVENT_NAME) {
    return gridOutboundLinkIcon;
  }
  return row.goal_id !== undefined ? gridGoalIcon : gridEventIcon;
}

function EventTableRow(props: EventTableRowProps) {
  const title = () => `${pageUrl(props.row)} · ${props.row.event_name}`;
  const cells = (
    <>
      <td>
        <span class={gridIconLabelCell}>
          <span class={eventIconClass(props.row)} aria-hidden="true" />
          <span>{props.row.event_name}</span>
        </span>
      </td>
      <td class={gridLandingPathsCell} title={pageUrl(props.row)}>
        <Show
          when={props.onPathSelect}
          fallback={<span class={gridLandingPathsText}>{pageUrl(props.row)}</span>}
        >
          {(onPathSelect) => (
            <ViewDetailsButton
              title={`View details for ${pageUrl(props.row)} in Pages`}
              onClick={(event) => {
                event.stopPropagation();
                onPathSelect()(props.row.path_id);
              }}
            >
              {pageUrl(props.row)}
            </ViewDetailsButton>
          )}
        </Show>
      </td>
      <td class={gridNumberCell}>
        <Show when={props.onVisitorsSelect} fallback={props.row.visitors.toLocaleString()}>
          {(onVisitorsSelect) => (
            <ViewDetailsButton
              title={`View visitors for ${props.row.event_name} on ${pageUrl(props.row)}`}
              onClick={(event) => {
                event.stopPropagation();
                onVisitorsSelect()({
                  kind: "event",
                  eventNameId: props.row.event_name_id,
                  pathId: props.row.path_id,
                });
              }}
            >
              {props.row.visitors.toLocaleString()}
            </ViewDetailsButton>
          )}
        </Show>
      </td>
      <td class={gridNumberCell}>{formatRevenueAmount(props.row.revenue)}</td>
      <td class={gridNumberCell}>{props.row.conversions.toLocaleString()}</td>
      <td class={gridNumberCell}>{formatWilsonScoreInterval(props.row.conversion_ci, 2)}</td>
      <Show when={props.onEventFilter}>
        <td class={gridFilterActionCell}>
          {(() => {
            const active = () =>
              props.activeEventFilterKeys?.has(String(props.row.event_name_id)) ?? false;
            const label = () =>
              active()
                ? `Filtering by ${props.row.event_name}`
                : `Filter by ${props.row.event_name}`;
            return (
              <button
                type="button"
                class={
                  active() ? `${gridFilterButton} ${gridFilterButtonActive}` : gridFilterButton
                }
                title={label()}
                aria-label={label()}
                aria-pressed={active()}
                onClick={(event) => {
                  event.stopPropagation();
                  props.onEventFilter?.(props.row.event_name_id, props.row.event_name);
                }}
              >
                <span class={gridFilterIcon} aria-hidden="true" />
              </button>
            );
          })()}
        </td>
      </Show>
    </>
  );

  return (
    <MetricHoverCard
      title={title()}
      content={buildEventHoverContent(props.row)}
      statLayout="threeColumn"
      size="revenueSource"
      renderTrigger={(mergeProps) => <tr {...mergeProps({})}>{cells}</tr>}
    />
  );
}

export default function Events(props: EventsProps) {
  const columnCount = () => COLUMNS.length + (props.onEventFilter ? 1 : 0);
  const rows = () => props.data?.events ?? [];
  const offset = () => props.data?.offset ?? 0;
  const limit = () => props.data?.limit ?? EVENTS_PAGE_SIZE;

  const handleSort = (column: SortColumn) => {
    props.onSortChange(nextSortState(props.sort, column));
  };

  const showInitialLoading = () => props.loading && !props.data;
  const countLabel = () => {
    const count = rows().length;
    if (count === 0) {
      return "No events for this period";
    }
    const start = offset() + 1;
    const end = offset() + count;
    return `Showing ${start.toLocaleString()}-${end.toLocaleString()} event row${
      count === 1 ? "" : "s"
    }`;
  };

  return (
    <Show when={!props.error} fallback={<p class={message}>{props.error}</p>}>
      <div class={gridShell}>
        <table class={gridTable}>
          <thead>
            <tr>
              <For each={COLUMNS}>
                {(column) => (
                  <SortableHeaderCell
                    label={column.label}
                    sortDirection={
                      props.sort.column === column.key ? props.sort.direction : undefined
                    }
                    numeric={column.numeric}
                    info={column.info}
                    onSort={() => handleSort(column.key)}
                  />
                )}
              </For>
              <Show when={props.onEventFilter}>
                <th class={gridFilterActionCell} aria-label="Filter" />
              </Show>
            </tr>
          </thead>
          <tbody>
            <Show
              when={!showInitialLoading()}
              fallback={
                <tr>
                  <td colSpan={columnCount()}>Loading events…</td>
                </tr>
              }
            >
              <Show
                when={rows().length > 0}
                fallback={
                  <tr>
                    <td colSpan={columnCount()}>No events for this period.</td>
                  </tr>
                }
              >
                <For each={rows()}>
                  {(row) => (
                    <EventTableRow
                      row={row}
                      onPathSelect={props.onPathSelect}
                      onVisitorsSelect={props.onVisitorsSelect}
                      onEventFilter={props.onEventFilter}
                      activeEventFilterKeys={props.activeEventFilterKeys}
                    />
                  )}
                </For>
              </Show>
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
