import { render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import type { JSX } from "solid-js";
import { describe, expect, it, vi } from "vitest";
import { testLanding, testPayment, testVisitor } from "../../../test/dashboardFixtures";
import type { PaymentJourneyAction } from "../../lib/paymentsApi";
import JourneyDetail from "./ConversionJourney";
import { DashboardStateProvider } from "./dashboardPeriodContext";

const T = Date.parse("2026-03-10T09:00:00Z") / 1000;

function withDashboard(children: () => JSX.Element) {
  return render(() => <DashboardStateProvider>{children()}</DashboardStateProvider>);
}

/** The value cell beside a summary label. */
function detail(label: string): string {
  return screen.getByText(label, { selector: "th, td" }).nextElementSibling?.textContent ?? "";
}

const referrer = { id: 4, referrer: "news.ycombinator.com" };
const payment = testPayment({
  customer_email: "amy@x.io",
  amount: 49_000,
  mrr_amount: 49_000,
  interval_unit: "month",
  paid_at: T + 2 * 86_400,
  provider_customer_id: "cus_9",
  metadata: { plan: { tier: "pro" }, seats: 3 },
  landing: testLanding({
    viewed_at: T,
    country: "DE",
    city: "Berlin",
    referrer,
    referrer_type: undefined,
  } as never),
});

const timeline: PaymentJourneyAction[] = [
  { kind: "source", occurred_at: T, source_group: "referrers" },
  {
    kind: "pageview",
    occurred_at: T + 10,
    subtitle: "/pricing",
    path_id: 7,
    duration_seconds: 65,
    scrolled_percent: 80,
  },
  { kind: "pageview", occurred_at: T + 100, subtitle: "/docs", path_id: 8 },
  {
    kind: "event",
    occurred_at: T + 200,
    name: "revscope-outbound-click",
    metadata: { to: "github.com/acme" },
  },
  { kind: "event", occurred_at: T + 300, name: "signup", goal_id: 2 },
  { kind: "purchase", occurred_at: T + 2 * 86_400, subtitle: "$49" },
  { kind: "pageview", occurred_at: T + 3 * 86_400, subtitle: "/account", path_id: 9 },
];

describe("JourneyDetail", () => {
  it("summarizes the payment and its custom attributes", () => {
    withDashboard(() => (
      <JourneyDetail journey={{ payment, timeline }} loading={false} error="" onBack={vi.fn()} />
    ));
    expect(screen.getByRole("heading", { name: "amy@x.io" })).toBeInTheDocument();
    expect(screen.getByText("news.ycombinator.com", { selector: "p" })).toBeInTheDocument();
    expect(detail("Amount")).toBe("$49");
    expect(detail("Subscription")).toBe("Monthly");
    expect(detail("Time to purchase")).toBe("2 days");
    expect(detail("City")).toBe("Berlin");
    expect(detail("Order ID")).toBe("—");
    expect(detail("plan.tier")).toBe("pro");
    expect(detail("seats")).toBe("3");
  });

  it("tells the story up to the purchase, hiding what came after", async () => {
    withDashboard(() => (
      <JourneyDetail journey={{ payment, timeline }} loading={false} error="" onBack={vi.fn()} />
    ));
    expect(screen.getByText("Discovered via")).toBeInTheDocument();
    expect(screen.getByText("Duration 1m 5s · Scrolled 80%")).toBeInTheDocument();
    expect(screen.getByText("Goal: signup")).toBeInTheDocument();
    expect(screen.getByText("Event: revscope-outbound-click")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "github.com/acme" })).toHaveAttribute(
      "href",
      "https://github.com/acme",
    );
    expect(screen.queryByText("/account")).toBeNull();

    await userEvent.click(screen.getByRole("button", { name: "Show 1 event after purchase" }));
    expect(screen.getByText("/account")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Hide events after purchase" }));
    expect(screen.queryByText("/account")).toBeNull();
  });

  it("links the landing page, later pages and the source to their tabs", async () => {
    const onLandingPathSelect = vi.fn();
    const onPathSelect = vi.fn();
    const onRevenueSourceSelect = vi.fn();
    withDashboard(() => (
      <JourneyDetail
        journey={{ payment, timeline }}
        loading={false}
        error=""
        onBack={vi.fn()}
        onLandingPathSelect={onLandingPathSelect}
        onPathSelect={onPathSelect}
        onRevenueSourceSelect={onRevenueSourceSelect}
      />
    ));
    await userEvent.click(
      screen.getByRole("button", { name: "View details for /pricing in Landing Pages" }),
    );
    await userEvent.click(screen.getByRole("button", { name: "View details for /docs in Pages" }));
    await userEvent.click(
      screen.getByRole("button", { name: "View details for news.ycombinator.com in Sources" }),
    );
    expect(onLandingPathSelect).toHaveBeenCalledWith(7);
    expect(onPathSelect).toHaveBeenCalledWith(8);
    expect(onRevenueSourceSelect).toHaveBeenCalledWith({ kind: "referrer", id: 4 });
  });

  it("goes back from either header button", async () => {
    const onBack = vi.fn();
    withDashboard(() => (
      <JourneyDetail
        payment={payment}
        loading
        error=""
        onBack={onBack}
        backLabel="Back to sources"
      />
    ));
    expect(screen.getByText("Loading journey…")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Back to sources" }));
    await userEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(onBack).toHaveBeenCalledTimes(2);
  });

  it("describes a visitor without a purchase", () => {
    const visitor = testVisitor({
      visitor_timestamp_id: "abc",
      country: "FR",
      last_view_at: 1_700_000_000 + 3 * 86_400,
    });
    withDashboard(() => (
      <JourneyDetail
        journey={{ visitor, timeline: [] }}
        loading={false}
        error=""
        onBack={vi.fn()}
      />
    ));
    expect(screen.getByRole("heading", { name: "Visitor abc" })).toBeInTheDocument();
    expect(detail("Session age")).toBe("3 days");
    expect(screen.queryByText("Amount")).toBeNull();
  });

  it("waits for a subject and shows journey errors", () => {
    withDashboard(() => (
      <>
        <JourneyDetail loading={false} error="" onBack={vi.fn()} titleFallback="Customer" />
        <JourneyDetail
          payment={payment}
          loading={false}
          error="Journey unavailable."
          onBack={vi.fn()}
        />
      </>
    ));
    expect(screen.getByRole("heading", { name: "Customer" })).toBeInTheDocument();
    expect(screen.getByText("Loading…")).toBeInTheDocument();
    expect(screen.getByText("Journey unavailable.")).toBeInTheDocument();
  });
});
