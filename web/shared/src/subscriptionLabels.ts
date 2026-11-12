/** How the billing screen and the dashboard name RevScope's own plans and prices. */

import type { UnpaidSubscription } from "./authState";

const PLAN_NAMES: Record<string, string> = {
  free: "Free",
  starter: "Starter",
  growth: "Growth",
  enterprise: "Enterprise",
  // The founder sale is the Growth plan at the Starter price for a year.
  founder: "Growth (founder)",
  founder_friends: "Founder & Friends",
};

export function planLabel(planType: string): string {
  return PLAN_NAMES[planType] ?? planType;
}

export function billingPeriodLabel(period: string): string {
  if (period === "monthly") {
    return "Monthly";
  }
  return period === "yearly" ? "Yearly" : period;
}

export function eventsLimitLabel(limit: number | null): string {
  if (limit === null) {
    return "Unlimited";
  }
  return `${new Intl.NumberFormat().format(limit)} / month`;
}

export function formatMoney(cents: number, currency: string): string {
  return new Intl.NumberFormat(undefined, {
    style: "currency",
    currency: currency.toUpperCase(),
  }).format(cents / 100);
}

/** Formats a unix-seconds timestamp from the API in the reader's timezone. */
export function formatDate(epochSeconds: number): string {
  return new Intl.DateTimeFormat(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  }).format(new Date(epochSeconds * 1000));
}

/** The lapsed plan and its volume, e.g. "Growth (1,000,000 events / month)". */
export function unpaidPlanLabel(unpaid: UnpaidSubscription): string {
  const limit = unpaid.events_monthly_limit;
  const events =
    limit === null ? "unlimited events" : `${new Intl.NumberFormat().format(limit)} events / month`;
  return `${planLabel(unpaid.plan)} (${events})`;
}

/** What restoring it costs, e.g. "$172.00 for a month". */
export function unpaidAmountLabel(unpaid: UnpaidSubscription): string {
  const period = unpaid.billing_period === "yearly" ? "a year" : "a month";
  return `${formatMoney(unpaid.amount_cents, unpaid.currency)} for ${period}`;
}
