import { describe, expect, it } from "vitest";
import { testBucket, testOverview, testPeriodMetrics } from "../../../test/dashboardFixtures";
import type { PageRow } from "../../lib/pagesApi";
import type { GroupSummary } from "../../lib/revenueSourcesApi";
import type { SearchTermRow, SearchTermsPeriod } from "../../lib/searchTermsApi";
import {
  buildMetricHoverContent,
  highlightMetricLabel,
  highlightTone,
  type MetricHoverContent,
} from "./metricHoverDetails";
import { buildPageHoverContent } from "./pageHoverDetails";
import {
  buildBestSourceHoverContent,
  buildGroupSummaryHoverContent,
} from "./revenueSourceHoverDetails";
import type { RevenueSourceRecord } from "./revenueSourcesLogic";
import {
  buildSearchTermHoverContent,
  buildSearchTermOtherHoverContent,
  indexSearchTermsByQuery,
} from "./searchTermHoverDetails";

function stat(content: MetricHoverContent, label: string) {
  return content.stats.find((line) => line.label === label);
}

describe("highlights", () => {
  it("labels metrics and derives their tone", () => {
    expect(highlightMetricLabel("revenue")).toBe("Best revenue");
    expect(highlightTone({ metric: "revenue" })).toBe("positive");
    expect(highlightTone({ metric: "highestExitRate" })).toBe("negative");
    expect(highlightTone({ description: "x" })).toBe("positive");
    expect(highlightTone({ description: "x", tone: "negative" })).toBe("negative");
  });
});

describe("buildMetricHoverContent", () => {
  const overview = testOverview({
    mrr: 12_000_000,
    previous_mrr: 10_000_000,
    mrr_growth: { monthly: 0.2, annualized: 0 },
    buckets: [
      testBucket({
        visitors: 100,
        bounced_sessions: 40,
        recurring_subscriptions: 60_000,
        new_subscriptions: 30_000,
        other_revenue: 10_000,
        refunds: -5_000,
        chargebacks: -1_000,
        other_payments_count: 4,
      }),
    ],
    totals_changes: { ...testOverview().totals_changes, refunds: { ratio: 0.5 } },
  });

  it("breaks total revenue into shares of gross, with refunds as a cost", () => {
    const content = buildMetricHoverContent("total_revenue", overview, testPeriodMetrics());
    expect(stat(content, "Recurring")?.value).toBe("$60 (60.0%)");
    expect(stat(content, "Refunds")).toMatchObject({
      value: "$5 (5.0%)",
      changeRatio: 0.5,
      higherIsBetter: false,
    });
    expect(stat(content, "Net after refunds")?.value).toBe("$94");
  });

  it("drops the share when a line is the whole total", () => {
    const content = buildMetricHoverContent(
      "new_revenue",
      overview,
      testPeriodMetrics({ new_revenue: 10_005 }),
    );
    expect(stat(content, "Non-subscription")?.value).toBe("$10");
    expect(stat(content, "Avg payment")?.value).toBe("$2.50");
  });

  it("shows MRR movement and growth rates", () => {
    const content = buildMetricHoverContent(
      "mrr",
      overview,
      testPeriodMetrics({ new_subscriptions_mrr: 3_000_000, churn_mrr: 1_000_000 }),
    );
    expect(stat(content, "Net MRR change")?.value).toBe("$2.0k");
    expect(stat(content, "Monthly growth rate")).toMatchObject({
      changeRatio: 0.2,
      changeInValueColumn: true,
    });
    expect(stat(content, "Annualized growth rate")).toMatchObject({
      changeRatio: 0,
      changeInValueColumn: true,
    });
    expect(
      stat(
        buildMetricHoverContent("mrr", testOverview(), testPeriodMetrics()),
        "Monthly growth rate",
      )?.value,
    ).toBe("—");
  });

  it("uses a dash for averages over nothing", () => {
    const empty = testOverview();
    expect(
      stat(buildMetricHoverContent("churn_mrr", empty, testPeriodMetrics()), "Avg MRR per churn")
        ?.value,
    ).toBe("—");
    expect(
      stat(buildMetricHoverContent("visitors", empty, testPeriodMetrics()), "Conversion share")
        ?.value,
    ).toBe("—");
  });

  it("only compares bounce rate and session time when both periods had visitors", () => {
    const metrics = testPeriodMetrics({ bounce_rate: 0.4 });
    const noBaseline = buildMetricHoverContent("bounce_rate", overview, metrics);
    expect(stat(noBaseline, "Bounce rate")).toEqual({ label: "Bounce rate", value: "40.0%" });
    expect(stat(noBaseline, "Engaged visitors")?.value).toBe("60");

    const withBaseline = {
      ...overview,
      previous_totals: { ...overview.previous_totals, visitors: 10 },
      changes: { ...overview.changes, bounce_rate: { ratio: -0.1 }, session_time: { ratio: 0.3 } },
    };
    expect(
      stat(buildMetricHoverContent("bounce_rate", withBaseline, metrics), "Bounce rate"),
    ).toMatchObject({
      changeRatio: -0.1,
      higherIsBetter: false,
    });
    expect(
      stat(buildMetricHoverContent("session_time", withBaseline, metrics), "Average per visitor")
        ?.changeRatio,
    ).toBe(0.3);
  });

  it("builds content for every tile", () => {
    for (const key of Object.keys(testPeriodMetrics()) as (keyof ReturnType<
      typeof testPeriodMetrics
    >)[]) {
      expect(
        buildMetricHoverContent(key, overview, testPeriodMetrics()).stats.length,
      ).toBeGreaterThan(0);
    }
  });
});

describe("buildPageHoverContent", () => {
  const row = {
    visitors: 1200,
    attributed_revenue: 5000,
    revenue_per_visitor: 4,
    converting_visitors: 3,
    bounce_rate: 0.25,
    exit_rate: 0.5,
    avg_session_duration: 65,
    avg_page_duration: 30,
    avg_scroll_percent: 80,
    increase_visitors_25pct: 1000,
    increase_conversion_25pct: 2000,
    reduce_exits_25pct: 3000,
    reduce_bounces_25pct: 4000,
    increase_session_25pct: 5000,
  } as PageRow;

  it("shows exits for pages and bounces for landing pages", () => {
    const pages = buildPageHoverContent(row, "pages");
    const landing = buildPageHoverContent(row, "landingPages", [{ metric: "bounceUplift" }]);
    expect(stat(pages, "Attributed revenue")?.value).toBe("$5");
    expect(stat(pages, "Exit rate")?.value).toBe("50.0%");
    expect(stat(pages, "Value of −25% exit rate")?.value).toBe("$3");
    expect(stat(pages, "Bounce rate")).toBeUndefined();
    expect(stat(landing, "Revenue")?.value).toBe("$5");
    expect(stat(landing, "Bounce rate")?.value).toBe("25.0%");
    expect(stat(landing, "Value of −25% bounce rate")?.value).toBe("$4");
    expect(stat(landing, "Avg scroll depth")?.value).toBe("80.0%");
    expect(landing.highlightDescriptions).toEqual([{ metric: "bounceUplift" }]);
  });
});

describe("revenue source hover content", () => {
  it("summarizes a group with its changes", () => {
    const content = buildGroupSummaryHoverContent({
      revenue: 9000,
      visitors: 30,
      revenue_per_visitor: 300,
      conversion_rate: 0.1,
      revenue_change: { ratio: 0.5 },
      visitors_change: { ratio: null },
      revenue_per_visitor_change: { ratio: 0 },
      conversion_rate_change: { ratio: -0.1 },
    } as GroupSummary);
    expect(stat(content, "Revenue")).toMatchObject({ value: "$9", changeRatio: 0.5 });
    expect(stat(content, "Conversion 95% CI")?.value).toBe("—");
  });

  it("lists referrers only for campaign rows", () => {
    const record = {
      revenue: 1000,
      visitors: 10,
      convertingVisitors: 2,
      conversionRate: 0.2,
      revenuePerVisitor: 100,
      landingPaths: [{ id: 1, path: "/a", count: 4 }],
      referrers: [{ label: "news.site", visitors: 3 }],
      isCampaign: false,
    } as unknown as RevenueSourceRecord;
    expect(buildBestSourceHoverContent(record).referrers).toBeUndefined();
    const campaign = buildBestSourceHoverContent({ ...record, isCampaign: true }, []);
    expect(campaign.referrers).toEqual([{ id: 0, path: "news.site", visitors: 3 }]);
    expect(campaign.landingPaths).toEqual([{ id: 1, path: "/a", visitors: 4 }]);
    expect(campaign).not.toHaveProperty("highlightDescriptions");
  });
});

describe("search term hover content", () => {
  function term(overrides: Partial<SearchTermRow>): SearchTermRow {
    const none = { ratio: null };
    return {
      query: "q",
      landing_paths: [],
      position: 1,
      impressions: 0,
      clicks: 0,
      mobile_clicks: 0,
      detail_clicks: 0,
      estimated_revenue: 0,
      ctr: 0,
      mobile_share: 0,
      expected_ctr: 0,
      lost_clicks: 0,
      lost_revenue: 0,
      highest_lost_clicks: false,
      highest_cannibalization: false,
      position_change: none,
      impressions_change: none,
      clicks_change: none,
      ctr_change: none,
      mobile_share_change: none,
      estimated_revenue_change: none,
      ...overrides,
    };
  }

  it("treats a lower position as better and adds lost-click lines in ranks 1–10", () => {
    const content = buildSearchTermHoverContent(
      term({ position: 3.25, expected_ctr: 0.1, lost_clicks: 12.6, lost_revenue: 4000 }),
    );
    expect(stat(content, "Position")).toMatchObject({ value: "3.3", higherIsBetter: false });
    expect(stat(content, "Lost clicks")?.value).toBe("13");
    expect(stat(buildSearchTermHoverContent(term({})), "Lost clicks")).toBeUndefined();
  });

  it("orders landing paths by clicks with their share", () => {
    const content = buildSearchTermHoverContent(
      term({
        landing_paths: [
          { path_id: 1, path: "/b", clicks: 1 },
          { path_id: 2, path: "/a", clicks: 3 },
        ],
      }),
    );
    expect(content.landingPaths).toEqual([
      { id: 2, path: "/a", visitors: 3, share: 0.75 },
      { id: 1, path: "/b", visitors: 1, share: 0.25 },
    ]);
    expect(content.landingPathsCountLabel).toBe("Clicks");
  });

  it("indexes terms by row key", () => {
    expect([...indexSearchTermsByQuery([term({ query: "a" })]).keys()]).toEqual(["term:a"]);
  });

  describe("Other tile", () => {
    const period = (
      terms: SearchTermRow[],
      impressions: number,
      clicks: number,
    ): SearchTermsPeriod =>
      ({
        search_channel_revenue: 10_000,
        summary: { impressions, clicks },
        terms,
      }) as SearchTermsPeriod;

    it("covers what the top terms leave over", () => {
      const content = buildSearchTermOtherHoverContent(
        period(
          [
            term({ query: "top", impressions: 100, clicks: 10, estimated_revenue: 6000 }),
            term({
              query: "a",
              position: 4,
              impressions: 50,
              clicks: 5,
              mobile_clicks: 1,
              detail_clicks: 4,
            }),
            term({ query: "b", position: 8, impressions: 50, clicks: 5 }),
          ],
          200,
          20,
        ),
        new Set(["top"]),
        1,
      );
      expect(content.description).toBe("Estimated search revenue outside the top 1 terms");
      expect(stat(content, "Est. revenue")?.value).toBe("$4");
      expect(stat(content, "Impressions")?.value).toBe("100");
      expect(stat(content, "Position")?.value).toBe("6.0");
      expect(stat(content, "CTR")?.value).toBe("10.00%");
      expect(stat(content, "% Mobile")?.value).toBe("25.0%");
    });

    it("averages position over loaded terms only, not impressions on later pages", () => {
      const content = buildSearchTermOtherHoverContent(
        period(
          [
            term({ query: "top", impressions: 100 }),
            term({ query: "a", position: 9, impressions: 100 }),
          ],
          // 800 more impressions belong to terms on pages not loaded yet.
          1000,
          0,
        ),
        new Set(["top"]),
        1,
      );
      expect(stat(content, "Position")?.value).toBe("9.0");
    });

    it("omits position when no loaded term is outside the top", () => {
      const content = buildSearchTermOtherHoverContent(
        period([term({ query: "top", impressions: 100 })], 500, 10),
        new Set(["top"]),
        1,
      );
      expect(stat(content, "Position")).toBeUndefined();
      expect(stat(content, "CTR")?.value).toBe("2.50%");
    });
  });
});
