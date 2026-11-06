import { createSignal } from "solid-js";
import type { PageRow, PageTransitionsResponse, PageTransitionsVariant } from "../../lib/pagesApi";
import type { ConversionsResponse, PaymentEntry, PaymentsFilter } from "../../lib/paymentsApi";
import {
  DEFAULT_VISITOR_SORT,
  type VisitorSessionFilter,
  type VisitorSortState,
  type VisitorsResponse,
} from "../../lib/visitorsApi";
import { pageRowKey } from "./PagesTreemap";
import type { PagesVariant } from "./pageHoverDetails";
import { pageRowHasConversions } from "./pagesLogic";

export type DrilldownTab = "transitions" | "conversions" | "visitors";

export type ExpandedDrilldown = {
  key: string;
  activeTab: DrilldownTab;
  transitions: PageTransitionsResponse | undefined;
  transitionsLoading: boolean;
  transitionsError: string;
  payments: PaymentEntry[] | undefined;
  paymentsNextCursor?: string;
  paymentsCursor?: string;
  paymentsCursorHistory: (string | undefined)[];
  paymentsLoading: boolean;
  paymentsError: string;
  visitors: VisitorsResponse | undefined;
  visitorsLoading: boolean;
  visitorsError: string;
  visitorsOffset: number;
  visitorsSort: VisitorSortState;
};

export type PageDrilldownFetchers = {
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
};

export type PageDrilldown = ReturnType<typeof createPageDrilldown>;

/**
 * State for the one expanded page row and its transitions, conversions, and
 * visitors tabs. Each tab's newest request wins; collapsing drops them all.
 */
export function createPageDrilldown(sources: PageDrilldownFetchers, variant: () => PagesVariant) {
  const [expanded, setExpanded] = createSignal<ExpandedDrilldown | null>(null);
  let transitionsRequestId = 0;
  let paymentsRequestId = 0;
  let visitorsRequestId = 0;

  const paymentFiltersForRow = (row: PageRow): PaymentsFilter =>
    variant() === "landingPages" ? { landingPathId: row.id } : { pathId: row.id };

  const visitorFilterForRow = (row: PageRow): VisitorSessionFilter =>
    variant() === "landingPages"
      ? { kind: "landing_page", pathId: row.id }
      : { kind: "pageview", pathId: row.id };

  const updateExpanded = (
    key: string,
    update: (current: ExpandedDrilldown) => ExpandedDrilldown,
  ) => {
    setExpanded((current) => {
      if (!current || current.key !== key) {
        return current;
      }
      return update(current);
    });
  };

  const emptyExpanded = (key: string, activeTab: DrilldownTab): ExpandedDrilldown => ({
    key,
    activeTab,
    transitions: undefined,
    transitionsLoading: false,
    transitionsError: "",
    payments: undefined,
    paymentsNextCursor: undefined,
    paymentsCursor: undefined,
    paymentsCursorHistory: [],
    paymentsLoading: false,
    paymentsError: "",
    visitors: undefined,
    visitorsLoading: false,
    visitorsError: "",
    visitorsOffset: 0,
    visitorsSort: DEFAULT_VISITOR_SORT,
  });

  const loadTransitionsForRow = async (row: PageRow) => {
    const key = pageRowKey(row);
    const requestId = ++transitionsRequestId;
    setExpanded((current) => ({
      ...(current?.key === key ? current : emptyExpanded(key, "transitions")),
      activeTab: "transitions",
      transitionsLoading: true,
      transitionsError: "",
    }));
    try {
      const transitions = await sources.fetchTransitions(row.id, variant());
      if (requestId === transitionsRequestId) {
        updateExpanded(key, (current) => ({
          ...current,
          transitions,
          transitionsLoading: false,
          transitionsError: "",
        }));
      }
    } catch (loadError) {
      if (requestId === transitionsRequestId) {
        updateExpanded(key, (current) => ({
          ...current,
          transitions: undefined,
          transitionsLoading: false,
          transitionsError:
            loadError instanceof Error ? loadError.message : "Unable to load page transitions.",
        }));
      }
    }
  };

  const loadPaymentsForRow = async (
    row: PageRow,
    cursor?: string,
    cursorHistory: (string | undefined)[] = [],
  ) => {
    const key = pageRowKey(row);
    if (!pageRowHasConversions(row)) {
      setExpanded((current) => ({
        ...(current?.key === key ? current : emptyExpanded(key, "conversions")),
        activeTab: "conversions",
        payments: [],
        paymentsNextCursor: undefined,
        paymentsCursor: cursor,
        paymentsCursorHistory: cursorHistory,
        paymentsLoading: false,
        paymentsError: "",
      }));
      return;
    }
    const requestId = ++paymentsRequestId;
    const filters = paymentFiltersForRow(row);
    setExpanded((current) => ({
      ...(current?.key === key ? current : emptyExpanded(key, "conversions")),
      activeTab: "conversions",
      payments: undefined,
      paymentsNextCursor: undefined,
      paymentsCursor: cursor,
      paymentsCursorHistory: cursorHistory,
      paymentsLoading: true,
      paymentsError: "",
    }));
    try {
      const page = await sources.fetchPayments(filters, cursor);
      if (requestId === paymentsRequestId) {
        updateExpanded(key, (current) => ({
          ...current,
          payments: page.conversions,
          paymentsNextCursor: page.next_cursor,
          paymentsCursor: cursor,
          paymentsCursorHistory: cursorHistory,
          paymentsLoading: false,
          paymentsError: "",
        }));
      }
    } catch (loadError) {
      if (requestId === paymentsRequestId) {
        updateExpanded(key, (current) => ({
          ...current,
          payments: undefined,
          paymentsNextCursor: undefined,
          paymentsCursor: cursor,
          paymentsCursorHistory: cursorHistory,
          paymentsLoading: false,
          paymentsError:
            loadError instanceof Error ? loadError.message : "Unable to load payments.",
        }));
      }
    }
  };

  const handlePaymentsPageChange = (row: PageRow, direction: "previous" | "next") => {
    const current = expanded();
    if (!current || current.key !== pageRowKey(row)) {
      return;
    }
    if (direction === "previous") {
      if (current.paymentsCursorHistory.length === 0) {
        return;
      }
      const previous = current.paymentsCursorHistory[current.paymentsCursorHistory.length - 1];
      void loadPaymentsForRow(row, previous, current.paymentsCursorHistory.slice(0, -1));
      return;
    }
    if (!current.paymentsNextCursor) {
      return;
    }
    void loadPaymentsForRow(row, current.paymentsNextCursor, [
      ...current.paymentsCursorHistory,
      current.paymentsCursor,
    ]);
  };

  const loadVisitorsForRow = async (
    row: PageRow,
    offset = 0,
    sort: VisitorSortState = DEFAULT_VISITOR_SORT,
  ) => {
    const key = pageRowKey(row);
    const requestId = ++visitorsRequestId;
    setExpanded((current) => ({
      ...(current?.key === key ? current : emptyExpanded(key, "visitors")),
      activeTab: "visitors",
      visitorsOffset: offset,
      visitorsSort: sort,
      visitorsLoading: true,
      visitorsError: "",
    }));
    try {
      const visitors = await sources.fetchVisitors(visitorFilterForRow(row), offset, sort);
      if (requestId === visitorsRequestId) {
        updateExpanded(key, (current) => ({
          ...current,
          visitors,
          visitorsLoading: false,
          visitorsError: "",
        }));
      }
    } catch (loadError) {
      if (requestId === visitorsRequestId) {
        updateExpanded(key, (current) => ({
          ...current,
          visitors: undefined,
          visitorsLoading: false,
          visitorsError:
            loadError instanceof Error ? loadError.message : "Unable to load visitors.",
        }));
      }
    }
  };

  const activateDrilldownTab = (row: PageRow, tab: DrilldownTab) => {
    const key = pageRowKey(row);
    setExpanded((current) => ({
      ...(current?.key === key ? current : emptyExpanded(key, tab)),
      activeTab: tab,
    }));
    const current = expanded();
    if (tab === "transitions") {
      if (current?.key !== key || !current.transitions) {
        void loadTransitionsForRow(row);
      }
      return;
    }
    if (tab === "visitors") {
      if (current?.key !== key || !current.visitors) {
        void loadVisitorsForRow(row);
      }
      return;
    }
    if (current?.key !== key || !current.payments) {
      void loadPaymentsForRow(row);
    }
  };

  const handleVisitorsSortChange = (row: PageRow, sort: VisitorSortState) => {
    void loadVisitorsForRow(row, 0, sort);
  };

  const handleVisitorsPageChange = (row: PageRow, offset: number) => {
    const sort = expanded()?.visitorsSort ?? DEFAULT_VISITOR_SORT;
    void loadVisitorsForRow(row, offset, sort);
  };

  const collapse = () => {
    transitionsRequestId += 1;
    paymentsRequestId += 1;
    visitorsRequestId += 1;
    setExpanded(null);
  };

  return {
    expanded,
    /** Expands the row (if needed) and (re)loads its transitions tab. */
    showTransitions: loadTransitionsForRow,
    activateTab: activateDrilldownTab,
    changePaymentsPage: handlePaymentsPageChange,
    changeVisitorsSort: handleVisitorsSortChange,
    changeVisitorsPage: handleVisitorsPageChange,
    collapse,
  };
}
