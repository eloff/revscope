import { Tabs } from "@ark-ui/solid/tabs";
import { Show } from "solid-js";
import type { PageRow } from "../../lib/pagesApi";
import { CONVERSIONS_PAGE_SIZE, type PaymentJourneyResponse } from "../../lib/paymentsApi";
import { DEFAULT_VISITOR_SORT, type VisitorJourneyResponse } from "../../lib/visitorsApi";
import { gridExpandedCell, gridExpandedRow, gridSubtableShell } from "./grid.css";
import { message } from "./messages.css";
import {
  gridDrilldownTabContent,
  gridDrilldownTabList,
  gridDrilldownTabsRoot,
  gridDrilldownTabTrigger,
} from "./PageDrilldownRow.css";
import PageTransitionsTreemap from "./PageTransitionsTreemap";
import PaymentJourneyPanel from "./PaymentJourneyPanel";
import type { PageDrilldown } from "./pageDrilldownState";
import type { PagesVariant } from "./pageHoverDetails";
import type { RevenueSourceLinkTarget } from "./revenueSourcesLogic";
import { VisitorJourneyPanel } from "./Visitors";

interface PageDrilldownRowProps {
  row: PageRow;
  colSpan: number;
  variant: PagesVariant;
  drilldown: PageDrilldown;
  onTransitionPathSelect: (pathId: number) => void;
  fetchJourney: (paymentId: number) => Promise<PaymentJourneyResponse>;
  fetchVisitorJourney: (visitorTimestampId: string) => Promise<VisitorJourneyResponse>;
  onRevenueSourceSelect?: (target: RevenueSourceLinkTarget) => void;
  onVisitorPathSelect?: (pathId: number) => void;
  onVisitorLandingPathSelect?: (pathId: number) => void;
  onCountrySelect?: (countryCode: string) => void;
}

/** The expanded row under a page: its transitions, conversions, and visitors tabs. */
export default function PageDrilldownRow(props: PageDrilldownRowProps) {
  const expanded = () => props.drilldown.expanded();
  const noun = () => (props.variant === "landingPages" ? "landing page" : "page");
  const transitionTreemapLabels = () => ({
    loading: `Loading ${noun()} transitions…`,
    empty: `No next-page transitions for this ${noun()}.`,
    topGroup: "Next pages",
  });

  return (
    <tr class={gridExpandedRow}>
      <td class={gridExpandedCell} colSpan={props.colSpan}>
        <div class={gridSubtableShell}>
          <Tabs.Root
            value={expanded()?.activeTab ?? "transitions"}
            onValueChange={(details) => {
              const tab = details.value;
              if (tab === "transitions" || tab === "conversions" || tab === "visitors") {
                props.drilldown.activateTab(props.row, tab);
              }
            }}
            class={gridDrilldownTabsRoot}
            lazyMount
          >
            <Tabs.List class={gridDrilldownTabList}>
              <Tabs.Trigger value="transitions" class={gridDrilldownTabTrigger}>
                Transitions
              </Tabs.Trigger>
              <Tabs.Trigger value="conversions" class={gridDrilldownTabTrigger}>
                Conversions
              </Tabs.Trigger>
              <Tabs.Trigger value="visitors" class={gridDrilldownTabTrigger}>
                Visitors
              </Tabs.Trigger>
            </Tabs.List>
            <Tabs.Content value="transitions" class={gridDrilldownTabContent}>
              <Show
                when={!expanded()?.transitionsError}
                fallback={<p class={message}>{expanded()?.transitionsError}</p>}
              >
                <Show when={(props.row.imported_visitors ?? 0) > 0}>
                  <p class={message} role="status">
                    Imported Google Analytics data isn't included in transitions.
                  </p>
                </Show>
                <PageTransitionsTreemap
                  data={expanded()?.transitions}
                  loading={expanded()?.transitionsLoading ?? false}
                  labels={transitionTreemapLabels()}
                  onPathSelect={props.onTransitionPathSelect}
                />
              </Show>
            </Tabs.Content>
            <Tabs.Content value="conversions" class={gridDrilldownTabContent}>
              <Show
                when={!expanded()?.paymentsError}
                fallback={<p class={message}>{expanded()?.paymentsError}</p>}
              >
                <PaymentJourneyPanel
                  payments={expanded()?.payments}
                  loading={expanded()?.paymentsLoading ?? false}
                  loadingLabel="Loading conversions…"
                  emptyLabel={`No conversions for this ${noun()}`}
                  offset={(expanded()?.paymentsCursorHistory.length ?? 0) * CONVERSIONS_PAGE_SIZE}
                  hasNextPage={expanded()?.paymentsNextCursor !== undefined}
                  onPageChange={(direction) =>
                    props.drilldown.changePaymentsPage(props.row, direction)
                  }
                  fetchJourney={props.fetchJourney}
                  onRevenueSourceSelect={props.onRevenueSourceSelect}
                />
              </Show>
            </Tabs.Content>
            <Tabs.Content value="visitors" class={gridDrilldownTabContent}>
              <VisitorJourneyPanel
                data={expanded()?.visitors}
                loading={expanded()?.visitorsLoading ?? false}
                error={expanded()?.visitorsError ?? ""}
                sort={expanded()?.visitorsSort ?? DEFAULT_VISITOR_SORT}
                onSortChange={(sort) => props.drilldown.changeVisitorsSort(props.row, sort)}
                onPageChange={(offset) => props.drilldown.changeVisitorsPage(props.row, offset)}
                fetchJourney={props.fetchVisitorJourney}
                onRevenueSourceSelect={props.onRevenueSourceSelect}
                onPathSelect={props.onVisitorPathSelect}
                onLandingPathSelect={props.onVisitorLandingPathSelect}
                onCountrySelect={props.onCountrySelect}
                emptyLabel={`No visitors for this ${noun()}`}
                rangeNoteVariant="pageview"
              />
            </Tabs.Content>
          </Tabs.Root>
        </div>
      </td>
    </tr>
  );
}
