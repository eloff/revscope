import { render, screen } from "@solidjs/testing-library";
import { describe, expect, it } from "vitest";
import UnpaidSubscriptionBanner from "./UnpaidSubscriptionBanner";

const UNPAID = {
  plan: "growth",
  events_monthly_limit: 1_000_000,
  billing_period: "monthly",
  amount_cents: 17_200,
  currency: "usd",
  ended_at: Date.UTC(2026, 10, 1, 12) / 1000,
};

describe("UnpaidSubscriptionBanner", () => {
  it("names the lapsed plan and links to paying what it costs", () => {
    render(() => <UnpaidSubscriptionBanner unpaid={UNPAID} retentionMonths={3} />);
    const status = screen.getByRole("status");
    expect(status.textContent).toContain(
      `Growth (${new Intl.NumberFormat().format(1_000_000)} events / month)`,
    );
    expect(status.textContent).toContain("moved to the Free plan");
    expect(status.textContent).toContain("last 3 months");
    const pay = screen.getByRole("link", { name: /Pay .*172\.00 for a month to restore it/ });
    expect(pay.getAttribute("href")).toBe("/billing");
  });

  it("shows nothing when nothing is owed", () => {
    render(() => <UnpaidSubscriptionBanner unpaid={null} retentionMonths={3} />);
    expect(screen.queryByRole("status")).toBeNull();
  });
});
