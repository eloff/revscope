import { fireEvent, render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import type { PaymentActivityDay, PaymentActivityResponse } from "../../lib/paymentActivityApi";
import ActivityGraph from "./ActivityGraph";
import { DashboardStateProvider } from "./dashboardPeriodContext";

const DAY = 86_400;
// Sunday 15 March 2026 is the last day in the window; the grid runs to Saturday the 21st.
const END = Date.parse("2026-03-16T00:00:00Z") / 1000;

function day(date: string, overrides: Partial<PaymentActivityDay> = {}): PaymentActivityDay {
  return {
    date,
    single: 0,
    single_revenue: 0,
    new_subscription: 0,
    new_subscription_revenue: 0,
    recurring: 0,
    recurring_revenue: 0,
    ...overrides,
  };
}

const data: PaymentActivityResponse = {
  start: END - 365 * DAY,
  end: END,
  days: [
    day("2026-03-15", {
      single: 2,
      single_revenue: 10_000,
      recurring: 1,
      recurring_revenue: 90_000,
    }),
    day("2026-03-14", { new_subscription: 5, new_subscription_revenue: 5_000 }),
  ],
  monthly_revenue: [],
};

function renderGraph(props: Partial<Parameters<typeof ActivityGraph>[0]> = {}) {
  const result = render(() => (
    <DashboardStateProvider>
      <ActivityGraph data={data} loading={false} error="" {...props} />
    </DashboardStateProvider>
  ));
  const cells = () => [...result.container.querySelectorAll<HTMLElement>('[class*="dayCell"]')];
  return { ...result, cells };
}

describe("ActivityGraph", () => {
  it("lays out 53 weeks ending on the window's last week, future days blank", () => {
    const { cells } = renderGraph();
    expect(cells()).toHaveLength(53 * 7);
    const future = cells().slice(-6);
    expect(future.every((cell) => cell.style.background === "transparent")).toBe(true);
    expect(cells().at(-7)?.style.background).not.toBe("transparent");
  });

  it("summarizes purchases, or revenue when shading by it", async () => {
    renderGraph();
    expect(screen.getByText("8 purchases in the last 12 months")).toBeInTheDocument();
    await userEvent.click(screen.getByText("Revenue"));
    expect(await screen.findByText("$105 in the last 12 months")).toBeInTheDocument();
  });

  it("shades the busiest day darkest", async () => {
    const { cells } = renderGraph();
    const [saturday, sunday] = [cells().at(-8), cells().at(-7)];
    // By purchases the 14th (5) leads the 15th (3); by revenue the 15th wins.
    expect(saturday?.style.background).not.toBe(sunday?.style.background);
    const byPurchases = saturday?.style.background;
    await userEvent.click(screen.getByText("Revenue"));
    // The grid rebuilds when the shading changes, so read the cell again.
    expect(cells().at(-7)?.style.background).toBe(byPurchases);
  });

  it("drops a payment type from the totals when unticked", async () => {
    renderGraph();
    await userEvent.click(screen.getByLabelText("New subscriptions"));
    expect(screen.getByText("3 purchases in the last 12 months")).toBeInTheDocument();
    await userEvent.click(screen.getByLabelText("New revenue"));
    await userEvent.click(screen.getByLabelText("Recurring revenue"));
    expect(screen.getByText("Select a payment type to see activity.")).toBeInTheDocument();
  });

  it("breaks a day down by payment type on hover, but not a future day", async () => {
    const { cells } = renderGraph();
    fireEvent.mouseEnter(cells().at(-7) as HTMLElement, { clientX: 1, clientY: 2 });
    const title = await screen.findByText(/Mar 15th, 2026/);
    const card = title.parentElement as HTMLElement;
    expect(card).toHaveTextContent("$100");
    expect(card).toHaveTextContent("Recurring revenue");

    fireEvent.mouseEnter(cells().at(-1) as HTMLElement);
    expect(screen.queryByText(/Mar 21/)).toBeNull();
  });

  it("shows loading and errors", () => {
    render(() => (
      <DashboardStateProvider>
        <ActivityGraph loading error="Payments failed." />
      </DashboardStateProvider>
    ));
    expect(screen.getByText("Loading purchase activity...")).toBeInTheDocument();
    expect(screen.getByText("Payments failed.")).toBeInTheDocument();
  });
});
