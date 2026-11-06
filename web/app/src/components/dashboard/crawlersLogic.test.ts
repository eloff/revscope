import { describe, expect, it } from "vitest";
import type { CrawlerRow } from "../../lib/crawlersApi";
import {
  buildOperatorLeaves,
  CRAWLER_CATEGORY_ORDER,
  CRAWLERS_TREEMAP_OTHER_KEY,
  CRAWLERS_TREEMAP_TOP_OPERATORS,
  categoryForRow,
  crawlerRowKey,
  filterRowsByCategories,
  isCrawlersTreemapSort,
  nextCrawlersSortState,
  operatorLabelForRow,
  operatorTrustTitle,
  soloCrawlerCategory,
  toggleDisabledCrawlerCategory,
} from "./crawlersLogic";

function row(overrides: Partial<CrawlerRow>): CrawlerRow {
  return { useragent_id: 1, source: 0, verified: true, views: 1, ...overrides } as CrawlerRow;
}

describe("crawler rows", () => {
  it("keys rows by user agent, source and verification", () => {
    expect(crawlerRowKey(row({ useragent_id: 7, source: 1, verified: false }))).toBe("7:1:0");
  });

  it("files unknown categories under headless", () => {
    expect(categoryForRow({ category: "ai_search" })).toBe("ai_search");
    expect(categoryForRow({ category: "spaceship" })).toBe("headless");
    expect(categoryForRow({})).toBe("headless");
  });

  it("explains how an operator was identified", () => {
    expect(operatorTrustTitle({ verified: false, source: 1 })).toBe("Unverified user agent");
    expect(operatorTrustTitle({ verified: true, source: 1 })).toBe("Reported by Cloudflare");
    expect(operatorTrustTitle({ verified: true, source: 0 })).toMatch(/published ranges/);
  });

  it.each([
    [{ verified: true, operator: "openai" }, "OpenAI"],
    [{ verified: true, operator: "anthropic" }, "Anthropic"],
    [{ verified: true }, "Unknown"],
    [{ verified: false, ip: " 1.2.3.4 " }, "1.2.3.4"],
    [{ verified: false, operator: "openai" }, "Unknown"],
  ])("labels %j as %s", (input, expected) => {
    expect(operatorLabelForRow(input)).toBe(expected);
  });
});

describe("crawler category toggles", () => {
  it("toggles a category without mutating the input", () => {
    const disabled = new Set<(typeof CRAWLER_CATEGORY_ORDER)[number]>(["archive"]);
    expect(toggleDisabledCrawlerCategory("archive", disabled)).toEqual(new Set());
    expect(toggleDisabledCrawlerCategory("headless", disabled)).toEqual(
      new Set(["archive", "headless"]),
    );
    expect(disabled).toEqual(new Set(["archive"]));
  });

  it("solos a category, and soloing it again re-enables everything", () => {
    const solo = soloCrawlerCategory("ai_search");
    expect(solo.size).toBe(CRAWLER_CATEGORY_ORDER.length - 1);
    expect(solo.has("ai_search")).toBe(false);
    expect(soloCrawlerCategory("ai_search", solo)).toEqual(new Set());
  });

  it("filters rows by enabled category", () => {
    const rows = [row({ category: "archive" }), row({ category: "ai_search" }), row({})];
    expect(filterRowsByCategories(rows, new Set())).toBe(rows);
    expect(filterRowsByCategories(rows, new Set(["headless", "archive"]))).toEqual([rows[1]]);
  });
});

describe("buildOperatorLeaves", () => {
  it("groups verified rows by operator and every unverified row together", () => {
    const leaves = buildOperatorLeaves([
      row({ operator: "openai", views: 5 }),
      row({ operator: "openai", views: 3 }),
      row({ verified: false, ip: "1.1.1.1", views: 1 }),
      row({ verified: false, ip: "2.2.2.2", views: 1 }),
    ]);
    expect(leaves.map((leaf) => [leaf.label, leaf.views, leaf.share])).toEqual([
      ["OpenAI", 8, 0.8],
      ["Unverified", 2, 0.2],
    ]);
  });

  it("rolls operators past the top 20 into Other", () => {
    const rows = Array.from({ length: CRAWLERS_TREEMAP_TOP_OPERATORS + 3 }, (_, index) =>
      row({ operator: `op${String(index).padStart(2, "0")}`, views: 100 - index }),
    );
    const leaves = buildOperatorLeaves(rows);
    const other = leaves.at(-1);
    expect(leaves).toHaveLength(CRAWLERS_TREEMAP_TOP_OPERATORS + 1);
    expect(other).toMatchObject({
      datumKey: CRAWLERS_TREEMAP_OTHER_KEY,
      isOther: true,
      otherCount: 3,
    });
    expect(other?.views).toBe(80 + 79 + 78 - 3 * 0);
  });

  it("breaks view ties by name and gives an operator a stable color", () => {
    const leaves = buildOperatorLeaves([row({ operator: "zeta" }), row({ operator: "alpha" })]);
    expect(leaves.map((leaf) => leaf.label)).toEqual(["Alpha", "Zeta"]);
    expect(buildOperatorLeaves([row({ operator: "zeta" })])[0]?.color).toBe(leaves[1]?.color);
  });

  it("shares nothing when there are no views", () => {
    expect(buildOperatorLeaves([row({ operator: "a", views: 0 })])[0]?.share).toBe(0);
  });
});

describe("crawler sorting", () => {
  it("starts views descending and other columns ascending", () => {
    expect(nextCrawlersSortState({ column: "views", direction: "desc" }, "views").direction).toBe(
      "asc",
    );
    expect(
      nextCrawlersSortState({ column: "views", direction: "desc" }, "operator" as never),
    ).toEqual({ column: "operator", direction: "asc" });
    expect(isCrawlersTreemapSort({ column: "views", direction: "desc" })).toBe(true);
  });
});
