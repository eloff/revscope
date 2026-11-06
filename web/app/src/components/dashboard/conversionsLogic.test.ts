import { describe, expect, it } from "vitest";
import type { PaymentEntry, PaymentLanding } from "../../lib/paymentsApi";
import { nextColumnSort } from "./columnSort";
import {
  conversionsSortValue,
  DEFAULT_CONVERSIONS_SORT,
  formatAvgTimeToPurchase,
  formatTimeToPurchaseDuration,
  landingRevenueSourceTarget,
  landingSourceLabel,
  landingSourceSubtitle,
  paymentCustomerLabel,
  paymentSourceGroup,
  paymentSubscriptionType,
  paymentTimeToPurchaseSeconds,
} from "./conversionsLogic";

const DAY = 86_400;

function payment(overrides: Partial<PaymentEntry> = {}): PaymentEntry {
  return { id: 1, amount: 5000, paid_at: 10 * DAY, ...overrides } as PaymentEntry;
}

function landing(overrides: Partial<PaymentLanding> = {}): PaymentLanding {
  return { viewed_at: 9 * DAY, ...overrides } as PaymentLanding;
}

const google = { id: 4, referrer: "google.com" } as PaymentLanding["referrer"];
const campaign = {
  id: 9,
  source: "news",
  medium: "",
  campaign: "",
  display_name: "Spring",
} as PaymentLanding["campaign"];

describe("time to purchase", () => {
  it.each([
    [42, "42s"],
    [DAY, "1 day"],
    [1.5 * DAY, "1.5 days"],
    [7 * DAY, "1 week"],
    [45 * DAY, "1.5 months"],
  ])("formats %d seconds as %s", (seconds, expected) => {
    expect(formatTimeToPurchaseDuration(seconds)).toBe(expected);
  });

  it("shows a dash for no average", () => {
    expect(formatAvgTimeToPurchase(0)).toBe("—");
  });

  it("measures from the landing view, else the visitor id's timestamp", () => {
    expect(paymentTimeToPurchaseSeconds(payment({ landing: landing() }))).toBe(DAY);
    expect(
      paymentTimeToPurchaseSeconds(payment({ visitor_timestamp_id: 8 * DAY * 1_000_000_000 })),
    ).toBe(2 * DAY);
    expect(paymentTimeToPurchaseSeconds(payment())).toBeNull();
    expect(
      paymentTimeToPurchaseSeconds(payment({ landing: landing({ viewed_at: 11 * DAY }) })),
    ).toBe(0);
  });
});

describe("customer and subscription labels", () => {
  it.each([
    [{ customer_email: "a@x.io", order_email: "b@x.io" }, "a@x.io"],
    [{ order_email: "b@x.io", provider_customer_id: "cus_1" }, "b@x.io"],
    [{ provider_customer_id: "cus_1" }, "cus_1"],
    [{ visitor_timestamp_id: 0 }, "0"],
    [{}, "—"],
  ])("labels %j as %s", (overrides, expected) => {
    expect(paymentCustomerLabel(payment(overrides))).toBe(expected);
  });

  it("names the billing interval", () => {
    expect(paymentSubscriptionType(payment({ interval_unit: "year" }))).toBe("Annual");
    expect(paymentSubscriptionType(payment({ interval_unit: "fortnight" }))).toBe("fortnight");
    expect(paymentSubscriptionType(payment())).toBe("Single Payment");
  });
});

describe("payment sources", () => {
  it("groups by campaign, then referrer type, else direct", () => {
    expect(paymentSourceGroup(payment())).toBeNull();
    expect(paymentSourceGroup(payment({ landing: landing({ campaign, referrer: google }) }))).toBe(
      "campaigns",
    );
    expect(
      paymentSourceGroup(
        payment({ landing: landing({ referrer: google, referrer_type: "search" }) }),
      ),
    ).toBe("search");
    expect(
      paymentSourceGroup(
        payment({ landing: landing({ referrer: google, referrer_type: "email" }) }),
      ),
    ).toBe("campaigns");
    expect(paymentSourceGroup(payment({ landing: landing({ referrer: google }) }))).toBe(
      "referrers",
    );
    expect(paymentSourceGroup(payment({ landing: landing() }))).toBe("direct");
  });

  it("labels the source with its group and detail", () => {
    expect(landingSourceLabel(undefined)).toBe("—");
    expect(landingSourceLabel(landing())).toBe("Direct");
    expect(landingSourceLabel(landing({ referrer: google, referrer_type: "search" }))).toBe(
      "Search · google.com",
    );
    expect(landingSourceLabel(landing({ campaign }))).toBe("Campaigns · Spring");
    expect(landingSourceSubtitle(landing())).toBe("Direct");
  });

  it("links to the matching revenue source", () => {
    expect(landingRevenueSourceTarget(undefined)).toBeNull();
    expect(landingRevenueSourceTarget(landing({ campaign }))).toEqual({ kind: "campaign", id: 9 });
    expect(landingRevenueSourceTarget(landing({ referrer: google }))).toEqual({
      kind: "referrer",
      id: 4,
    });
    expect(landingRevenueSourceTarget(landing())).toEqual({ kind: "direct" });
  });
});

describe("conversions sorting", () => {
  it("flips the active column and starts others descending", () => {
    expect(nextColumnSort(DEFAULT_CONVERSIONS_SORT, "purchasedAt").direction).toBe("asc");
    expect(nextColumnSort(DEFAULT_CONVERSIONS_SORT, "amount")).toEqual({
      column: "amount",
      direction: "desc",
    });
  });

  it("sorts case-insensitively by label and puts unknown time to purchase first", () => {
    const entry = payment({ customer_email: "Zed@X.io", interval_unit: "month" });
    expect(conversionsSortValue(entry, "customer")).toBe("zed@x.io");
    expect(conversionsSortValue(entry, "subscriptionType")).toBe("monthly");
    expect(conversionsSortValue(entry, "amount")).toBe(5000);
    expect(conversionsSortValue(entry, "timeToPurchase")).toBe(-1);
  });
});
