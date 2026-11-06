/// <reference types="solid-js" />
import { createMemo, createSignal, For, type JSX, Show } from "solid-js";
import { formatRevenueAmount } from "../../lib/dashboardApi";
import { useDashboardPreferences } from "../../lib/dashboardPreferences";
import type {
  ConversionsResponse,
  PaymentJourneyResponse,
  PaymentsFilter,
} from "../../lib/paymentsApi";
import { formatRevenueSourcesSessionNote } from "../../lib/periodComparison";
import type { RevenueSourceResponse } from "../../lib/revenueSourcesApi";
import type { SourceFilter } from "../../lib/segmentFilters";
import { gridChangeClass } from "./changeTone";
import { useDashboardState } from "./dashboardPeriodContext";
import MetricHoverCard from "./MetricHoverCard";
import { errorMessage, message } from "./messages.css";
import RevenueSourcesGrid from "./RevenueSourcesGrid";
import RevenueSourcesTreemap from "./RevenueSourcesTreemap";
import { rangeNote } from "./rangeNote.css";
import { buildGroupSummaryHoverContent } from "./revenueSourceHoverDetails";
import {
  buildGroupOtherRecords,
  buildTreemapLeaves,
  filterRecordsByGroups,
  findBestSourceHighlights,
  formatTileChangeRatio,
  groupTotalsFromResponse,
  isSourceGroupEnabled,
  type RevenueSourceLinkTarget,
  type RevenueSourceRecord,
  revenueSourceLinkTargetForRecord,
  rowsFromResponse,
  SOURCE_GROUP_COLORS,
  SOURCE_GROUP_LABELS,
  SOURCE_GROUP_ORDER,
  type SourceGroup,
  soloSourceGroup,
  takeTopSourcesByRevenue,
  toggleDisabledSourceGroup,
} from "./revenueSourcesLogic";
import {
  summaryBar,
  summaryCard,
  summaryCardChange,
  summaryCardDisabled,
  summaryCardLabel,
  summaryCardMetric,
  summaryCardSecondaryValue,
  summaryCardValue,
  summaryContent,
  summaryPanel,
} from "./summaryCards.css";

/** Treemap shows at most this many individual sources (by revenue), plus per-group Other. */
const TREEMAP_TOP_SOURCES = 50;

function summaryChangeClass(ratio: number): string {
  return `${summaryCardChange} ${gridChangeClass(ratio)}`;
}

function SummaryMetricChange(props: { ratio: number | null | undefined }): JSX.Element | null {
  const ratio = props.ratio;
  if (ratio == null) {
    return null;
  }
  const changeText = formatTileChangeRatio(ratio);
  if (!changeText) {
    return null;
  }
  return <span class={summaryChangeClass(ratio)}>{changeText}</span>;
}

export type RevenueSourcesProps = {
  data: RevenueSourceResponse | undefined;
  loading: boolean;
  error: string;
  scrollTarget?: { key: string; id: number } | null;
  onLandingPathSelect?: (pathId: number) => void;
  fetchPayments: (filters: PaymentsFilter, cursor?: string) => Promise<ConversionsResponse>;
  fetchJourney: (paymentId: number) => Promise<PaymentJourneyResponse>;
  onRevenueSourceSelect?: (target: RevenueSourceLinkTarget | null) => void;
  /** Adds a row's source to the global segment filters when provided. */
  onSourceFilter?: (source: SourceFilter) => void;
  /** Keys of sources currently filtered on, for the active button state. */
  activeSourceFilterKeys?: Set<string>;
};

export default function RevenueSources(props: RevenueSourcesProps) {
  const { range, timeZone } = useDashboardState();
  const { disabledSourceGroupsSet, updateDisabledSourceGroups } = useDashboardPreferences();
  const disabledGroups = disabledSourceGroupsSet;
  const sessionNote = createMemo(() => formatRevenueSourcesSessionNote(range(), timeZone()));

  const handleGroupClick = (group: SourceGroup, event: MouseEvent) => {
    updateDisabledSourceGroups((previous) =>
      event.metaKey || event.ctrlKey
        ? soloSourceGroup(group, previous)
        : toggleDisabledSourceGroup(group, previous),
    );
  };

  const groupTotals = createMemo(() => {
    const response = props.data;
    return response ? groupTotalsFromResponse(response.current) : undefined;
  });

  const groupSummaries = createMemo(() => props.data?.groups);

  const treemapRecords = createMemo(() => {
    const response = props.data;
    if (!response) {
      return [];
    }
    return filterRecordsByGroups(rowsFromResponse(response.current), disabledGroups());
  });

  const treemapLeaves = createMemo(() => {
    const totals = groupTotals();
    if (!totals) {
      return undefined;
    }
    return buildTreemapLeaves(treemapRecords(), totals, TREEMAP_TOP_SOURCES, disabledGroups());
  });

  const gridRecords = createMemo(() => {
    const response = props.data;
    const totals = groupTotals();
    if (!response || !totals) {
      return [];
    }
    const filtered = filterRecordsByGroups(rowsFromResponse(response.current), disabledGroups());
    const topDisplayed = takeTopSourcesByRevenue(treemapRecords(), TREEMAP_TOP_SOURCES);
    return [...filtered, ...buildGroupOtherRecords(topDisplayed, totals, disabledGroups())];
  });

  const recordByKey = createMemo(() => {
    const map = new Map<string, RevenueSourceRecord>();
    for (const record of gridRecords()) {
      map.set(record.key, record);
    }
    return map;
  });

  const bestHighlights = createMemo(() =>
    findBestSourceHighlights(gridRecords(), groupSummaries()),
  );

  const [scrollTarget, setScrollTarget] = createSignal<{
    key: string;
    id: number;
  } | null>(null);

  const gridScrollTarget = () => props.scrollTarget ?? scrollTarget() ?? null;

  const requestGridScroll = (key: string) => {
    setScrollTarget({ key, id: Date.now() });
  };

  const handleTreemapSelect = (key: string) => {
    const record = recordByKey().get(key);
    if (!record) {
      return;
    }
    const target = revenueSourceLinkTargetForRecord(record);
    if (target) {
      setScrollTarget(null);
      props.onRevenueSourceSelect?.(target);
      return;
    }
    props.onRevenueSourceSelect?.(null);
    requestGridScroll(key);
  };

  const showInitialLoading = () => props.loading && !props.data;

  return (
    <div class={summaryPanel}>
      <p class={rangeNote}>{sessionNote()}</p>
      <Show
        when={!showInitialLoading() && !props.error && groupTotals()}
        fallback={
          <Show when={props.error}>
            <p class={errorMessage}>{props.error}</p>
          </Show>
        }
      >
        <div class={summaryBar}>
          <For each={SOURCE_GROUP_ORDER}>
            {(group: SourceGroup) => {
              const summary = () => groupSummaries()?.[group];
              const enabled = () => isSourceGroupEnabled(group, disabledGroups());
              const revenueChange = () => {
                const s = summary();
                return s ? s.revenue_change.ratio : undefined;
              };
              const visitorsChange = () => {
                const s = summary();
                return s ? s.visitors_change.ratio : undefined;
              };
              const hoverContent = () => {
                const s = summary();
                return s ? buildGroupSummaryHoverContent(s) : undefined;
              };

              return (
                <MetricHoverCard
                  title={SOURCE_GROUP_LABELS[group]}
                  content={hoverContent()}
                  statLayout="threeColumn"
                  size="revenueSource"
                  renderTrigger={(mergeProps) => (
                    <button
                      {...mergeProps({
                        type: "button",
                        class: `${summaryCard} ${enabled() ? "" : summaryCardDisabled}`,
                        style: {
                          "border-color": SOURCE_GROUP_COLORS[group],
                        },
                        "aria-pressed": enabled(),
                        title:
                          (enabled()
                            ? `Hide ${SOURCE_GROUP_LABELS[group]} sources`
                            : `Show ${SOURCE_GROUP_LABELS[group]} sources`) +
                          ". Ctrl+click (⌘ on Mac) to show only this group, or again to show all.",
                        onClick: (event: MouseEvent) => handleGroupClick(group, event),
                      })}
                    >
                      <span class={summaryCardLabel}>{SOURCE_GROUP_LABELS[group]}</span>
                      <div class={summaryCardMetric}>
                        <span class={summaryCardValue}>
                          {formatRevenueAmount(summary()?.revenue ?? 0)}
                        </span>
                        <SummaryMetricChange ratio={revenueChange()} />
                      </div>
                      <div class={summaryCardMetric}>
                        <span class={summaryCardSecondaryValue}>
                          {(summary()?.visitors ?? 0).toLocaleString()}
                        </span>
                        <SummaryMetricChange ratio={visitorsChange()} />
                      </div>
                    </button>
                  )}
                />
              );
            }}
          </For>
        </div>
      </Show>

      <Show when={showInitialLoading() && !props.error}>
        <p class={message}>Loading sources…</p>
      </Show>

      <Show when={props.data && !props.error}>
        <div class={summaryContent}>
          <RevenueSourcesTreemap
            leaves={treemapLeaves()}
            recordByKey={recordByKey()}
            bestHighlights={bestHighlights()}
            loading={showInitialLoading()}
            error={props.error}
            onLeafSelect={handleTreemapSelect}
          />

          <RevenueSourcesGrid
            records={gridRecords()}
            groups={groupSummaries()}
            loading={showInitialLoading()}
            scrollTarget={gridScrollTarget()}
            onScrollTargetHandled={() => setScrollTarget(null)}
            onLandingPathSelect={props.onLandingPathSelect}
            fetchPayments={props.fetchPayments}
            fetchJourney={props.fetchJourney}
            onRevenueSourceSelect={props.onRevenueSourceSelect}
            onSourceFilter={props.onSourceFilter}
            activeSourceFilterKeys={props.activeSourceFilterKeys}
          />
        </div>
      </Show>
    </div>
  );
}
