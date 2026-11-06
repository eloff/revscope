/// <reference types="solid-js" />

import { createEffect, createMemo, createSignal, For, Show } from "solid-js";
import { formatRevenueAmount } from "../../lib/dashboardApi";
import { useDashboardPreferences } from "../../lib/dashboardPreferences";
import {
  CONVERSIONS_PAGE_SIZE,
  type ConversionsResponse,
  type PaymentEntry,
  type PaymentJourneyResponse,
  type PaymentsFilter,
} from "../../lib/paymentsApi";
import type { GroupSummaries } from "../../lib/revenueSourcesApi";
import { type SourceFilter, sourceFilterKey } from "../../lib/segmentFilters";
import { nextColumnSort } from "./columnSort";
import { formatSourceConversionCell } from "./conversionConfidence";
import { createRowHighlight } from "./createRowHighlight";
import {
  clickableRow,
  gridExpandedCell,
  gridExpandedRow,
  gridFilterActionCell,
  gridFilterButton,
  gridFilterButtonActive,
  gridGroupBadge,
  gridLandingPathsCell,
  gridLandingPathsText,
  gridMetricValueCell,
  gridNumberCell,
  gridReferrerCell,
  gridRowHighlighted,
  gridShell,
  gridSubtableShell,
  gridTable,
} from "./grid.css";
import HighlightedValue from "./HighlightedValue";
import { gridFilterIcon } from "./icons.css";
import MetricHoverCard from "./MetricHoverCard";
import { message } from "./messages.css";
import PaginationBar from "./PaginationBar";
import PaymentJourneyPanel from "./PaymentJourneyPanel";
import { buildBestSourceHoverContent } from "./revenueSourceHoverDetails";
import {
  type BestSourceHighlights,
  bestSourceRowHighlights,
  findBestSourceHighlights,
  GROUP_OTHER_LABEL,
  landingPathsTooltip,
  paymentFiltersForRecord,
  type RevenueSourceLinkTarget,
  type RevenueSourceRecord,
  recordSupportsConversionsDrillDown,
  revenueSourceLinkTargetForRecord,
  SOURCE_GROUP_COLORS,
  SOURCE_GROUP_LABELS,
  type SortColumn,
  type SourceGroup,
  sortRecords,
  sourceFilterFromRecord,
  sourceRowHasBestHighlight,
} from "./revenueSourcesLogic";
import SortableHeaderCell from "./SortableHeaderCell";
import ViewDetailsButton from "./ViewDetailsButton";

/** Client-side page size for the revenue-sources grid. */
const PAGE_SIZE = 100;

type RevenueSourcesGridProps = {
  records: RevenueSourceRecord[];
  /** Per-group tile metrics; used for conversion-leak warnings. */
  groups?: GroupSummaries;
  loading: boolean;
  /** When set, scroll the matching row into view and briefly highlight it. */
  scrollTarget?: { key: string; id: number } | null;
  onScrollTargetHandled?: () => void;
  onLandingPathSelect?: (pathId: number) => void;
  fetchPayments: (filters: PaymentsFilter, cursor?: string) => Promise<ConversionsResponse>;
  fetchJourney: (paymentId: number) => Promise<PaymentJourneyResponse>;
  onRevenueSourceSelect?: (target: RevenueSourceLinkTarget | null) => void;
  /** Adds the row's source to the global segment filters when provided. */
  onSourceFilter?: (source: SourceFilter) => void;
  /** Keys ({@link sourceFilterKey}) of sources currently filtered on. */
  activeSourceFilterKeys?: Set<string>;
};

const GRID_HEADERS: {
  key: SortColumn;
  label: string;
  info?: { title: string; description: string };
}[] = [
  { key: "group", label: "Group" },
  { key: "source", label: "Source" },
  { key: "visitors", label: "Visitors" },
  { key: "revenue", label: "Revenue" },
  { key: "conversion", label: "Conversion rate" },
  { key: "landingPaths", label: "Landing paths" },
];

const GRID_COLUMN_COUNT = GRID_HEADERS.length;

const RIGHT_ALIGNED_SORT_COLUMNS = new Set<SortColumn>(["visitors", "revenue", "conversion"]);

function groupBadgeStyle(group: SourceGroup): {
  background: string;
  color: string;
} {
  return {
    background: SOURCE_GROUP_COLORS[group],
    color: "#ffffff",
  };
}

function rowBorderStyle(group: SourceGroup): { "--row-border": string } {
  return { "--row-border": SOURCE_GROUP_COLORS[group] };
}

function rowHasBestHighlight(
  record: RevenueSourceRecord,
  highlights: BestSourceHighlights,
): boolean {
  return sourceRowHasBestHighlight(record.key, highlights);
}

type RevenueSourceRowProps = {
  record: RevenueSourceRecord;
  highlights: BestSourceHighlights;
  highlighted: boolean;
  expanded: boolean;
  onToggle?: (record: RevenueSourceRecord) => void;
  onLandingPathSelect?: (pathId: number) => void;
  onSourceFilter?: (source: SourceFilter) => void;
  activeSourceFilterKeys?: Set<string>;
};

function RevenueSourceRow(props: RevenueSourceRowProps) {
  const isStarred = () => rowHasBestHighlight(props.record, props.highlights);
  const drillDownEnabled = () => recordSupportsConversionsDrillDown(props.record);
  const hoverContent = () =>
    buildBestSourceHoverContent(
      props.record,
      isStarred() ? bestSourceRowHighlights(props.record, props.highlights) : undefined,
    );

  const renderLandingPaths = () => {
    const visiblePaths = props.record.landingPaths.slice(0, 2);
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
                props.onLandingPathSelect?.(entry.id);
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
      <td>
        <span class={gridGroupBadge} style={groupBadgeStyle(props.record.group)}>
          {SOURCE_GROUP_LABELS[props.record.group]}
        </span>
      </td>
      <td class={gridReferrerCell}>
        <Show when={props.record.isGroupOther} fallback={props.record.label}>
          {GROUP_OTHER_LABEL}
        </Show>
      </td>
      <td class={gridMetricValueCell}>{props.record.visitors.toLocaleString()}</td>
      <td class={gridMetricValueCell}>
        <HighlightedValue positive={props.highlights.revenue.has(props.record.key)}>
          {formatRevenueAmount(props.record.revenue)}
        </HighlightedValue>
      </td>
      <td class={gridNumberCell}>
        <HighlightedValue
          positive={props.highlights.conversion.has(props.record.key)}
          negative={props.highlights.conversionLeak.has(props.record.key)}
        >
          {formatSourceConversionCell(props.record.conversionRate, props.record.conversionCi)}
        </HighlightedValue>
      </td>
      <td class={gridLandingPathsCell} title={landingPathsTooltip(props.record.landingPaths)}>
        <span class={gridLandingPathsText}>{renderLandingPaths()}</span>
      </td>
      <Show when={props.onSourceFilter}>
        <td class={gridFilterActionCell}>
          <Show when={sourceFilterFromRecord(props.record)}>
            {(source) => {
              const active = () =>
                props.activeSourceFilterKeys?.has(sourceFilterKey(source())) ?? false;
              const label = () =>
                active() ? `Filtering by ${props.record.label}` : `Filter by ${props.record.label}`;
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
                    props.onSourceFilter?.(source());
                  }}
                >
                  <span class={gridFilterIcon} aria-hidden="true" />
                </button>
              );
            }}
          </Show>
        </td>
      </Show>
    </>
  );

  const rowClass = () =>
    [
      drillDownEnabled() ? clickableRow : undefined,
      props.highlighted ? gridRowHighlighted : undefined,
    ]
      .filter(Boolean)
      .join(" ");

  const rowTriggerProps = (
    mergeProps: (props: Record<string, unknown>) => Record<string, unknown>,
  ) => {
    const base = {
      "data-record-key": props.record.key,
      style: rowBorderStyle(props.record.group),
      class: rowClass(),
    };
    const onToggle = props.onToggle;
    if (!drillDownEnabled() || !onToggle) {
      return mergeProps(base);
    }
    return mergeProps({
      ...base,
      role: "button",
      tabIndex: 0,
      "aria-expanded": props.expanded,
      "aria-label": `${props.expanded ? "Hide" : "Show"} conversions for ${props.record.label}`,
      onClick: () => onToggle(props.record),
      onKeyDown: (event: KeyboardEvent) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          onToggle(props.record);
        }
      },
    });
  };

  return (
    <MetricHoverCard
      title={props.record.label}
      content={hoverContent()}
      statLayout="threeColumn"
      size="revenueSource"
      onLandingPathSelect={props.onLandingPathSelect}
      renderTrigger={(mergeProps) => <tr {...rowTriggerProps(mergeProps)}>{cells}</tr>}
    />
  );
}

type ExpandedPayments = {
  key: string;
  payments: PaymentEntry[] | undefined;
  nextCursor?: string;
  cursor?: string;
  cursorHistory: (string | undefined)[];
  loading: boolean;
  error: string;
};

export default function RevenueSourcesGrid(props: RevenueSourcesGridProps) {
  let tbodyElement: HTMLTableSectionElement | undefined;
  const { revenueSourceSort, setRevenueSourceSort } = useDashboardPreferences();
  const { highlightKey, scrollToRow } = createRowHighlight();
  const [pageOffset, setPageOffset] = createSignal(0);
  const [expandedPayments, setExpandedPayments] = createSignal<ExpandedPayments | null>(null);
  let paymentsRequestId = 0;
  const sortedRecords = createMemo(() => sortRecords(props.records, revenueSourceSort()));
  const pageRecords = createMemo(() => {
    const offset = pageOffset();
    return sortedRecords().slice(offset, offset + PAGE_SIZE);
  });
  const bestHighlights = createMemo(() => findBestSourceHighlights(props.records, props.groups));
  const totalCount = () => sortedRecords().length;
  const hasNextPage = () => pageOffset() + PAGE_SIZE < totalCount();
  const paginationEnabled = () => totalCount() > PAGE_SIZE;

  const handleSort = (column: SortColumn) => {
    setPageOffset(0);
    setRevenueSourceSort((previous) => nextColumnSort(previous, column));
  };

  const goToPage = (direction: "previous" | "next") => {
    paymentsRequestId += 1;
    setExpandedPayments(null);
    if (direction === "previous") {
      setPageOffset((offset) => Math.max(0, offset - PAGE_SIZE));
      return;
    }
    if (hasNextPage()) {
      setPageOffset((offset) => offset + PAGE_SIZE);
    }
  };

  const loadPaymentsForRecord = async (
    record: RevenueSourceRecord,
    cursor?: string,
    cursorHistory: (string | undefined)[] = [],
  ) => {
    const filters = paymentFiltersForRecord(record);
    if (!filters || !recordSupportsConversionsDrillDown(record)) {
      return;
    }
    const key = record.key;
    const requestId = ++paymentsRequestId;
    setExpandedPayments({
      key,
      payments: undefined,
      nextCursor: undefined,
      cursor,
      cursorHistory,
      loading: true,
      error: "",
    });
    try {
      const page = await props.fetchPayments(filters, cursor);
      if (requestId === paymentsRequestId) {
        setExpandedPayments({
          key,
          payments: page.conversions,
          nextCursor: page.next_cursor,
          cursor,
          cursorHistory,
          loading: false,
          error: "",
        });
      }
    } catch (loadError) {
      if (requestId === paymentsRequestId) {
        setExpandedPayments({
          key,
          payments: undefined,
          nextCursor: undefined,
          cursor,
          cursorHistory,
          loading: false,
          error: loadError instanceof Error ? loadError.message : "Unable to load payments.",
        });
      }
    }
  };

  const handlePaymentsPageChange = (direction: "previous" | "next") => {
    const expanded = expandedPayments();
    if (!expanded) {
      return;
    }
    const record = props.records.find((entry) => entry.key === expanded.key);
    if (!record) {
      return;
    }
    if (direction === "previous") {
      if (expanded.cursorHistory.length === 0) {
        return;
      }
      const previous = expanded.cursorHistory[expanded.cursorHistory.length - 1];
      void loadPaymentsForRecord(record, previous, expanded.cursorHistory.slice(0, -1));
      return;
    }
    if (!expanded.nextCursor) {
      return;
    }
    void loadPaymentsForRecord(record, expanded.nextCursor, [
      ...expanded.cursorHistory,
      expanded.cursor,
    ]);
  };

  const togglePayments = (record: RevenueSourceRecord) => {
    const key = record.key;
    if (expandedPayments()?.key === key) {
      paymentsRequestId += 1;
      setExpandedPayments(null);
      props.onRevenueSourceSelect?.(null);
      return;
    }
    const target = revenueSourceLinkTargetForRecord(record);
    if (target) {
      props.onRevenueSourceSelect?.(target);
      return;
    }
    void loadPaymentsForRecord(record);
  };

  const expandRecordByKey = (key: string) => {
    const record = props.records.find((entry) => entry.key === key);
    if (!record || !recordSupportsConversionsDrillDown(record)) {
      return;
    }
    if (expandedPayments()?.key !== key) {
      void loadPaymentsForRecord(record);
    }
  };

  createEffect<RevenueSourceRecord[]>((previousRecords) => {
    const currentRecords = props.records;
    if (currentRecords !== previousRecords) {
      paymentsRequestId += 1;
      setExpandedPayments(null);
      // Keep the current page when jumping to a scroll target; otherwise reset.
      if (!props.scrollTarget) {
        setPageOffset(0);
      }
    }
    return currentRecords;
  });

  createEffect(() => {
    const sorted = sortedRecords();
    const maxOffset =
      sorted.length === 0 ? 0 : Math.floor((sorted.length - 1) / PAGE_SIZE) * PAGE_SIZE;
    if (pageOffset() > maxOffset) {
      setPageOffset(maxOffset);
    }
  });

  createEffect(() => {
    const target = props.scrollTarget;
    if (!target || props.loading) {
      return;
    }
    const index = sortedRecords().findIndex((record) => record.key === target.key);
    if (index < 0) {
      props.onScrollTargetHandled?.();
      return;
    }
    const targetOffset = Math.floor(index / PAGE_SIZE) * PAGE_SIZE;
    if (pageOffset() !== targetOffset) {
      setPageOffset(targetOffset);
      return;
    }
    expandRecordByKey(target.key);
    const tbody = tbodyElement;
    if (!tbody) {
      return;
    }
    scrollToRow(tbody, target.key, () => props.onScrollTargetHandled?.());
  });

  const columnCount = () => GRID_COLUMN_COUNT + (props.onSourceFilter ? 1 : 0);

  const sourceCount = () => props.records.filter((record) => !record.isGroupOther).length;

  const countLabel = () => {
    const count = sourceCount();
    if (count === 0) {
      return "No sources for this period";
    }
    const total = totalCount();
    if (!paginationEnabled()) {
      return `${count.toLocaleString()} source${count === 1 ? "" : "s"}`;
    }
    const start = pageOffset() + 1;
    const end = Math.min(pageOffset() + PAGE_SIZE, total);
    return `Showing ${start.toLocaleString()}-${end.toLocaleString()} of ${total.toLocaleString()} sources`;
  };

  return (
    <div class={gridShell}>
      <table class={gridTable}>
        <thead>
          <tr>
            <For each={GRID_HEADERS}>
              {(column) => {
                const rightAligned = RIGHT_ALIGNED_SORT_COLUMNS.has(column.key);
                return (
                  <SortableHeaderCell
                    label={column.label}
                    sortDirection={
                      revenueSourceSort().column === column.key
                        ? revenueSourceSort().direction
                        : undefined
                    }
                    numeric={rightAligned}
                    info={column.info}
                    onSort={() => handleSort(column.key)}
                  />
                );
              }}
            </For>
            <Show when={props.onSourceFilter}>
              <th class={gridFilterActionCell} aria-label="Filter" />
            </Show>
          </tr>
        </thead>
        <tbody ref={tbodyElement}>
          <Show when={!props.loading}>
            <For each={pageRecords()}>
              {(record) => {
                const recordKey = () => record.key;
                return (
                  <>
                    <RevenueSourceRow
                      record={record}
                      highlights={bestHighlights()}
                      highlighted={highlightKey() === recordKey()}
                      expanded={expandedPayments()?.key === recordKey()}
                      onToggle={togglePayments}
                      onLandingPathSelect={props.onLandingPathSelect}
                      onSourceFilter={props.onSourceFilter}
                      activeSourceFilterKeys={props.activeSourceFilterKeys}
                    />
                    <Show
                      when={
                        recordSupportsConversionsDrillDown(record) &&
                        expandedPayments()?.key === recordKey()
                      }
                    >
                      <tr class={gridExpandedRow}>
                        <td class={gridExpandedCell} colSpan={columnCount()}>
                          <div class={gridSubtableShell}>
                            <Show
                              when={!expandedPayments()?.error}
                              fallback={<p class={message}>{expandedPayments()?.error}</p>}
                            >
                              <PaymentJourneyPanel
                                payments={expandedPayments()?.payments}
                                loading={expandedPayments()?.loading ?? false}
                                loadingLabel="Loading conversions…"
                                emptyLabel={`No conversions for ${record.label}`}
                                offset={
                                  (expandedPayments()?.cursorHistory.length ?? 0) *
                                  CONVERSIONS_PAGE_SIZE
                                }
                                hasNextPage={expandedPayments()?.nextCursor !== undefined}
                                onPageChange={handlePaymentsPageChange}
                                fetchJourney={props.fetchJourney}
                                onRevenueSourceSelect={props.onRevenueSourceSelect}
                              />
                            </Show>
                          </div>
                        </td>
                      </tr>
                    </Show>
                  </>
                );
              }}
            </For>
          </Show>
          <Show when={props.loading}>
            <tr>
              <td colSpan={columnCount()}>Loading…</td>
            </tr>
          </Show>
        </tbody>
      </table>
      <PaginationBar
        label={countLabel()}
        loading={props.loading}
        showButtons={paginationEnabled()}
        atStart={pageOffset() <= 0}
        atEnd={!hasNextPage()}
        onPrevious={() => goToPage("previous")}
        onNext={() => goToPage("next")}
      />
    </div>
  );
}
