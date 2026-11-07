import { describe, expect, it } from "vitest";
import type { CountryRow, RowMetrics } from "../../lib/audienceApi";
import {
  AUDIENCE_BREAKDOWN_TREEMAP_TOP,
  type AudienceBreakdownRow,
  browserFamilyIconClass,
  bucketByRevenue,
  buildAudienceBreakdownTreemapLeaves,
  buildBrowserBreakdownRows,
  buildCityBreakdownRows,
  buildCityHoverContent,
  buildCountryBreakdownRows,
  buildCountryHoverContent,
  buildCountryMetrics,
  buildScreenSizeMetrics,
  countryDisplayName,
  deviceIconClass,
  findAudienceBreakdownHighlights,
  osIconClass,
  REVENUE_BUCKET_COLORS,
  sortAudienceBreakdownRows,
  widthBucketLabel,
  withAudienceBreakdownHighlightDescription,
} from "./audienceLogic";
import { nextColumnSort } from "./columnSort";

function metrics(visitors: number, revenue: number, converting = 0): RowMetrics {
  return {
    visitors,
    converting_visitors: converting,
    revenue,
    revenue_per_visitor: visitors > 0 ? revenue / visitors : 0,
    revenue_change: { ratio: 0.1 },
    visitors_change: { ratio: null },
    conversions_change: { ratio: -0.2 },
  } as RowMetrics;
}

function country(code: string | undefined, visitors: number, revenue: number): CountryRow {
  return {
    country: code,
    country_name: "",
    conversion_rate: 0.05,
    conversion_lower_bound: 0.01,
    conversion_ci: { lower: 0.01, upper: 0.09 },
    ...metrics(visitors, revenue, 1),
  };
}

function breakdown(key: string, visitors: number, revenue: number, lowerBound?: number) {
  return {
    key,
    name: key,
    visitors,
    convertingVisitors: lowerBound ? 1 : 0,
    revenue,
    revenuePerVisitor: 0,
    conversionLowerBound: lowerBound,
  } satisfies AudienceBreakdownRow;
}

describe("country names and labels", () => {
  it("resolves region names, falling back to the code", () => {
    expect(countryDisplayName("DE")).toBe("Germany");
    expect(countryDisplayName("not-a-code")).toBe("not-a-code");
  });

  it("labels width buckets", () => {
    expect(widthBucketLabel("small_mobile")).toBe("Mobile < 360");
    expect(widthBucketLabel("x_large_desktop")).toBe("Desktop ≥ 1920");
    expect(widthBucketLabel("watch")).toBe("watch");
  });

  it("falls back to the unknown icon", () => {
    expect(browserFamilyIconClass("netscape")).toBe(browserFamilyIconClass("unknown"));
    expect(browserFamilyIconClass("chrome")).not.toBe(browserFamilyIconClass("unknown"));
    expect(osIconClass("chromeos")).toBe(browserFamilyIconClass("chrome"));
    expect(osIconClass("beos")).toBe(osIconClass("unknown"));
    expect(deviceIconClass("fridge")).toBe(osIconClass("unknown"));
  });
});

describe("buildCountryMetrics", () => {
  it("keys placeable countries by code with revenue share over every row", () => {
    const map = buildCountryMetrics({
      current: [country("US", 10, 600), country("DE", 5, 200), country(undefined, 3, 200)],
    });
    expect([...map.keys()]).toEqual(["US", "DE"]);
    expect(map.get("US")).toMatchObject({ name: "United States", revenueShare: 0.6 });
    expect(map.get("DE")?.revenueShare).toBe(0.2);
  });

  it("shares nothing when there is no revenue", () => {
    expect(buildCountryMetrics({ current: [country("US", 1, 0)] }).get("US")?.revenueShare).toBe(0);
  });
});

describe("breakdown rows", () => {
  it("names countries, with Unknown for missing codes", () => {
    const rows = buildCountryBreakdownRows({
      current: [{ ...country("FR", 1, 1), country_name: "France (FR)" }, country(undefined, 1, 1)],
    });
    expect(rows.map((row) => [row.key, row.name])).toEqual([
      ["FR", "France (FR)"],
      ["unknown", "Unknown"],
    ]);
    expect(rows[0]?.conversionLowerBound).toBe(0.01);
  });

  it("names cities with their country and flags them", () => {
    const rows = buildCityBreakdownRows({
      current: [
        { city: "Paris", country: "FR", ...metrics(1, 1) },
        { city: "Atlantis", ...metrics(1, 1) },
      ],
      offset: 0,
      limit: 200,
    });
    expect(rows.map((row) => [row.key, row.name, row.flagCountryCode])).toEqual([
      ["FR:Paris", "Paris, France", "FR"],
      ["unknown:Atlantis", "Atlantis", undefined],
    ]);
  });

  it("falls back to the slug when the API sends no display name", () => {
    expect(
      buildBrowserBreakdownRows({
        current: [{ browser_family: "arc", browser_family_name: "", ...metrics(1, 1) }],
      })[0]?.name,
    ).toBe("arc");
    expect(
      buildScreenSizeMetrics({ current: [{ screen_size: "tablet", ...metrics(1, 1) }] }).get(
        "tablet",
      )?.name,
    ).toBe("Tablet [768, 1024)");
  });
});

describe("sorting and highlights", () => {
  const rows = [
    breakdown("b", 10, 100, 0.2),
    breakdown("a", 10, 300, 0.1),
    breakdown("c", 20, 100),
  ];

  it("sorts by the column, then visitors, then name", () => {
    expect(
      sortAudienceBreakdownRows(rows, { column: "revenue", direction: "desc" }).map((r) => r.key),
    ).toEqual(["a", "c", "b"]);
    expect(
      sortAudienceBreakdownRows(rows, { column: "name", direction: "asc" }).map((r) => r.key),
    ).toEqual(["a", "b", "c"]);
  });

  it("breaks visitor ties by revenue", () => {
    expect(
      sortAudienceBreakdownRows(rows, { column: "visitors", direction: "asc" }).map((r) => r.key),
    ).toEqual(["a", "b", "c"]);
  });

  it("treats a missing conversion bound as zero", () => {
    expect(
      sortAudienceBreakdownRows(rows, { column: "conversion", direction: "desc" }).map(
        (r) => r.key,
      ),
    ).toEqual(["b", "a", "c"]);
  });

  it("flips the active column and starts others descending", () => {
    expect(nextColumnSort({ column: "revenue", direction: "desc" }, "revenue")).toEqual({
      column: "revenue",
      direction: "asc",
    });
    expect(nextColumnSort({ column: "revenue", direction: "asc" }, "name").direction).toBe("desc");
  });

  it("finds the best revenue and conversion rows", () => {
    expect(findAudienceBreakdownHighlights(rows)).toEqual({ revenue: "a", conversion: "b" });
    expect(findAudienceBreakdownHighlights([])).toEqual({});
  });

  it("stars nothing on zero revenue or zero conversions", () => {
    const unconverted = [
      { ...rows[0], revenue: 0, convertingVisitors: 0, conversionLowerBound: 0 },
      { ...rows[1], revenue: 0, convertingVisitors: 0, conversionLowerBound: 0 },
    ];
    expect(findAudienceBreakdownHighlights(unconverted)).toEqual({});
  });

  it("stars no rate whose bound rounds above zero without conversions", () => {
    // The server's Wilson bound for 0 of 3 visitors comes out as 5.55e-17, not 0.
    const unconverted = [{ ...rows[0], convertingVisitors: 0, conversionLowerBound: 5.55e-17 }];
    expect(findAudienceBreakdownHighlights(unconverted, ["conversion"])).toEqual({});
  });

  it("adds highlight lines only to highlighted rows", () => {
    const content = { stats: [] };
    const highlights = { revenue: "a", conversion: "a" };
    expect(withAudienceBreakdownHighlightDescription(content, "a", highlights)).toEqual({
      stats: [],
      highlightDescriptions: [{ metric: "revenue" }, { metric: "conversion" }],
    });
    expect(withAudienceBreakdownHighlightDescription(content, "b", highlights)).toBe(content);
    expect(withAudienceBreakdownHighlightDescription(content, "a", undefined)).toBe(content);
  });
});

describe("buildAudienceBreakdownTreemapLeaves", () => {
  it("drops zero-revenue rows and ranks by revenue", () => {
    const leaves = buildAudienceBreakdownTreemapLeaves(
      [breakdown("x", 5, 0), breakdown("y", 1, 50), breakdown("z", 1, 90)],
      "browsers",
    );
    expect(leaves.map((leaf) => leaf.datumKey)).toEqual(["z", "y"]);
  });

  it("rolls the long tail into Other", () => {
    const rows = Array.from({ length: AUDIENCE_BREAKDOWN_TREEMAP_TOP + 2 }, (_, index) =>
      breakdown(`r${index}`, 1, 1000 - index),
    );
    const other = buildAudienceBreakdownTreemapLeaves(rows, "cities").at(-1);
    expect(other).toMatchObject({
      isOther: true,
      revenue: 1000 - 50 + 1000 - 51,
      visitors: 2,
      tooltip: "Revenue from 2 cities outside the top 50",
    });
  });
});

describe("bucketByRevenue", () => {
  it("splits countries into five buckets, darkest for the most revenue", () => {
    const map = buildCountryMetrics({
      current: ["A", "B", "C", "D", "E", "F", "G", "H", "I", "J"].map((code, index) =>
        country(code, 1, 100 - index),
      ),
    });
    const buckets = bucketByRevenue(map);
    expect(REVENUE_BUCKET_COLORS).toHaveLength(5);
    expect(buckets.get("A")).toBe(5);
    expect(buckets.get("B")).toBe(5);
    expect(buckets.get("C")).toBe(4);
    expect(buckets.get("J")).toBe(1);
  });

  it("puts a lone country in the top bucket", () => {
    expect(bucketByRevenue(buildCountryMetrics({ current: [country("US", 1, 1)] })).get("US")).toBe(
      5,
    );
    expect(bucketByRevenue(new Map()).size).toBe(0);
  });
});

describe("hover content", () => {
  it("adds conversion rate and interval for countries only", () => {
    const us = buildCountryMetrics({ current: [country("US", 4, 8000)] }).get("US");
    if (!us) throw new Error("missing US");
    const labels = buildCountryHoverContent(us).stats.map((stat) => stat.label);
    expect(labels).toEqual([
      "Revenue",
      "Visitors",
      "Conversions",
      "Revenue / visitor",
      "Conversion rate",
      "95% CI",
    ]);
    expect(buildCountryHoverContent(us).stats[0]).toMatchObject({ value: "$8", changeRatio: 0.1 });
    expect(buildCityHoverContent({ ...us, key: "k", city: "x" }).stats).toHaveLength(4);
  });
});
