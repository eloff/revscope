/// <reference types="solid-js" />
import {
  type Accessor,
  createContext,
  createEffect,
  createMemo,
  createSignal,
  onMount,
  type ParentProps,
  type Setter,
  useContext,
} from "solid-js";
import {
  clampRangeToRetention,
  type DateRangeSeconds,
  DEFAULT_DATE_SHORTCUT,
  EMPTY_DATE_RANGE,
  oldestRetainedStart,
  shortcutToRange,
} from "../../lib/dateRange";
import {
  formatPeriodComparisonNote,
  type PeriodComparison,
  periodComparisonFromRange,
} from "../../lib/periodComparison";
import {
  type AttributeFilter,
  EMPTY_SEGMENT_FILTERS,
  type EventFilter,
  type LandingPageFilter,
  type PageFilter,
  type SegmentFilters,
  type SourceFilter,
  withAttributeFilter,
  withCountryFilter,
  withEventFilter,
  withLandingPageFilter,
  withoutAttributeFilter,
  withoutCountryFilter,
  withoutEventFilter,
  withoutLandingPageFilter,
  withoutPageFilter,
  withoutSourceFilter,
  withPageFilter,
  withSourceFilter,
} from "../../lib/segmentFilters";
import { UTC_TIME_ZONE } from "../../lib/timezone";

export type DashboardFilters = {
  range: DateRangeSeconds;
  segments: SegmentFilters;
};

type DashboardStateContextValue = {
  filters: Accessor<DashboardFilters>;
  setFilters: Setter<DashboardFilters>;
  /**
   * The selected project's IANA zone. Every calendar boundary the dashboard
   * computes or labels goes through this, so the ranges it sends and the dates
   * it prints agree with the buckets the server builds.
   */
  timeZone: Accessor<string>;
  /** The selected range, never starting before {@link oldestStart}. */
  range: Accessor<DateRangeSeconds>;
  setRange: (range: DateRangeSeconds) => void;
  /** The first instant the plan's retention shows, or `null` for no limit. */
  oldestStart: Accessor<number | null>;
  /** False during SSR and the first client paint; true after onMount sets the real range. */
  rangeReady: Accessor<boolean>;
  segments: Accessor<SegmentFilters>;
  addCountryFilter: (code: string, name: string) => void;
  removeCountryFilter: (code: string) => void;
  addSourceFilter: (source: SourceFilter) => void;
  removeSourceFilter: (key: string) => void;
  addPageFilter: (page: PageFilter) => void;
  removePageFilter: (key: string) => void;
  addLandingPageFilter: (page: LandingPageFilter) => void;
  removeLandingPageFilter: (key: string) => void;
  addEventFilter: (event: EventFilter) => void;
  removeEventFilter: (key: string) => void;
  addAttributeFilter: (filter: AttributeFilter) => void;
  removeAttributeFilter: (key: string) => void;
  clearSegments: () => void;
  previousRange: Accessor<DateRangeSeconds>;
  dayCount: Accessor<number>;
  comparison: Accessor<PeriodComparison>;
  comparisonNote: Accessor<string>;
};

const DashboardStateContext = createContext<DashboardStateContextValue>();

export type DashboardStateProviderProps = ParentProps<{
  /** The selected project's IANA zone; UTC until a project resolves. */
  timeZone?: Accessor<string>;
  /** A period the viewer already chose, carried over from another project. */
  initialRange?: DateRangeSeconds;
  /** Months of history the project's plan shows; `null` or absent for all of it. */
  retentionMonths?: Accessor<number | null>;
}>;

export function DashboardStateProvider(props: DashboardStateProviderProps) {
  const timeZone = createMemo(() => props.timeZone?.() || UTC_TIME_ZONE);
  // Empty on SSR so prerendered HTML stays clock-independent; filled in onMount.
  const [filters, setFilters] = createSignal<DashboardFilters>({
    range: EMPTY_DATE_RANGE,
    segments: EMPTY_SEGMENT_FILTERS,
  });
  const [rangeReady, setRangeReady] = createSignal(false);
  // Clock-dependent like the range, so it waits for mount too.
  const oldestStart = createMemo(() =>
    rangeReady() ? oldestRetainedStart(timeZone(), props.retentionMonths?.() ?? null) : null,
  );
  // Clamped on read rather than on every write, so a default, a carried-over
  // range, and a shifted one all stay inside retention, and a plan that keeps
  // more shows the range as it was chosen.
  const range = createMemo(() => clampRangeToRetention(timeZone(), filters().range, oldestStart()));
  // The project's zone arrives with the account, which lands after mount, so the
  // default range is realigned when it does. Only while the user hasn't picked a
  // range of their own — moving one they chose out from under them would be worse
  // than leaving it on the boundaries it was chosen with.
  let rangeIsDefault = true;
  const setDefaultRange = () =>
    setFilters((previous) => ({
      ...previous,
      range: shortcutToRange(DEFAULT_DATE_SHORTCUT, timeZone()),
    }));
  const setRange = (range: DateRangeSeconds) => {
    rangeIsDefault = false;
    setFilters((previous) => ({ ...previous, range }));
  };

  onMount(() => {
    if (props.initialRange) {
      setRange(props.initialRange);
    } else {
      setDefaultRange();
    }
    setRangeReady(true);
  });

  createEffect(() => {
    // Read the zone unconditionally so the effect tracks it either way.
    timeZone();
    if (rangeReady() && rangeIsDefault) {
      setDefaultRange();
    }
  });

  const segments = createMemo(() => filters().segments);
  const addCountryFilter = (code: string, name: string) =>
    setFilters((previous) => ({
      ...previous,
      segments: withCountryFilter(previous.segments, code, name),
    }));
  const removeCountryFilter = (code: string) =>
    setFilters((previous) => ({
      ...previous,
      segments: withoutCountryFilter(previous.segments, code),
    }));
  const addSourceFilter = (source: SourceFilter) =>
    setFilters((previous) => ({
      ...previous,
      segments: withSourceFilter(previous.segments, source),
    }));
  const removeSourceFilter = (key: string) =>
    setFilters((previous) => ({
      ...previous,
      segments: withoutSourceFilter(previous.segments, key),
    }));
  const addPageFilter = (page: PageFilter) =>
    setFilters((previous) => ({
      ...previous,
      segments: withPageFilter(previous.segments, page),
    }));
  const removePageFilter = (key: string) =>
    setFilters((previous) => ({
      ...previous,
      segments: withoutPageFilter(previous.segments, key),
    }));
  const addLandingPageFilter = (page: LandingPageFilter) =>
    setFilters((previous) => ({
      ...previous,
      segments: withLandingPageFilter(previous.segments, page),
    }));
  const removeLandingPageFilter = (key: string) =>
    setFilters((previous) => ({
      ...previous,
      segments: withoutLandingPageFilter(previous.segments, key),
    }));
  const addEventFilter = (event: EventFilter) =>
    setFilters((previous) => ({
      ...previous,
      segments: withEventFilter(previous.segments, event),
    }));
  const removeEventFilter = (key: string) =>
    setFilters((previous) => ({
      ...previous,
      segments: withoutEventFilter(previous.segments, key),
    }));
  const addAttributeFilter = (filter: AttributeFilter) =>
    setFilters((previous) => ({
      ...previous,
      segments: withAttributeFilter(previous.segments, filter),
    }));
  const removeAttributeFilter = (key: string) =>
    setFilters((previous) => ({
      ...previous,
      segments: withoutAttributeFilter(previous.segments, key),
    }));
  const clearSegments = () =>
    setFilters((previous) => ({
      ...previous,
      segments: EMPTY_SEGMENT_FILTERS,
    }));

  const comparison = createMemo(() => periodComparisonFromRange(range()));
  const previousRange = createMemo(() => comparison().previous);
  const dayCount = createMemo(() => comparison().dayCount);
  const comparisonNote = createMemo(() => formatPeriodComparisonNote(comparison(), timeZone()));

  const value: DashboardStateContextValue = {
    filters,
    setFilters,
    timeZone,
    range,
    setRange,
    oldestStart,
    rangeReady,
    segments,
    addCountryFilter,
    removeCountryFilter,
    addSourceFilter,
    removeSourceFilter,
    addPageFilter,
    removePageFilter,
    addLandingPageFilter,
    removeLandingPageFilter,
    addEventFilter,
    removeEventFilter,
    addAttributeFilter,
    removeAttributeFilter,
    clearSegments,
    previousRange,
    dayCount,
    comparison,
    comparisonNote,
  };

  return (
    <DashboardStateContext.Provider value={value}>{props.children}</DashboardStateContext.Provider>
  );
}

export function useDashboardState(): DashboardStateContextValue {
  const context = useContext(DashboardStateContext);
  if (!context) {
    throw new Error("useDashboardState must be used within DashboardStateProvider");
  }
  return context;
}

export const DashboardPeriodProvider = DashboardStateProvider;
export const useDashboardPeriod = useDashboardState;
