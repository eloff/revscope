/// <reference types="solid-js" />
import { createMemo, createSignal, For, Show } from "solid-js";
import { type BreakdownRow, formatPercentRatio, formatRevenueAmount } from "../../lib/dashboardApi";
import { nextColumnSort } from "./columnSort";
import { useDashboardState } from "./dashboardPeriodContext";
import {
  breakdownColumns,
  breakdownEmpty,
  breakdownFilterHint,
  breakdownFlag,
  breakdownLabel,
  breakdownLabelText,
  breakdownLegend,
  breakdownLegendRevenue,
  breakdownLegendVisitors,
  breakdownNumber,
  breakdownRevenueBar,
  breakdownRevenueNumber,
  breakdownRow,
  breakdownRows,
  breakdownSortHeader,
  breakdownSortHeaderRight,
  breakdownVisitorsBar,
} from "./OverviewBreakdowns.css";
import {
  type BreakdownColumn,
  type BreakdownSort,
  type BreakdownTab,
  breakdownRowCountry,
  breakdownRowLabel,
  DEFAULT_BREAKDOWN_SORT,
  formatBreakdownCount,
  isBreakdownRowFiltered,
  sortedBreakdownRows,
  toggleBreakdownFilter,
} from "./overviewBreakdownsLogic";
import SortHeaderButton from "./SortHeaderButton";

/** One breakdown's rows with sortable headers, scrolling within the panel's height. */
export default function BreakdownTable(props: {
  tab: BreakdownTab;
  rows: BreakdownRow[] | undefined;
  loading: boolean;
}) {
  const { segments, setFilters } = useDashboardState();
  const [sort, setSort] = createSignal<BreakdownSort>(DEFAULT_BREAKDOWN_SORT);
  const rows = createMemo(() => sortedBreakdownRows(props.rows ?? [], sort()));
  const maxVisitors = createMemo(() => Math.max(1, ...rows().map((row) => row.visitors)));
  const maxRevenue = createMemo(() => Math.max(1, ...rows().map((row) => row.revenue)));
  const toggle = (row: BreakdownRow) =>
    setFilters((previous) => ({
      ...previous,
      segments: toggleBreakdownFilter(previous.segments, props.tab.key, row),
    }));
  const header = (column: BreakdownColumn, label: string) => (
    <SortHeaderButton
      label={label}
      announceSort
      numeric={column !== "label"}
      class={column === "label" ? breakdownSortHeader : breakdownSortHeaderRight}
      sortDirection={sort().column === column ? sort().direction : undefined}
      onSort={() => setSort((current) => nextColumnSort(current, column))}
    />
  );

  return (
    <>
      <div class={breakdownColumns}>
        {header("label", props.tab.label)}
        {header("visitors", "Visitors")}
        {header("revenue", "Revenue")}
      </div>
      <div class={breakdownRows}>
        <Show
          when={rows().length > 0}
          fallback={
            <p class={breakdownEmpty}>
              {props.loading ? "Loading…" : "Nothing recorded for this period"}
            </p>
          }
        >
          <For each={rows()}>
            {(row) => {
              const label = () => breakdownRowLabel(row);
              const country = () => breakdownRowCountry(props.tab.key, row);
              const filtered = () => isBreakdownRowFiltered(segments(), props.tab.key, row);
              const tracked = row.visitors - (row.imported_visitors ?? 0);
              const detail = () =>
                `${label()}\n${row.visitors.toLocaleString()} visitors · ${formatRevenueAmount(
                  row.revenue,
                )} revenue\n${formatPercentRatio(
                  tracked > 0 ? row.converting_visitors / tracked : 0,
                )} converted · ${formatRevenueAmount(
                  tracked > 0 ? row.revenue / tracked : 0,
                )} per visitor`;
              return (
                <button
                  type="button"
                  class={breakdownRow}
                  aria-pressed={filtered()}
                  title={detail()}
                  onClick={() => toggle(row)}
                >
                  <span
                    class={breakdownVisitorsBar}
                    style={{ width: `${(row.visitors / maxVisitors()) * 100}%` }}
                  />
                  <span
                    class={breakdownRevenueBar}
                    style={{ width: `${(row.revenue / maxRevenue()) * 100}%` }}
                  />
                  <span class={breakdownLabel}>
                    <Show when={country()}>
                      {(code) => (
                        <span class={`f32 ${breakdownFlag}`} aria-hidden="true">
                          <span class={`flag ${code().toLowerCase()}`} />
                        </span>
                      )}
                    </Show>
                    <span class={breakdownLabelText}>{label()}</span>
                    <span class={breakdownFilterHint} aria-hidden="true">
                      {filtered() ? "Remove filter" : "Filter"}
                    </span>
                  </span>
                  <span class={breakdownNumber}>{formatBreakdownCount(row.visitors)}</span>
                  <span class={breakdownRevenueNumber}>{formatRevenueAmount(row.revenue)}</span>
                </button>
              );
            }}
          </For>
        </Show>
      </div>
      <div class={breakdownLegend} aria-hidden="true">
        <span>
          <span class={breakdownLegendVisitors} />
          Visitors
        </span>
        <span>
          <span class={breakdownLegendRevenue} />
          Revenue from these visitors
        </span>
      </div>
    </>
  );
}
