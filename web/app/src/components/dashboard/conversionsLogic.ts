import { formatSessionDuration } from "../../lib/dashboardApi";
import type { PaymentEntry, PaymentLanding } from "../../lib/paymentsApi";
import type { ReferrerType, SourceGroup } from "../../lib/revenueSourcesApi";
import {
  campaignDisplayLabel,
  type RevenueSourceLinkTarget,
  SOURCE_GROUP_LABELS,
} from "./revenueSourcesLogic";

export type ConversionsSortColumn =
  | "customer"
  | "source"
  | "amount"
  | "subscriptionType"
  | "purchasedAt"
  | "timeToPurchase";

export type ConversionsSortDirection = "asc" | "desc";

export type ConversionsSortState = {
  column: ConversionsSortColumn;
  direction: ConversionsSortDirection;
};

export const CONVERSIONS_SORT_COLUMNS = [
  "customer",
  "source",
  "amount",
  "subscriptionType",
  "purchasedAt",
  "timeToPurchase",
] as const satisfies readonly ConversionsSortColumn[];

export const DEFAULT_CONVERSIONS_SORT: ConversionsSortState = {
  column: "purchasedAt",
  direction: "desc",
};

const NANOS_PER_SECOND = 1_000_000_000;
const DAY_SECONDS = 86_400;
const WEEK_SECONDS = 7 * DAY_SECONDS;
const MONTH_SECONDS = 30 * DAY_SECONDS;

function formatScaledDurationUnit(amount: number, singular: string): string {
  const rounded = Math.round(amount * 10) / 10;
  const text = rounded === Math.floor(rounded) ? String(Math.floor(rounded)) : rounded.toFixed(1);
  const unit = rounded === 1 ? singular : `${singular}s`;
  return `${text} ${unit}`;
}

export function formatAvgTimeToPurchase(avgSeconds: number): string {
  return avgSeconds > 0 ? formatTimeToPurchaseDuration(avgSeconds) : "—";
}

export function formatTimeToPurchaseDuration(seconds: number): string {
  const total = Math.max(0, Math.round(seconds));
  if (total >= MONTH_SECONDS) {
    return formatScaledDurationUnit(total / MONTH_SECONDS, "month");
  }
  if (total >= WEEK_SECONDS) {
    return formatScaledDurationUnit(total / WEEK_SECONDS, "week");
  }
  if (total >= DAY_SECONDS) {
    return formatScaledDurationUnit(total / DAY_SECONDS, "day");
  }
  return formatSessionDuration(total);
}

const INTERVAL_UNIT_LABELS: Record<string, string> = {
  day: "Daily",
  week: "Weekly",
  month: "Monthly",
  year: "Annual",
};

export function paymentCustomerLabel(payment: PaymentEntry): string {
  if (payment.customer_email) {
    return payment.customer_email;
  }
  if (payment.order_email) {
    return payment.order_email;
  }
  if (payment.provider_customer_id) {
    return payment.provider_customer_id;
  }
  if (payment.visitor_timestamp_id !== undefined) {
    return String(payment.visitor_timestamp_id);
  }
  return "—";
}

function groupForReferrerType(referrerType?: ReferrerType): SourceGroup {
  if (!referrerType) {
    return "referrers";
  }
  switch (referrerType) {
    case "search":
      return "search";
    case "social":
      return "social";
    case "ai":
      return "ai";
    case "email":
    case "campaign":
    case "affiliate":
      return "campaigns";
  }
}

export type AttributedLanding = Pick<PaymentLanding, "campaign" | "referrer" | "referrer_type">;

export function landingSourceGroup(landing: AttributedLanding | undefined): SourceGroup | null {
  if (!landing) {
    return null;
  }
  if (landing.campaign) {
    return "campaigns";
  }
  if (!landing.referrer?.referrer) {
    return "direct";
  }
  return groupForReferrerType(landing.referrer_type);
}

export function paymentSourceGroup(payment: PaymentEntry): SourceGroup | null {
  return landingSourceGroup(payment.landing);
}

export function landingRevenueSourceTarget(
  landing: AttributedLanding | undefined,
): RevenueSourceLinkTarget | null {
  if (!landing) {
    return null;
  }
  if (landing.campaign) {
    return { kind: "campaign", id: landing.campaign.id };
  }
  if (landing.referrer) {
    return { kind: "referrer", id: landing.referrer.id };
  }
  return { kind: "direct" };
}

export function paymentRevenueSourceTarget(payment: PaymentEntry): RevenueSourceLinkTarget | null {
  return landingRevenueSourceTarget(payment.landing);
}

export function landingSourceDetail(landing: AttributedLanding | undefined): string | null {
  if (!landing) {
    return null;
  }
  if (landing.campaign) {
    return campaignDisplayLabel(landing.campaign);
  }
  return landing.referrer?.referrer ?? null;
}

export function paymentSourceDetail(payment: PaymentEntry): string | null {
  return landingSourceDetail(payment.landing);
}

/** Source detail for display, or `"Direct"` when there is no referrer or campaign. */
export function landingSourceSubtitle(landing: AttributedLanding | undefined): string {
  return landingSourceDetail(landing) ?? "Direct";
}

export function paymentSourceSubtitle(payment: PaymentEntry): string {
  return landingSourceSubtitle(payment.landing);
}

export function landingSourceLabel(landing: AttributedLanding | undefined): string {
  const group = landingSourceGroup(landing);
  if (group === null) {
    return "—";
  }
  const parts = [SOURCE_GROUP_LABELS[group]];
  const detail = landingSourceDetail(landing);
  if (detail) {
    parts.push(detail);
  }
  return parts.join(" · ");
}

export function paymentSourceLabel(payment: PaymentEntry): string {
  return landingSourceLabel(payment.landing);
}

export function paymentSubscriptionType(payment: PaymentEntry): string {
  if (payment.interval_unit) {
    return INTERVAL_UNIT_LABELS[payment.interval_unit] ?? payment.interval_unit;
  }
  return "Single Payment";
}

export function paymentTimeToPurchaseSeconds(payment: PaymentEntry): number | null {
  const viewedAt =
    payment.landing?.viewed_at ??
    (payment.visitor_timestamp_id !== undefined
      ? Math.floor(payment.visitor_timestamp_id / NANOS_PER_SECOND)
      : undefined);
  if (viewedAt === undefined) {
    return null;
  }
  return Math.max(0, payment.paid_at - viewedAt);
}

export function formatPaidAt(unixSeconds: number, timeZone: string): string {
  return new Date(unixSeconds * 1000).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
    hour: "numeric",
    minute: "2-digit",
    timeZone,
  });
}

export function conversionsSortValue(
  payment: PaymentEntry,
  column: ConversionsSortColumn,
): number | string {
  switch (column) {
    case "customer":
      return paymentCustomerLabel(payment).toLowerCase();
    case "source":
      return paymentSourceLabel(payment).toLowerCase();
    case "amount":
      return payment.amount;
    case "subscriptionType":
      return paymentSubscriptionType(payment).toLowerCase();
    case "purchasedAt":
      return payment.paid_at;
    case "timeToPurchase":
      return paymentTimeToPurchaseSeconds(payment) ?? -1;
  }
}
