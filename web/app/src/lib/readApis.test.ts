import { describe, expect, it } from "vitest";
import { mockApi, queryOf } from "../../../shared/test/api";
import { testSession } from "../../../shared/test/fixtures";
import { DEFAULT_AUDIENCE_CITIES_SORT, fetchAudience } from "./audienceApi";
import {
  DEFAULT_CRAWLERS_SORT,
  fetchCrawlerDetail,
  fetchCrawlers,
  last3DaysValues,
} from "./crawlersApi";
import { fetchOverview } from "./dashboardApi";
import { DEFAULT_EVENTS_SORT, fetchEvents } from "./eventsApi";
import {
  DEFAULT_PAGES_SORT,
  fetchLandingPages,
  fetchPages,
  fetchPageTransitions,
} from "./pagesApi";
import { fetchPaymentActivity } from "./paymentActivityApi";
import { fetchConversions, fetchPaymentJourney, fetchPayments } from "./paymentsApi";
import { fetchRevenueSources } from "./revenueSourcesApi";
import { DEFAULT_SEARCH_TERMS_SORT, fetchSearchTerms } from "./searchTermsApi";
import { EMPTY_SEGMENT_FILTERS, type SegmentFilters } from "./segmentFilters";
import { DEFAULT_VISITOR_SORT, fetchVisitorJourney, fetchVisitors } from "./visitorsApi";

const session = testSession();
const DAY = 86_400;
const START = 1_700_000_000;
const END = START + 7 * DAY;
const RANGE = { start: String(START), end: String(END) };

const segments: SegmentFilters = {
  ...EMPTY_SEGMENT_FILTERS,
  countries: [{ code: "US", name: "United States" }],
  sources: [{ kind: "campaign", id: 3, name: "Spring" }],
};

type ReadCase = {
  name: string;
  resource: string;
  /** The signed-in call; the public variant is checked separately where one exists. */
  call: () => Promise<unknown>;
  publicCall?: () => Promise<unknown>;
  query: Record<string, string>;
  error: string;
};

const cases: ReadCase[] = [
  {
    name: "overview",
    resource: "overview",
    call: () => fetchOverview(session, "42", START, END),
    publicCall: () => fetchOverview(undefined, "42", START, END),
    query: { ...RANGE, bucket: "daily" },
    error: "Unable to load overview data.",
  },
  {
    name: "revenue sources",
    resource: "revenue-sources",
    call: () => fetchRevenueSources(session, "42", START, END),
    publicCall: () => fetchRevenueSources(undefined, "42", START, END),
    query: RANGE,
    error: "Unable to load sources data.",
  },
  {
    name: "pages",
    resource: "pages",
    call: () => fetchPages(session, "42", START, END, 200, DEFAULT_PAGES_SORT),
    publicCall: () => fetchPages(undefined, "42", START, END, 200, DEFAULT_PAGES_SORT),
    query: { ...RANGE, offset: "200", order_by: "attributed_revenue", direction: "desc" },
    error: "Unable to load pages data.",
  },
  {
    name: "landing pages",
    resource: "landing-pages",
    call: () => fetchLandingPages(session, "42", START, END, 0, DEFAULT_PAGES_SORT),
    publicCall: () => fetchLandingPages(undefined, "42", START, END, 0, DEFAULT_PAGES_SORT),
    query: { ...RANGE, offset: "0", order_by: "attributed_revenue", direction: "desc" },
    error: "Unable to load pages data.",
  },
  {
    name: "page transitions",
    resource: "page-transitions",
    call: () => fetchPageTransitions(session, "42", START, END, 9, "landingPages"),
    publicCall: () => fetchPageTransitions(undefined, "42", START, END, 9, "pages"),
    query: { ...RANGE, path_id: "9", variant: "landing_pages" },
    error: "Unable to load page transitions.",
  },
  {
    name: "events",
    resource: "events",
    call: () => fetchEvents(session, "42", START, END, 0, DEFAULT_EVENTS_SORT),
    publicCall: () => fetchEvents(undefined, "42", START, END, 0, DEFAULT_EVENTS_SORT),
    query: { ...RANGE, offset: "0", order_by: "revenue", direction: "desc" },
    error: "Unable to load events data.",
  },
  {
    name: "search terms",
    resource: "search-terms",
    call: () => fetchSearchTerms(session, "42", START, END, 100, DEFAULT_SEARCH_TERMS_SORT),
    publicCall: () => fetchSearchTerms(undefined, "42", START, END, 100, DEFAULT_SEARCH_TERMS_SORT),
    query: { ...RANGE, offset: "100", order_by: "estimated_revenue", direction: "desc" },
    error: "Unable to load search terms data.",
  },
  {
    name: "audience",
    resource: "audience",
    call: () => fetchAudience(session, "42", START, END, 0, DEFAULT_AUDIENCE_CITIES_SORT),
    publicCall: () => fetchAudience(undefined, "42", START, END, 0, DEFAULT_AUDIENCE_CITIES_SORT),
    query: { ...RANGE, offset: "0", order_by: "revenue", direction: "desc" },
    error: "Unable to load audience data.",
  },
  {
    name: "crawlers",
    resource: "crawlers",
    call: () => fetchCrawlers(session, "42", START, END, 0, DEFAULT_CRAWLERS_SORT),
    publicCall: () => fetchCrawlers(undefined, "42", START, END, 0, DEFAULT_CRAWLERS_SORT),
    query: { ...RANGE, offset: "0", order_by: "views", direction: "desc" },
    error: "Unable to load crawlers data.",
  },
  {
    name: "crawler detail",
    resource: "crawler-detail",
    call: () => fetchCrawlerDetail(session, "42", START, START + DAY, 5, 1, true),
    publicCall: () => fetchCrawlerDetail(undefined, "42", START, START + DAY, 5, 1, false),
    query: {
      start: String(START),
      end: String(START + DAY),
      useragent_id: "5",
      source: "1",
      verified: "1",
      bucket: "hourly",
    },
    error: "Unable to load crawler details.",
  },
  {
    name: "visitors",
    resource: "visitors",
    call: () => fetchVisitors(session, "42", START, END, 0, DEFAULT_VISITOR_SORT),
    publicCall: () => fetchVisitors(undefined, "42", START, END, 0, DEFAULT_VISITOR_SORT),
    query: { ...RANGE, offset: "0", order_by: "last_view_at", direction: "desc" },
    error: "Unable to load visitors data.",
  },
  {
    name: "visitor journey",
    resource: "visitor-journey",
    call: () => fetchVisitorJourney(session, "42", "abc-1"),
    publicCall: () => fetchVisitorJourney(undefined, "42", "abc-1"),
    query: { visitor_timestamp_id: "abc-1" },
    error: "Unable to load visitor journey.",
  },
  {
    name: "conversions",
    resource: "conversions",
    call: () => fetchConversions(session, "42", START, END),
    publicCall: () => fetchConversions(undefined, "42", START, END),
    query: { ...RANGE, limit: "100" },
    error: "Unable to load conversions data.",
  },
  {
    name: "payments",
    resource: "conversions",
    call: () => fetchPayments(session, "42", START, END),
    publicCall: () => fetchPayments(undefined, "42", START, END),
    query: { ...RANGE, limit: "100" },
    error: "Unable to load conversions data.",
  },
  {
    name: "payment journey",
    resource: "payment-journey",
    call: () => fetchPaymentJourney(session, "42", 77),
    publicCall: () => fetchPaymentJourney(undefined, "42", 77),
    query: { payment_id: "77" },
    error: "Unable to load customer journey.",
  },
  {
    name: "payment activity",
    resource: "payment-activity",
    call: () => fetchPaymentActivity(session, "42"),
    publicCall: () => fetchPaymentActivity(undefined, "42"),
    query: {},
    error: "Unable to load payment activity.",
  },
];

describe.each(cases)("$name client", ({ resource, call, publicCall, query, error }) => {
  const path = `/v2/read/projects/42/${resource}`;

  it("sends the cookie and the expected query, and returns the payload", async () => {
    const calls = mockApi("get", path, { ok: "yes" });
    await expect(call()).resolves.toEqual({ ok: "yes" });
    expect(calls).toHaveLength(1);
    expect(calls[0]?.credentials).toBe("include");
    expect(queryOf(calls[0])).toEqual(query);
  });

  it("omits the cookie for the public variant", async () => {
    const calls = mockApi("get", path, {});
    await publicCall?.();
    expect(calls[0]?.credentials).toBe("omit");
  });

  it("throws the server's error", async () => {
    mockApi("get", path, { error: "Project not found." }, 404);
    await expect(call()).rejects.toThrow("Project not found.");
  });

  it("falls back to a default error", async () => {
    mockApi("get", path, {}, 500);
    await expect(call()).rejects.toThrow(error);
  });
});

describe("segment and filter params", () => {
  it("appends segment filters to list requests", async () => {
    const calls = mockApi("get", "/v2/read/projects/42/events", {});
    await fetchEvents(session, "42", START, END, 0, DEFAULT_EVENTS_SORT, segments);
    expect(calls[0]?.url.searchParams.getAll("country")).toEqual(["US"]);
    expect(calls[0]?.url.searchParams.getAll("source")).toEqual(["campaign-3"]);
  });

  it("replaces an unknown sort column with the default", async () => {
    const calls = mockApi("get", "/v2/read/projects/42/pages", {});
    await fetchPages(session, "42", START, END, 0, {
      column: "dropped_column",
      direction: "asc",
    } as unknown as typeof DEFAULT_PAGES_SORT);
    expect(queryOf(calls[0])).toMatchObject({ order_by: "attributed_revenue", direction: "asc" });
  });

  it("sends each disabled crawler category", async () => {
    const calls = mockApi("get", "/v2/read/projects/42/crawlers", {});
    await fetchCrawlers(session, "42", START, END, 0, DEFAULT_CRAWLERS_SORT, [
      "ai_training",
      "seo",
    ] as never);
    expect(calls[0]?.url.searchParams.getAll("disabled_category")).toEqual(["ai_training", "seo"]);
  });

  it("sends the visitors session filter", async () => {
    const calls = mockApi("get", "/v2/read/projects/42/visitors", {});
    await fetchVisitors(session, "42", START, END, 0, DEFAULT_VISITOR_SORT, {
      kind: "event",
      eventNameId: 8,
      pathId: 4,
    });
    expect(queryOf(calls[0])).toMatchObject({ filter: "event", path_id: "4", event_name_id: "8" });
  });

  it("sends every conversions filter and the cursor", async () => {
    const calls = mockApi("get", "/v2/read/projects/42/conversions", {});
    await fetchConversions(session, "42", START, END, "next-page", {
      pathId: 1,
      landingPathId: 2,
      campaignId: 3,
      referrerId: 4,
      direct: true,
    });
    expect(queryOf(calls[0])).toMatchObject({
      path_id: "1",
      landing_path_id: "2",
      campaign_id: "3",
      referrer_id: "4",
      direct: "1",
      cursor: "next-page",
    });
  });

  it("picks the overview bucket from the range unless one is given", async () => {
    const calls = mockApi("get", "/v2/read/projects/42/overview", {});
    await fetchOverview(session, "42", START, START + DAY);
    await fetchOverview(session, "42", START, START + 90 * DAY);
    await fetchOverview(session, "42", START, START + DAY, EMPTY_SEGMENT_FILTERS, "weekly");
    expect(calls.map((call) => call.url.searchParams.get("bucket"))).toEqual([
      "hourly",
      "weekly",
      "weekly",
    ]);
  });
});

describe("last3DaysValues", () => {
  it("expands an empty series to 18 zero buckets", () => {
    expect(last3DaysValues([])).toEqual(Array(18).fill(0));
    expect(last3DaysValues(undefined)).toHaveLength(18);
    expect(last3DaysValues([1, 2])).toEqual([1, 2]);
  });
});
