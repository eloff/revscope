import { createEffect, createMemo, For, Show, untrack } from "solid-js";
import { useDashboardPreferences } from "../../lib/dashboardPreferences";
import { PAGES_PAGE_SIZE, type PageRow, type PagesResponse } from "../../lib/pagesApi";
import type { PaymentJourneyResponse } from "../../lib/paymentsApi";
import type { VisitorJourneyResponse } from "../../lib/visitorsApi";
import { nextColumnSort } from "./columnSort";
import { createRowHighlight } from "./createRowHighlight";
import { gridFilterActionCell, gridShell, gridTable } from "./grid.css";
import { message } from "./messages.css";
import PageDrilldownRow from "./PageDrilldownRow";
import PagesTreemap, { pageRowKey, pageRowKeyFromId } from "./PagesTreemap";
import PageTableRow from "./PageTableRow";
import PaginationBar from "./PaginationBar";
import { createPageDrilldown, type PageDrilldownFetchers } from "./pageDrilldownState";
import {
  DEFAULT_PAGES_SORT,
  isPagesTreemapSort,
  type PagesSortColumn,
  pagesColumnsForVariant,
} from "./pagesLogic";
import { type RevenueSourceLinkTarget, SOURCE_GROUP_COLORS } from "./revenueSourcesLogic";
import SortableHeaderCell from "./SortableHeaderCell";

export type PagesGridProps = PageDrilldownFetchers & {
  data: PagesResponse | undefined;
  treemapData: PagesResponse | undefined;
  loading: boolean;
  treemapLoading: boolean;
  error: string;
  variant?: "pages" | "landingPages";
  scrollTarget?: { pathId: number; id: number } | null;
  onPathSelect?: (pathId: number | null) => void;
  onTransitionPathSelect?: (pathId: number) => void;
  fetchJourney: (paymentId: number) => Promise<PaymentJourneyResponse>;
  fetchVisitorJourney: (visitorTimestampId: string) => Promise<VisitorJourneyResponse>;
  onRevenueSourceSelect?: (target: RevenueSourceLinkTarget) => void;
  onVisitorPathSelect?: (pathId: number) => void;
  onVisitorLandingPathSelect?: (pathId: number) => void;
  onCountrySelect?: (countryCode: string) => void;
  onPageFilter?: (pathId: number, path: string) => void;
  activePageFilterKeys?: Set<string>;
  onPageChange: (offset: number) => void;
  onSortChange: () => void;
};

/** One pages mode: a treemap over a sortable, paginated grid with per-row drilldowns. */
export default function PagesGrid(props: PagesGridProps) {
  let tbodyElement: HTMLTableSectionElement | undefined;
  const { pagesSort, setPagesSort, landingPagesSort, setLandingPagesSort } =
    useDashboardPreferences();
  const { highlightKey, scrollToRow } = createRowHighlight();
  const variant = () => props.variant ?? "pages";
  const drilldown = createPageDrilldown(props, variant);
  const currentSort = () => (variant() === "landingPages" ? landingPagesSort() : pagesSort());
  const columns = createMemo(() => pagesColumnsForVariant(variant()));
  const columnCount = () => columns().length + (props.onPageFilter ? 1 : 0);

  const effectiveScrollTarget = () => {
    const external = props.scrollTarget;
    if (!external) {
      return null;
    }
    return {
      key: pageRowKeyFromId(external.pathId),
      id: external.id,
    };
  };

  const handleSort = (column: PagesSortColumn) => {
    if (variant() === "landingPages") {
      setLandingPagesSort((previous) => nextColumnSort(previous, column));
    } else {
      setPagesSort((previous) => nextColumnSort(previous, column));
    }
    props.onSortChange();
  };

  const expandRowByKey = (key: string) => {
    const row = props.data?.pages.find((page) => pageRowKey(page) === key);
    if (!row || row.visitors <= 0) {
      return;
    }
    // Untracked, or collapsing a row while its target is still set would reopen it.
    if (untrack(drilldown.expanded)?.key !== key) {
      void drilldown.showTransitions(row);
    }
  };

  const toggleDrilldown = (row: PageRow) => {
    const key = pageRowKey(row);
    if (drilldown.expanded()?.key === key) {
      drilldown.collapse();
      props.onPathSelect?.(null);
      return;
    }
    if (props.onPathSelect) {
      props.onPathSelect(row.id);
      return;
    }
    if (row.visitors <= 0) {
      return;
    }
    void drilldown.showTransitions(row);
  };

  const handleTreemapSelect = (key: string) => {
    const treemapPages = props.treemapData?.pages ?? props.data?.pages;
    const row = treemapPages?.find((page) => pageRowKey(page) === key);
    if (!row) {
      return;
    }
    const onCurrentPage = props.data?.pages.some((page) => pageRowKey(page) === key) ?? false;
    if (!onCurrentPage) {
      if (!isPagesTreemapSort(currentSort())) {
        if (variant() === "landingPages") {
          setLandingPagesSort(DEFAULT_PAGES_SORT);
        } else {
          setPagesSort(DEFAULT_PAGES_SORT);
        }
        props.onSortChange();
      } else if ((props.data?.offset ?? 0) !== 0) {
        props.onPageChange(0);
      }
    }
    if (props.onPathSelect) {
      props.onPathSelect(row.id);
      return;
    }
    if (row.visitors <= 0) {
      return;
    }
    void drilldown.showTransitions(row);
  };

  const rows = () => props.data?.pages ?? [];
  const offset = () => props.data?.offset ?? 0;
  const limit = () => props.data?.limit ?? PAGES_PAGE_SIZE;

  const showInitialLoading = () => props.loading && !props.data;

  // One effect, so new data always collapses before the URL's row reopens: as two, Solid
  // can run them in either order and leave that row closed.
  createEffect<PagesResponse | undefined>((previousData) => {
    const currentData = props.data;
    if (currentData !== previousData) {
      drilldown.collapse();
    }
    const target = effectiveScrollTarget();
    if (target && !showInitialLoading()) {
      expandRowByKey(target.key);
    }
    return currentData;
  });

  createEffect(() => {
    const target = effectiveScrollTarget();
    const tbody = tbodyElement;
    if (!target || !tbody || showInitialLoading()) {
      return;
    }
    scrollToRow(tbody, target.key);
  });

  const countLabel = () => {
    const count = rows().length;
    if (count === 0) {
      return variant() === "landingPages"
        ? "No landing pages for this period"
        : "No pages for this period";
    }
    const start = offset() + 1;
    const end = offset() + count;
    const noun = variant() === "landingPages" ? "landing page" : "page";
    return `Showing ${start.toLocaleString()}-${end.toLocaleString()} ${
      count === 1 ? noun : `${noun}s`
    }`;
  };

  const handleTransitionPathSelect = (pathId: number) => {
    if (props.onTransitionPathSelect) {
      props.onTransitionPathSelect(pathId);
      return;
    }
    if (props.onPathSelect) {
      props.onPathSelect(pathId);
      return;
    }
    const row = props.data?.pages.find((page) => page.id === pathId);
    if (row && row.visitors > 0) {
      void drilldown.showTransitions(row);
    }
  };

  return (
    <Show when={!props.error} fallback={<p class={message}>{props.error}</p>}>
      <div class={gridShell}>
        <PagesTreemap
          data={props.treemapData}
          variant={variant()}
          loading={props.treemapLoading && !props.treemapData}
          onLeafSelect={handleTreemapSelect}
          topGroupColor={
            variant() === "landingPages"
              ? SOURCE_GROUP_COLORS.referrers
              : SOURCE_GROUP_COLORS.social
          }
          labels={
            variant() === "landingPages"
              ? {
                  loading: "Loading landing pages treemap…",
                  otherTooltipNoun: "landing pages",
                }
              : undefined
          }
        />
        <table class={gridTable}>
          <thead>
            <tr>
              <For each={columns()}>
                {(column) => (
                  <SortableHeaderCell
                    label={column.label}
                    sortDirection={
                      currentSort().column === column.key ? currentSort().direction : undefined
                    }
                    numeric={column.numeric}
                    info={column.info}
                    onSort={() => handleSort(column.key)}
                  />
                )}
              </For>
              <Show when={props.onPageFilter}>
                <th class={gridFilterActionCell} aria-label="Filter" />
              </Show>
            </tr>
          </thead>
          <tbody ref={tbodyElement}>
            <Show when={!showInitialLoading()}>
              <For each={rows()}>
                {(row) => {
                  const rowKey = pageRowKey(row);
                  return (
                    <>
                      <PageTableRow
                        row={row}
                        variant={variant()}
                        highlighted={highlightKey() === rowKey}
                        expanded={drilldown.expanded()?.key === rowKey}
                        onToggle={(selectedRow) => void toggleDrilldown(selectedRow)}
                        onPageFilter={props.onPageFilter}
                        activePageFilterKeys={props.activePageFilterKeys}
                      />
                      <Show when={drilldown.expanded()?.key === rowKey}>
                        <PageDrilldownRow
                          row={row}
                          colSpan={columnCount()}
                          variant={variant()}
                          drilldown={drilldown}
                          onTransitionPathSelect={handleTransitionPathSelect}
                          fetchJourney={props.fetchJourney}
                          fetchVisitorJourney={props.fetchVisitorJourney}
                          onRevenueSourceSelect={props.onRevenueSourceSelect}
                          onVisitorPathSelect={props.onVisitorPathSelect}
                          onVisitorLandingPathSelect={props.onVisitorLandingPathSelect}
                          onCountrySelect={props.onCountrySelect}
                        />
                      </Show>
                    </>
                  );
                }}
              </For>
            </Show>
            <Show when={showInitialLoading()}>
              <tr>
                <td colSpan={columnCount()}>Loading…</td>
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
