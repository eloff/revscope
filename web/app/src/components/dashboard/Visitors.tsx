/// <reference types="solid-js" />
import { createEffect, createMemo, createSignal, onCleanup, onMount, Show } from "solid-js";
import {
  backFromJourneyHistory,
  clearJourneyHistoryParam,
  installJourneyHistoryListener,
  pushJourneyHistoryParam,
} from "../../lib/journeyHistory";
import type { ConversionsResponse, PaymentJourneyResponse } from "../../lib/paymentsApi";
import {
  formatCustomersPurchaseNote,
  formatVisitorsPageviewNote,
  formatVisitorsSessionNote,
} from "../../lib/periodComparison";
import type {
  VisitorEntry,
  VisitorJourneyResponse,
  VisitorSortState,
  VisitorsResponse,
} from "../../lib/visitorsApi";
import { VISITORS_PAGE_SIZE } from "../../lib/visitorsApi";
import ActiveFilters, { type ActiveFilterChip } from "./ActiveFilters";
import JourneyDetail from "./ConversionJourney";
import Conversions from "./Conversions";
import { useDashboardState } from "./dashboardPeriodContext";
import { errorMessage } from "./messages.css";
import NoDataView from "./NoDataView";
import { rangeNoteSpaced } from "./rangeNote.css";
import type { RevenueSourceLinkTarget } from "./revenueSourcesLogic";
import SegmentedControl from "./SegmentedControl";
import { segmentedControlRow } from "./SegmentedControl.css";
import SlidePanels from "./SlidePanels";
import VisitorsTable from "./VisitorsTable";

export type VisitorsRangeNoteVariant = "session" | "pageview" | "customers";

const VISITOR_JOURNEY_PARAM = "visitor_timestamp_id";

export type PeopleMode = "visitors" | "customers";

function VisitorsRangeNote(props: { variant: VisitorsRangeNoteVariant }) {
  const { range, timeZone } = useDashboardState();
  const text = createMemo(() => {
    switch (props.variant) {
      case "pageview":
        return formatVisitorsPageviewNote(range(), timeZone());
      case "customers":
        return formatCustomersPurchaseNote(range(), timeZone());
      default:
        return formatVisitorsSessionNote(range(), timeZone());
    }
  });
  return <p class={rangeNoteSpaced}>{text()}</p>;
}

export type VisitorsProps = {
  peopleMode: PeopleMode;
  onPeopleModeChange: (mode: PeopleMode) => void;
  filterChips: ActiveFilterChip[];
  visitorsEmpty: boolean;
  conversionsEmpty: boolean;
  data: VisitorsResponse | undefined;
  loading: boolean;
  error: string;
  sort: VisitorSortState;
  onSortChange: (sort: VisitorSortState) => void;
  onPageChange: (offset: number) => void;
  fetchJourney: (visitorTimestampId: string) => Promise<VisitorJourneyResponse>;
  conversionsData: ConversionsResponse | undefined;
  conversionsLoading: boolean;
  conversionsError: string;
  conversionsOffset: number;
  onConversionsPageChange: (direction: "previous" | "next") => void;
  fetchPaymentJourney: (paymentId: number) => Promise<PaymentJourneyResponse>;
  onRevenueSourceSelect?: (target: RevenueSourceLinkTarget) => void;
  onPathSelect?: (pathId: number) => void;
  onLandingPathSelect?: (pathId: number) => void;
  onCountrySelect?: (countryCode: string) => void;
  emptyLabel?: string;
};

export function VisitorJourneyPanel(props: {
  data: VisitorsResponse | undefined;
  loading: boolean;
  error: string;
  sort: VisitorSortState;
  onSortChange: (sort: VisitorSortState) => void;
  onPageChange: (offset: number) => void;
  fetchJourney: (visitorTimestampId: string) => Promise<VisitorJourneyResponse>;
  onRevenueSourceSelect?: (target: RevenueSourceLinkTarget) => void;
  onPathSelect?: (pathId: number) => void;
  onLandingPathSelect?: (pathId: number) => void;
  onCountrySelect?: (countryCode: string) => void;
  emptyLabel?: string;
  /** Session-start note by default; pageview for page/event drill-ins. */
  rangeNoteVariant?: VisitorsRangeNoteVariant;
  /** Set false when the parent already renders VisitorsRangeNote. */
  showRangeNote?: boolean;
}) {
  const [selectedVisitor, setSelectedVisitor] = createSignal<VisitorEntry | undefined>();
  const [journey, setJourney] = createSignal<VisitorJourneyResponse>();
  const [journeyLoading, setJourneyLoading] = createSignal(false);
  const [journeyError, setJourneyError] = createSignal("");
  let journeyRequestId = 0;

  const resetJourney = () => {
    journeyRequestId += 1;
    setSelectedVisitor(undefined);
    setJourney(undefined);
    setJourneyLoading(false);
    setJourneyError("");
  };

  const closeJourney = () => {
    resetJourney();
    clearJourneyHistoryParam(VISITOR_JOURNEY_PARAM);
  };

  const handleBack = () => {
    if (selectedVisitor()) {
      backFromJourneyHistory(closeJourney);
      return;
    }
    closeJourney();
  };

  const openJourney = async (visitor: VisitorEntry, options?: { fromHistory?: boolean }) => {
    setSelectedVisitor(visitor);
    setJourneyError("");
    if (!options?.fromHistory) {
      pushJourneyHistoryParam(VISITOR_JOURNEY_PARAM, visitor.visitor_timestamp_id);
    }
    const requestId = ++journeyRequestId;
    setJourney(undefined);
    setJourneyLoading(true);
    try {
      const response = await props.fetchJourney(visitor.visitor_timestamp_id);
      if (requestId === journeyRequestId) {
        setJourney(response);
      }
    } catch (loadError) {
      if (requestId === journeyRequestId) {
        setJourneyError(
          loadError instanceof Error ? loadError.message : "Unable to load visitor journey.",
        );
      }
    } finally {
      if (requestId === journeyRequestId) {
        setJourneyLoading(false);
      }
    }
  };

  onMount(() => {
    onCleanup(
      installJourneyHistoryListener(VISITOR_JOURNEY_PARAM, (visitorId) => {
        if (!visitorId) {
          resetJourney();
          return;
        }
        if (selectedVisitor()?.visitor_timestamp_id === visitorId) {
          return;
        }
        const visitor = props.data?.visitors?.find(
          (entry) => entry.visitor_timestamp_id === visitorId,
        );
        if (visitor) {
          void openJourney(visitor, { fromHistory: true });
          return;
        }
        resetJourney();
      }),
    );
  });

  createEffect(() => {
    const visitors = props.data?.visitors;
    const selected = selectedVisitor();
    if (!visitors || !selected) {
      return;
    }
    if (
      !visitors.some((visitor) => visitor.visitor_timestamp_id === selected.visitor_timestamp_id)
    ) {
      closeJourney();
    }
  });

  return (
    <Show when={!props.error} fallback={<p class={errorMessage}>{props.error}</p>}>
      <Show when={props.showRangeNote !== false}>
        <VisitorsRangeNote variant={props.rangeNoteVariant ?? "session"} />
      </Show>
      <SlidePanels
        speed="fast"
        showSecond={selectedVisitor() !== undefined}
        first={
          <VisitorsTable
            visitors={props.data?.visitors}
            loading={props.loading}
            emptyLabel={props.emptyLabel ?? "No visitors for this period"}
            offset={props.data?.offset ?? 0}
            limit={props.data?.limit ?? VISITORS_PAGE_SIZE}
            nextOffset={props.data?.next_offset}
            sort={props.sort}
            onSortChange={props.onSortChange}
            onPageChange={props.onPageChange}
            onVisitorSelect={(visitor) => void openJourney(visitor)}
            onRevenueSourceSelect={props.onRevenueSourceSelect}
            onCountrySelect={props.onCountrySelect}
          />
        }
        second={
          <JourneyDetail
            visitor={selectedVisitor()}
            journey={journey()}
            loading={journeyLoading()}
            error={journeyError()}
            backLabel="Back to visitors"
            titleFallback="Visitor journey"
            onBack={handleBack}
            onRevenueSourceSelect={props.onRevenueSourceSelect}
            onPathSelect={props.onPathSelect}
            onLandingPathSelect={props.onLandingPathSelect}
            onCountrySelect={props.onCountrySelect}
          />
        }
      />
    </Show>
  );
}

const PEOPLE_MODE_OPTIONS = [
  { value: "visitors", label: "Visitors" },
  { value: "customers", label: "New Customers" },
] as const;

export default function Visitors(props: VisitorsProps) {
  const rangeNoteVariant = createMemo<VisitorsRangeNoteVariant>(() => {
    if (props.peopleMode === "customers") {
      return "customers";
    }
    return props.filterChips.length > 0 ? "pageview" : "session";
  });

  return (
    <div>
      <div class={segmentedControlRow}>
        <SegmentedControl
          value={props.peopleMode}
          options={PEOPLE_MODE_OPTIONS}
          onChange={props.onPeopleModeChange}
        />
      </div>

      <Show when={props.peopleMode === "visitors"}>
        <ActiveFilters filters={props.filterChips} />
      </Show>
      <VisitorsRangeNote variant={rangeNoteVariant()} />

      <SlidePanels
        speed="slow"
        showSecond={props.peopleMode === "customers"}
        first={
          <Show when={!props.visitorsEmpty} fallback={<NoDataView />}>
            <VisitorJourneyPanel
              data={props.data}
              loading={props.loading}
              error={props.error}
              sort={props.sort}
              onSortChange={props.onSortChange}
              onPageChange={props.onPageChange}
              fetchJourney={props.fetchJourney}
              onRevenueSourceSelect={props.onRevenueSourceSelect}
              onPathSelect={props.onPathSelect}
              onLandingPathSelect={props.onLandingPathSelect}
              onCountrySelect={props.onCountrySelect}
              emptyLabel={props.emptyLabel}
              showRangeNote={false}
            />
          </Show>
        }
        second={
          <Show when={!props.conversionsEmpty} fallback={<NoDataView />}>
            <Conversions
              data={props.conversionsData}
              loading={props.conversionsLoading}
              error={props.conversionsError}
              offset={props.conversionsOffset}
              onPageChange={props.onConversionsPageChange}
              fetchJourney={props.fetchPaymentJourney}
              onRevenueSourceSelect={props.onRevenueSourceSelect}
              onPathSelect={props.onPathSelect}
              onLandingPathSelect={props.onLandingPathSelect}
              onCountrySelect={props.onCountrySelect}
            />
          </Show>
        }
      />
    </div>
  );
}
