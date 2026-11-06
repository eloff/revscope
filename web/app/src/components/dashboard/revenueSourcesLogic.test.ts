import { describe, expect, it } from "vitest";
import type {
  GroupSummaries,
  RevenueSourcePeriod,
  SourceGroup,
  SourceMetrics,
} from "../../lib/revenueSourcesApi";
import { nextColumnSort } from "./columnSort";
import {
  buildGroupOtherRecords,
  buildTreemapLeaves,
  campaignDisplayLabel,
  DEFAULT_SORT,
  findBestSourceHighlights,
  formatTileChangeRatio,
  groupTotalsFromResponse,
  isSourceGroupSoloed,
  paymentFiltersForRecord,
  type RevenueSourceRecord,
  rowsFromResponse,
  SOURCE_GROUP_ORDER,
  soloSourceGroup,
  sortRecords,
  sourceFilterFromRecord,
  toggleDisabledSourceGroup,
} from "./revenueSourcesLogic";

function metrics(visitors: number, converting: number, revenue: number): SourceMetrics {
  const conversionRate = visitors > 0 ? converting / visitors : 0;
  return {
    visitors,
    converting_visitors: converting,
    revenue,
    conversion_rate: conversionRate,
    revenue_per_visitor: visitors > 0 ? revenue / visitors : 0,
    conversion_lower_bound: conversionRate / 2,
    revenue_change: { ratio: null },
    visitors_change: { ratio: null },
    conversions_change: { ratio: null },
    revenue_per_visitor_change: { ratio: null },
    conversion_rate_change: { ratio: null },
  };
}

function period(entries: RevenueSourcePeriod["entries"]): RevenueSourcePeriod {
  return {
    entries,
    total_visitors: 0,
    total_revenue: 0,
    direct_visitors: 0,
    direct_revenue: 0,
    referrer_visitors: 0,
    referrer_revenue: 0,
    campaign_visitors: 0,
    campaign_revenue: 0,
    search_visitors: 0,
    search_revenue: 0,
    ai_visitors: 0,
    ai_revenue: 0,
    social_visitors: 0,
    social_revenue: 0,
    campaigns_visitors: 0,
    campaigns_revenue: 0,
    referrers_group_visitors: 0,
    referrers_group_revenue: 0,
  };
}

const response = period([
  {
    kind: "referrer",
    id: 3,
    referrer: "news.ycombinator.com/item",
    referrer_type: "social",
    group: "social",
    ...metrics(100, 2, 50_000),
  },
  { kind: "referrer", group: "direct", ...metrics(400, 4, 90_000) },
  {
    kind: "campaign",
    id: 7,
    source: "newsletter",
    medium: "email",
    campaign: "launch",
    group: "campaigns",
    referrers: [
      { referrer: "", count: 3 },
      { referrer: "mail.google.com", count: 2 },
    ],
    ...metrics(50, 1, 90_000),
  },
]);

describe("rowsFromResponse", () => {
  const rows = rowsFromResponse(response);

  it("orders by revenue, with visitors breaking ties", () => {
    expect(rows.map((row) => row.key)).toEqual(["d", "c:7", "r:3"]);
  });

  it("gives each kind of source its identity and label", () => {
    const [direct, campaign, referrer] = rows;
    expect(direct).toMatchObject({ label: "Direct", isDirect: true, referrerId: null });
    expect(campaign).toMatchObject({
      label: "source=newsletter, medium=email, campaign=launch",
      isCampaign: true,
      campaignId: 7,
      referrers: [
        { label: "Direct", visitors: 3 },
        { label: "mail.google.com", visitors: 2 },
      ],
    });
    expect(referrer).toMatchObject({
      label: "news.ycombinator.com/item",
      referrerId: 3,
      referrerType: "social",
    });
  });

  it("maps each row to the drill-down and segment filters for it", () => {
    const [direct, campaign, referrer] = rows;
    expect(paymentFiltersForRecord(direct)).toEqual({ direct: true });
    expect(paymentFiltersForRecord(campaign)).toEqual({ campaignId: 7 });
    expect(paymentFiltersForRecord(referrer)).toEqual({ referrerId: 3 });
    expect(sourceFilterFromRecord(campaign)).toEqual({
      kind: "campaign",
      id: 7,
      name: campaign.label,
    });
    expect(sourceFilterFromRecord(direct)).toEqual({ kind: "direct", name: "Direct" });
  });
});

describe("campaignDisplayLabel", () => {
  it("prefers the display name, then the non-empty attribution fields", () => {
    const base = { source: "", medium: "", campaign: "", network: undefined };
    expect(campaignDisplayLabel({ ...base, display_name: "Spring sale" })).toBe("Spring sale");
    expect(campaignDisplayLabel({ ...base, network: "google_ads", campaign: "brand" })).toBe(
      "ads=google_ads, campaign=brand",
    );
    expect(campaignDisplayLabel({ ...base, network: "generic", source: "x" })).toBe("source=x");
    expect(campaignDisplayLabel(base)).toBe("(unnamed campaign)");
  });
});

describe("group Other rows", () => {
  const rows = rowsFromResponse(response);
  const totals = groupTotalsFromResponse({
    ...response,
    social_visitors: 160,
    social_revenue: 70_000,
    campaigns_visitors: 50,
    campaigns_revenue: 90_000,
    direct_visitors: 999,
    direct_revenue: 999_000,
  });

  it("fills the gap between a group's total and the rows shown", () => {
    const others = buildGroupOtherRecords(rows, totals);
    expect(others).toHaveLength(1);
    expect(others[0]).toMatchObject({
      group: "social",
      label: "Other",
      isGroupOther: true,
      visitors: 60,
      revenue: 20_000,
    });
    expect(paymentFiltersForRecord(others[0])).toBeNull();
    expect(sourceFilterFromRecord(others[0])).toBeNull();
  });

  it("leaves out disabled groups and never builds one for direct", () => {
    expect(buildGroupOtherRecords(rows, totals, new Set<SourceGroup>(["social"]))).toEqual([]);
  });

  it("builds treemap tiles from the top sources plus Other, skipping zero revenue", () => {
    const leaves = buildTreemapLeaves(rows, totals, 2);
    expect(leaves.get("direct")?.map((leaf) => leaf.label)).toEqual(["Direct"]);
    expect(leaves.get("campaigns")?.map((leaf) => leaf.datumKey)).toEqual(["c:7"]);
    // The HN row fell outside the top two, so all of social is Other.
    expect(leaves.get("social")?.map((leaf) => [leaf.label, leaf.revenue])).toEqual([
      ["Other", 70_000],
    ]);
    expect(leaves.get("search")).toEqual([]);
  });
});

describe("source group toggles", () => {
  it("solos a group, and a second solo brings every group back", () => {
    const soloed = soloSourceGroup("ai");
    expect(soloed.size).toBe(SOURCE_GROUP_ORDER.length - 1);
    expect(isSourceGroupSoloed("ai", soloed)).toBe(true);
    expect(soloSourceGroup("ai", soloed).size).toBe(0);
  });

  it("toggles without mutating the set it was given", () => {
    const disabled = new Set<SourceGroup>(["ai"]);
    expect([...toggleDisabledSourceGroup("ai", disabled)]).toEqual([]);
    expect([...toggleDisabledSourceGroup("search", disabled)].sort()).toEqual(["ai", "search"]);
    expect([...disabled]).toEqual(["ai"]);
  });
});

describe("sorting", () => {
  const rows = rowsFromResponse(response);

  it("starts a new column descending and flips the active one", () => {
    expect(nextColumnSort(DEFAULT_SORT, "visitors")).toEqual({
      column: "visitors",
      direction: "desc",
    });
    expect(nextColumnSort(DEFAULT_SORT, "revenue")).toEqual({
      column: "revenue",
      direction: "asc",
    });
  });

  it("orders by the column, then visitors, then key", () => {
    const keys = (column: typeof DEFAULT_SORT.column, direction: "asc" | "desc") =>
      sortRecords(rows, { column, direction }).map((row) => row.key);
    expect(keys("visitors", "desc")).toEqual(["d", "r:3", "c:7"]);
    expect(keys("source", "asc")).toEqual(["d", "r:3", "c:7"]);
    expect(keys("group", "asc")).toEqual(["r:3", "c:7", "d"]);
    // Direct and the campaign tie on revenue; Direct has more visitors.
    expect(keys("revenue", "desc")).toEqual(["d", "c:7", "r:3"]);
    expect(keys("revenue", "asc")).toEqual(["r:3", "d", "c:7"]);
  });
});

describe("findBestSourceHighlights", () => {
  function record(
    key: string,
    group: SourceGroup,
    visitors: number,
    converting: number,
    revenue: number,
  ): RevenueSourceRecord {
    const [row] = rowsFromResponse(
      period([
        {
          kind: "referrer",
          id: 1,
          referrer: key,
          group,
          ...metrics(visitors, converting, revenue),
        },
      ]),
    );
    return { ...row, key };
  }

  it("stars the best row per group, with ties sharing it, and never a 0% rate", () => {
    const rows = [
      record("a", "social", 100, 10, 5_000),
      record("b", "social", 100, 0, 5_000),
      record("c", "referrers", 10, 0, 0),
      record("d", "search", 10, 5, 9_000),
    ];
    const highlights = findBestSourceHighlights(rows);
    expect([...highlights.revenue].sort()).toEqual(["a", "b"]);
    expect([...highlights.conversion]).toEqual(["a"]);
    // Nothing in referrers earned anything, and search is not highlighted.
    expect(highlights.revenue.has("c")).toBe(false);
    expect(highlights.revenue.has("d")).toBe(false);
  });

  it("warns about the source losing the most converters against its group's rate", () => {
    const rows = [
      record("big-leak", "campaigns", 1_000, 10, 1),
      record("small-leak", "campaigns", 100, 1, 1),
      record("no-sales", "campaigns", 5_000, 0, 0),
    ];
    const groups = { campaigns: { conversion_rate: 0.05 } } as unknown as GroupSummaries;
    expect([...findBestSourceHighlights(rows, groups).conversionLeak]).toEqual([]);

    const withoutZero = rows.slice(0, 2);
    expect([...findBestSourceHighlights(withoutZero, groups).conversionLeak]).toEqual(["big-leak"]);
  });
});

describe("change formatting", () => {
  it("formats the change with an arrow", () => {
    expect(formatTileChangeRatio(0.123)).toBe("▲12%");
    expect(formatTileChangeRatio(-0.054)).toBe("▼5.4%");
    expect(formatTileChangeRatio(0.05)).toBe("▲5%");
    expect(formatTileChangeRatio(0.0996)).toBe("▲10%");
    expect(formatTileChangeRatio(0.0004)).toBe("");
    expect(formatTileChangeRatio(null)).toBe("");
  });
});
