import { render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { createSignal, type JSX } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import { testPayment, testVisitor } from "../../../test/dashboardFixtures";
import type { PaymentEntry, PaymentJourneyResponse } from "../../lib/paymentsApi";
import { DEFAULT_VISITOR_SORT, type VisitorJourneyResponse } from "../../lib/visitorsApi";
import Conversions from "./Conversions";
import { DashboardStateProvider } from "./dashboardPeriodContext";
import PaymentJourneyPanel from "./PaymentJourneyPanel";
import Visitors from "./Visitors";

afterEach(() => window.history.replaceState(null, "", "/"));

function withDashboard(children: () => JSX.Element) {
  return render(() => <DashboardStateProvider>{children()}</DashboardStateProvider>);
}

const amy = testPayment({ id: 11, customer_email: "amy@x.io" });
const bo = testPayment({ id: 12, customer_email: "bo@x.io" });

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

describe("PaymentJourneyPanel", () => {
  it("opens a payment's journey and records it in history", async () => {
    const fetchJourney = vi.fn(
      async (id: number): Promise<PaymentJourneyResponse> => ({
        payment: id === 11 ? amy : bo,
        timeline: [{ kind: "purchase", occurred_at: amy.paid_at, subtitle: "Bought Pro" }],
      }),
    );
    withDashboard(() => (
      <PaymentJourneyPanel
        payments={[amy, bo]}
        loading={false}
        emptyLabel="None"
        fetchJourney={fetchJourney}
      />
    ));
    await userEvent.click(screen.getByRole("button", { name: "View journey for amy@x.io" }));
    expect(await screen.findByText("Bought Pro")).toBeInTheDocument();
    expect(fetchJourney).toHaveBeenCalledWith(11);
    expect(new URLSearchParams(window.location.search).get("payment_id")).toBe("11");
  });

  it("closes the journey when the browser goes back", async () => {
    const fetchJourney = vi.fn(
      async (): Promise<PaymentJourneyResponse> => ({ payment: amy, timeline: [] }),
    );
    withDashboard(() => (
      <PaymentJourneyPanel
        payments={[amy]}
        loading={false}
        emptyLabel="None"
        fetchJourney={fetchJourney}
      />
    ));
    await userEvent.click(screen.getByRole("button", { name: "View journey for amy@x.io" }));
    await screen.findByRole("heading", { name: "amy@x.io" });
    await userEvent.click(screen.getByRole("button", { name: "Back to conversions" }));
    await vi.waitFor(() => expect(window.location.search).toBe(""));
    await vi.waitFor(() => expect(screen.queryByRole("heading", { name: "amy@x.io" })).toBeNull());
  });

  it("reopens a journey when the browser goes forward to it", async () => {
    const fetchJourney = vi.fn(
      async (): Promise<PaymentJourneyResponse> => ({ payment: bo, timeline: [] }),
    );
    withDashboard(() => (
      <PaymentJourneyPanel
        payments={[amy, bo]}
        loading={false}
        emptyLabel="None"
        fetchJourney={fetchJourney}
      />
    ));
    window.history.pushState(null, "", "/?payment_id=12");
    window.dispatchEvent(new PopStateEvent("popstate"));
    expect(await screen.findByRole("heading", { name: "bo@x.io" })).toBeInTheDocument();
  });

  it("keeps only the latest journey when clicks overlap", async () => {
    const slow = deferred<PaymentJourneyResponse>();
    const fetchJourney = vi.fn((id: number) =>
      id === 11
        ? slow.promise
        : Promise.resolve({
            payment: bo,
            timeline: [{ kind: "purchase" as const, occurred_at: 1, subtitle: "bo bought" }],
          }),
    );
    withDashboard(() => (
      <PaymentJourneyPanel
        payments={[amy, bo]}
        loading={false}
        emptyLabel="None"
        fetchJourney={fetchJourney}
      />
    ));
    await userEvent.click(screen.getByRole("button", { name: "View journey for amy@x.io" }));
    await userEvent.click(screen.getByRole("button", { name: "View journey for bo@x.io" }));
    await screen.findByText("bo bought");
    slow.resolve({
      payment: amy,
      timeline: [{ kind: "purchase", occurred_at: 1, subtitle: "amy bought" }],
    });
    await Promise.resolve();
    expect(screen.queryByText("amy bought")).toBeNull();
  });

  it("closes the journey when its payment leaves the list", async () => {
    const [payments, setPayments] = createSignal<PaymentEntry[]>([amy]);
    withDashboard(() => (
      <PaymentJourneyPanel
        payments={payments()}
        loading={false}
        emptyLabel="None"
        fetchJourney={async () => ({ payment: amy, timeline: [] })}
      />
    ));
    await userEvent.click(screen.getByRole("button", { name: "View journey for amy@x.io" }));
    await screen.findByRole("heading", { name: "amy@x.io" });
    setPayments([bo]);
    expect(screen.queryByRole("heading", { name: "amy@x.io" })).toBeNull();
    expect(window.location.search).toBe("");
  });

  it("shows a journey that fails to load", async () => {
    withDashboard(() => (
      <PaymentJourneyPanel
        payments={[amy]}
        loading={false}
        emptyLabel="None"
        fetchJourney={async () => {
          throw new Error("Journey timed out.");
        }}
      />
    ));
    await userEvent.click(screen.getByRole("button", { name: "View journey for amy@x.io" }));
    expect(await screen.findByText("Journey timed out.")).toBeInTheDocument();
  });
});

describe("Conversions", () => {
  it("shows the tab error instead of the table", () => {
    withDashboard(() => (
      <Conversions
        data={undefined}
        loading={false}
        error="Conversions failed."
        offset={0}
        onPageChange={vi.fn()}
        fetchJourney={vi.fn()}
      />
    ));
    expect(screen.getByText("Conversions failed.")).toBeInTheDocument();
    expect(screen.queryByRole("table")).toBeNull();
  });

  it("enables Next only when there is a next cursor", () => {
    withDashboard(() => (
      <Conversions
        data={{ conversions: [amy], next_cursor: "c2" }}
        loading={false}
        error=""
        offset={0}
        onPageChange={vi.fn()}
        fetchJourney={vi.fn()}
      />
    ));
    expect(screen.getByRole("button", { name: "Next" })).toBeEnabled();
  });
});

describe("Visitors", () => {
  const visitor = testVisitor({ visitor_timestamp_id: "v1" });

  function renderVisitors(overrides: Partial<Parameters<typeof Visitors>[0]> = {}) {
    const props: Parameters<typeof Visitors>[0] = {
      peopleMode: "visitors",
      onPeopleModeChange: vi.fn(),
      filterChips: [],
      visitorsEmpty: false,
      conversionsEmpty: false,
      data: { visitors: [visitor], offset: 0, limit: 100 },
      loading: false,
      error: "",
      sort: DEFAULT_VISITOR_SORT,
      onSortChange: vi.fn(),
      onPageChange: vi.fn(),
      fetchJourney: async (): Promise<VisitorJourneyResponse> => ({ visitor, timeline: [] }),
      conversionsData: { conversions: [amy] },
      conversionsLoading: false,
      conversionsError: "",
      conversionsOffset: 0,
      onConversionsPageChange: vi.fn(),
      fetchPaymentJourney: vi.fn(),
      ...overrides,
    };
    withDashboard(() => <Visitors {...props} />);
    return props;
  }

  it("notes the session window, or the pageview window when filtered", () => {
    renderVisitors({ filterChips: [{ label: "Page: /pricing", onRemove: vi.fn() }] });
    expect(
      screen.getByText(/Displaying visitors who viewed this page between/),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Remove filter: Page: /pricing" }),
    ).toBeInTheDocument();
  });

  it("switches to new customers", async () => {
    const props = renderVisitors();
    expect(
      screen.getByText(/Displaying visitors whose sessions began between/),
    ).toBeInTheDocument();
    await userEvent.click(screen.getByText("New Customers"));
    expect(props.onPeopleModeChange).toHaveBeenCalledWith("customers");
  });

  it("notes purchase time for customers and hides visitor filters", () => {
    renderVisitors({
      peopleMode: "customers",
      filterChips: [{ label: "Page: /pricing", onRemove: vi.fn() }],
    });
    expect(screen.getByText(/Displaying customers who purchased between/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Remove filter/ })).toBeNull();
  });

  it("opens a visitor's journey", async () => {
    renderVisitors();
    await userEvent.click(screen.getByRole("button", { name: "View journey for visitor v1" }));
    expect(await screen.findByRole("heading", { name: "Visitor v1" })).toBeInTheDocument();
    expect(new URLSearchParams(window.location.search).get("visitor_timestamp_id")).toBe("v1");
  });

  it("shows the setup prompt for an empty tab", () => {
    renderVisitors({ visitorsEmpty: true });
    expect(screen.getAllByText("No data for the selected date range").length).toBeGreaterThan(0);
  });
});
