import { describe, expect, it } from "vitest";
import { testBucket, testOverview } from "../../../test/dashboardFixtures";
import type { AudienceResponse } from "../../lib/audienceApi";
import type { CrawlersResponse } from "../../lib/crawlersApi";
import {
  hasAudienceData,
  hasCrawlersData,
  hasOverviewData,
  hasPaymentActivityData,
} from "./tabDataPresence";

describe("tab data presence", () => {
  it("counts visitors, conversions, revenue or MRR as overview data", () => {
    expect(hasOverviewData(testOverview())).toBe(false);
    expect(hasOverviewData(testOverview({ buckets: [testBucket({ visitors: 1 })] }))).toBe(true);
    expect(hasOverviewData(testOverview({ mrr: 1 }))).toBe(true);
    expect(
      hasOverviewData(testOverview({ current: { ...testOverview().current, total_revenue: 5 } })),
    ).toBe(true);
  });

  it("counts payment days or any non-zero month as revenue activity", () => {
    const empty = { start: 0, end: 0, days: [], monthly_revenue: [{ months_ago: 0, revenue: 0 }] };
    expect(hasPaymentActivityData(empty)).toBe(false);
    expect(
      hasPaymentActivityData({ ...empty, monthly_revenue: [{ months_ago: 3, revenue: -10 }] }),
    ).toBe(true);
  });

  it("counts any audience breakdown row", () => {
    const empty = {
      countries: { current: [] },
      cities: { current: [] },
      browsers: { current: [] },
      os: { current: [] },
      devices: { current: [] },
      screen_sizes: { current: [] },
      viewports: { current: [] },
    } as unknown as AudienceResponse;
    expect(hasAudienceData(empty)).toBe(false);
    expect(
      hasAudienceData({ ...empty, viewports: { current: [{}] } } as unknown as AudienceResponse),
    ).toBe(true);
  });

  it("counts crawler category totals even when this page has no rows", () => {
    const empty = {
      rows: [],
      operators: [],
      category_totals: { archive: 0 },
    } as unknown as CrawlersResponse;
    expect(hasCrawlersData(empty)).toBe(false);
    expect(
      hasCrawlersData({ ...empty, category_totals: { archive: 3 } } as unknown as CrawlersResponse),
    ).toBe(true);
  });
});
