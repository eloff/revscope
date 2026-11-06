/// <reference types="solid-js" />
import { createEffect, createMemo, createSignal, For, Show } from "solid-js";
import {
  CRAWLERS_PAGE_SIZE,
  type CrawlerDetailResponse,
  type CrawlerSource,
  type CrawlersResponse,
  type CrawlersSortState,
  DEFAULT_CRAWLERS_SORT,
} from "../../lib/crawlersApi";
import { useDashboardPreferences } from "../../lib/dashboardPreferences";
import { formatCrawlersViewNote } from "../../lib/periodComparison";
import CrawlersGrid from "./CrawlersGrid";
import CrawlersTreemap from "./CrawlersTreemap";
import {
  buildOperatorLeaves,
  CRAWLER_CATEGORY_COLORS,
  CRAWLER_CATEGORY_LABELS,
  CRAWLER_CATEGORY_ORDER,
  type CrawlerCategory,
  crawlerRowKey,
  filterRowsByCategories,
  isCrawlerCategoryEnabled,
  isCrawlersTreemapSort,
  soloCrawlerCategory,
  toggleDisabledCrawlerCategory,
  treemapOperatorLabelForRow,
} from "./crawlersLogic";
import { useDashboardState } from "./dashboardPeriodContext";
import { errorMessage, message } from "./messages.css";
import { rangeNote } from "./rangeNote.css";
import {
  summaryBar,
  summaryCard,
  summaryCardDisabled,
  summaryCardLabel,
  summaryCardMetric,
  summaryCardValue,
  summaryContent,
  summaryPanel,
} from "./summaryCards.css";

export type CrawlersProps = {
  data: CrawlersResponse | undefined;
  loading: boolean;
  error: string;
  sort: CrawlersSortState;
  onSortChange: (sort: CrawlersSortState) => void;
  onPageChange: (offset: number) => void;
  onFilterChange: () => void;
  fetchDetail: (
    useragentId: number,
    source: CrawlerSource,
    verified: boolean,
  ) => Promise<CrawlerDetailResponse>;
};

export default function Crawlers(props: CrawlersProps) {
  const { range, timeZone } = useDashboardState();
  const { disabledCrawlerCategoriesSet, updateDisabledCrawlerCategories } =
    useDashboardPreferences();
  const disabledCategories = disabledCrawlerCategoriesSet;
  const sessionNote = createMemo(() => formatCrawlersViewNote(range(), timeZone()));

  const handleCategoryClick = (category: CrawlerCategory, event: MouseEvent) => {
    updateDisabledCrawlerCategories((previous) =>
      event.metaKey || event.ctrlKey
        ? soloCrawlerCategory(category, previous)
        : toggleDisabledCrawlerCategory(category, previous),
    );
    props.onFilterChange();
  };

  const totals = () => props.data?.category_totals;
  const filteredOperators = createMemo(() => {
    const response = props.data;
    if (!response) {
      return [];
    }
    return filterRowsByCategories(response.operators ?? [], disabledCategories());
  });
  const treemapLeaves = createMemo(() =>
    props.data ? buildOperatorLeaves(filteredOperators()) : undefined,
  );

  const [scrollTarget, setScrollTarget] = createSignal<{ key: string; id: number } | null>(null);
  const [pendingOperator, setPendingOperator] = createSignal<string | null>(null);

  const handleTreemapSelect = (operatorKey: string) => {
    const match = props.data?.rows.find((row) => treemapOperatorLabelForRow(row) === operatorKey);
    if (match) {
      setPendingOperator(null);
      setScrollTarget({ key: crawlerRowKey(match), id: Date.now() });
      return;
    }
    setPendingOperator(operatorKey);
    if (!isCrawlersTreemapSort(props.sort)) {
      props.onSortChange(DEFAULT_CRAWLERS_SORT);
    } else if ((props.data?.offset ?? 0) !== 0) {
      props.onPageChange(0);
    }
  };

  createEffect(() => {
    const operator = pendingOperator();
    const rows = props.data?.rows;
    if (!operator || !rows) {
      return;
    }
    const match = rows.find((row) => treemapOperatorLabelForRow(row) === operator);
    if (match) {
      setPendingOperator(null);
      setScrollTarget({ key: crawlerRowKey(match), id: Date.now() });
      return;
    }
    if (isCrawlersTreemapSort(props.sort) && (props.data?.offset ?? 0) === 0) {
      setPendingOperator(null);
    }
  });

  const showInitialLoading = () => props.loading && !props.data;
  const offset = () => props.data?.offset ?? 0;
  const limit = () => props.data?.limit ?? CRAWLERS_PAGE_SIZE;

  return (
    <div class={summaryPanel}>
      <p class={rangeNote}>{sessionNote()}</p>
      <Show
        when={!showInitialLoading() && !props.error && totals()}
        fallback={
          <Show when={props.error}>
            <p class={errorMessage}>{props.error}</p>
          </Show>
        }
      >
        <div class={summaryBar}>
          <For each={CRAWLER_CATEGORY_ORDER}>
            {(category) => {
              const enabled = () => isCrawlerCategoryEnabled(category, disabledCategories());
              return (
                <button
                  type="button"
                  class={`${summaryCard} ${enabled() ? "" : summaryCardDisabled}`}
                  style={{
                    "border-color": CRAWLER_CATEGORY_COLORS[category],
                  }}
                  aria-pressed={enabled()}
                  title={
                    (enabled()
                      ? `Hide ${CRAWLER_CATEGORY_LABELS[category]}`
                      : `Show ${CRAWLER_CATEGORY_LABELS[category]}`) +
                    ". Ctrl+click (⌘ on Mac) to show only this category, or again to show all."
                  }
                  onClick={(event) => handleCategoryClick(category, event)}
                >
                  <span class={summaryCardLabel}>{CRAWLER_CATEGORY_LABELS[category]}</span>
                  <div class={summaryCardMetric}>
                    <span class={summaryCardValue}>
                      {(totals()?.[category] ?? 0).toLocaleString()}
                    </span>
                  </div>
                </button>
              );
            }}
          </For>
        </div>
      </Show>

      <Show when={showInitialLoading() && !props.error}>
        <p class={message}>Loading crawlers…</p>
      </Show>

      <Show when={props.data && !props.error}>
        <div class={summaryContent}>
          <CrawlersTreemap
            leaves={treemapLeaves()}
            loading={showInitialLoading()}
            onLeafSelect={handleTreemapSelect}
          />
          <CrawlersGrid
            rows={props.data?.rows ?? []}
            loading={showInitialLoading()}
            offset={offset()}
            limit={limit()}
            nextOffset={props.data?.next_offset}
            sort={props.sort}
            onSortChange={props.onSortChange}
            onPageChange={props.onPageChange}
            scrollTarget={scrollTarget()}
            onScrollTargetHandled={() => setScrollTarget(null)}
            fetchDetail={props.fetchDetail}
          />
        </div>
      </Show>
    </div>
  );
}
