/// <reference types="solid-js" />
import { createMemo, Show } from "solid-js";
import type {
  PagesResponse,
  PageTransitionsResponse,
  PageTransitionsVariant,
} from "../../lib/pagesApi";
import type {
  ConversionsResponse,
  PaymentJourneyResponse,
  PaymentsFilter,
} from "../../lib/paymentsApi";
import { formatLandingPagesSessionNote, formatPagesViewedNote } from "../../lib/periodComparison";
import type {
  VisitorJourneyResponse,
  VisitorSessionFilter,
  VisitorSortState,
  VisitorsResponse,
} from "../../lib/visitorsApi";
import { useDashboardState } from "./dashboardPeriodContext";
import NoDataView from "./NoDataView";
import PagesGrid from "./PagesGrid";
import { rangeNoteSpaced } from "./rangeNote.css";
import type { RevenueSourceLinkTarget } from "./revenueSourcesLogic";
import SegmentedControl from "./SegmentedControl";
import { segmentedControlRow } from "./SegmentedControl.css";
import SlidePanels from "./SlidePanels";

export type PagesMode = "pages" | "landing";

export type PagesProps = {
  pagesMode: PagesMode;
  onPagesModeChange: (mode: PagesMode) => void;
  pagesEmpty: boolean;
  landingPagesEmpty: boolean;
  data: PagesResponse | undefined;
  treemapData: PagesResponse | undefined;
  loading: boolean;
  treemapLoading: boolean;
  error: string;
  scrollTarget?: { pathId: number; id: number } | null;
  onPathSelect?: (pathId: number | null) => void;
  landingData: PagesResponse | undefined;
  landingTreemapData: PagesResponse | undefined;
  landingLoading: boolean;
  landingTreemapLoading: boolean;
  landingError: string;
  landingScrollTarget?: { pathId: number; id: number } | null;
  onLandingPathSelect?: (pathId: number | null) => void;
  onTransitionPathSelect?: (pathId: number) => void;
  onPagesPageChange: (offset: number) => void;
  onLandingPagesPageChange: (offset: number) => void;
  onPagesSortChange: () => void;
  onLandingPagesSortChange: () => void;
  fetchTransitions: (
    pathId: number,
    variant: PageTransitionsVariant,
  ) => Promise<PageTransitionsResponse>;
  fetchPayments: (filters: PaymentsFilter, cursor?: string) => Promise<ConversionsResponse>;
  fetchVisitors: (
    filter: VisitorSessionFilter,
    offset: number,
    sort: VisitorSortState,
  ) => Promise<VisitorsResponse>;
  fetchJourney: (paymentId: number) => Promise<PaymentJourneyResponse>;
  fetchVisitorJourney: (visitorTimestampId: string) => Promise<VisitorJourneyResponse>;
  onRevenueSourceSelect?: (target: RevenueSourceLinkTarget) => void;
  onVisitorPathSelect?: (pathId: number) => void;
  onVisitorLandingPathSelect?: (pathId: number) => void;
  onCountrySelect?: (countryCode: string) => void;
  /** Adds this row's page to the global page segment filter when provided. */
  onPageFilter?: (pathId: number, path: string) => void;
  /** Keys ({@link String} of `paths.id`) of pages currently filtered on. */
  activePageFilterKeys?: Set<string>;
  onLandingPageFilter?: (pathId: number, path: string) => void;
  activeLandingPageFilterKeys?: Set<string>;
};

const PAGES_MODE_OPTIONS = [
  { value: "pages", label: "Pages" },
  { value: "landing", label: "Landing Pages" },
] as const;

export default function Pages(props: PagesProps) {
  const { range, timeZone } = useDashboardState();
  const rangeNote = createMemo(() =>
    props.pagesMode === "landing"
      ? formatLandingPagesSessionNote(range(), timeZone())
      : formatPagesViewedNote(range(), timeZone()),
  );

  return (
    <div>
      <div class={segmentedControlRow}>
        <SegmentedControl
          value={props.pagesMode}
          options={PAGES_MODE_OPTIONS}
          onChange={props.onPagesModeChange}
        />
      </div>

      <p class={rangeNoteSpaced}>{rangeNote()}</p>

      <SlidePanels
        speed="slow"
        showSecond={props.pagesMode === "landing"}
        first={
          <Show when={!props.pagesEmpty} fallback={<NoDataView />}>
            <PagesGrid
              data={props.data}
              treemapData={props.treemapData}
              loading={props.loading}
              treemapLoading={props.treemapLoading}
              error={props.error}
              scrollTarget={props.scrollTarget}
              onPathSelect={props.onPathSelect}
              onTransitionPathSelect={props.onTransitionPathSelect}
              fetchTransitions={props.fetchTransitions}
              fetchPayments={props.fetchPayments}
              fetchVisitors={props.fetchVisitors}
              fetchJourney={props.fetchJourney}
              fetchVisitorJourney={props.fetchVisitorJourney}
              onRevenueSourceSelect={props.onRevenueSourceSelect}
              onVisitorPathSelect={props.onVisitorPathSelect}
              onVisitorLandingPathSelect={props.onVisitorLandingPathSelect}
              onCountrySelect={props.onCountrySelect}
              onPageFilter={props.onPageFilter}
              activePageFilterKeys={props.activePageFilterKeys}
              onPageChange={props.onPagesPageChange}
              onSortChange={props.onPagesSortChange}
            />
          </Show>
        }
        second={
          <Show when={!props.landingPagesEmpty} fallback={<NoDataView />}>
            <PagesGrid
              data={props.landingData}
              treemapData={props.landingTreemapData}
              loading={props.landingLoading}
              treemapLoading={props.landingTreemapLoading}
              error={props.landingError}
              variant="landingPages"
              scrollTarget={props.landingScrollTarget}
              onPathSelect={props.onLandingPathSelect}
              onTransitionPathSelect={props.onTransitionPathSelect}
              fetchTransitions={props.fetchTransitions}
              fetchPayments={props.fetchPayments}
              fetchVisitors={props.fetchVisitors}
              fetchJourney={props.fetchJourney}
              fetchVisitorJourney={props.fetchVisitorJourney}
              onRevenueSourceSelect={props.onRevenueSourceSelect}
              onVisitorPathSelect={props.onVisitorPathSelect}
              onVisitorLandingPathSelect={props.onVisitorLandingPathSelect}
              onCountrySelect={props.onCountrySelect}
              onPageFilter={props.onLandingPageFilter}
              activePageFilterKeys={props.activeLandingPageFilterKeys}
              onPageChange={props.onLandingPagesPageChange}
              onSortChange={props.onLandingPagesSortChange}
            />
          </Show>
        }
      />
    </div>
  );
}
