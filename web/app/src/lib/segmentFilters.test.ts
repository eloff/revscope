import { describe, expect, it } from "vitest";
import {
  appendSegmentParams,
  EMPTY_SEGMENT_FILTERS,
  hasSegmentFilters,
  type SegmentFilters,
  segmentFiltersKey,
  sourceFilterParam,
  withAttributeFilter,
  withCountryFilter,
  withEventFilter,
  withLandingPageFilter,
  withoutAttributeFilter,
  withoutCountryFilter,
  withoutSourceFilter,
  withPageFilter,
  withSourceFilter,
} from "./segmentFilters";

describe("country filters", () => {
  it("uppercases, dedupes, and ignores codes that are not ISO alpha-2", () => {
    let segments = withCountryFilter(EMPTY_SEGMENT_FILTERS, "us", "United States");
    expect(segments.countries).toEqual([{ code: "US", name: "United States" }]);
    expect(withCountryFilter(segments, "US", "USA")).toBe(segments);
    expect(withCountryFilter(segments, "USA", "USA")).toBe(segments);
    expect(withCountryFilter(segments, "1A", "?")).toBe(segments);

    segments = withoutCountryFilter(segments, "us");
    expect(segments.countries).toEqual([]);
  });

  it("does not mutate the filters it was given", () => {
    const before = structuredClone(EMPTY_SEGMENT_FILTERS);
    withCountryFilter(EMPTY_SEGMENT_FILTERS, "DE", "Germany");
    expect(EMPTY_SEGMENT_FILTERS).toEqual(before);
  });
});

describe("source filters", () => {
  it("serializes to the backend source param without characters URLs escape", () => {
    expect(sourceFilterParam({ kind: "campaign", id: 7, name: "Launch" })).toBe("campaign-7");
    expect(sourceFilterParam({ kind: "referrer", id: 3, name: "x.com" })).toBe("referrer-3");
    expect(sourceFilterParam({ kind: "direct", name: "Direct" })).toBe("direct");
  });

  it("dedupes by identity, not by label", () => {
    const segments = withSourceFilter(EMPTY_SEGMENT_FILTERS, {
      kind: "referrer",
      id: 3,
      name: "x.com",
    });
    expect(withSourceFilter(segments, { kind: "referrer", id: 3, name: "renamed" })).toBe(segments);
    expect(
      withSourceFilter(segments, { kind: "campaign", id: 3, name: "x.com" }).sources,
    ).toHaveLength(2);
    expect(withoutSourceFilter(segments, "referrer-3").sources).toEqual([]);
  });
});

describe("serialization", () => {
  const segments: SegmentFilters = {
    countries: [
      { code: "US", name: "United States" },
      { code: "DE", name: "Germany" },
    ],
    sources: [{ kind: "direct", name: "Direct" }],
    pages: [{ id: 12, name: "/pricing" }],
    landingPages: [{ id: 4, name: "/" }],
    events: [{ id: 9, name: "signup" }],
    attributes: [
      { attribute: "city", value: "US-New York", name: "New York" },
      { attribute: "os", value: "macos", name: "macOS" },
    ],
  };

  it("appends one query param per active value", () => {
    const url = new URL("https://api.example.test/v2/read/projects/1/overview?start=1");
    appendSegmentParams(url, segments);
    expect(url.searchParams.getAll("country")).toEqual(["US", "DE"]);
    expect(url.searchParams.getAll("source")).toEqual(["direct"]);
    expect(url.searchParams.getAll("page")).toEqual(["12"]);
    expect(url.searchParams.getAll("landing_page")).toEqual(["4"]);
    expect(url.searchParams.getAll("event")).toEqual(["9"]);
    expect(url.searchParams.getAll("city")).toEqual(["US-New York"]);
    expect(url.searchParams.getAll("os")).toEqual(["macos"]);
    expect(url.searchParams.get("start")).toBe("1");
  });

  it("keys the same selection the same way regardless of order", () => {
    const reordered = { ...segments, countries: [...segments.countries].reverse() };
    expect(segmentFiltersKey(reordered)).toBe(segmentFiltersKey(segments));
    const reorderedAttributes = { ...segments, attributes: [...segments.attributes].reverse() };
    expect(segmentFiltersKey(reorderedAttributes)).toBe(segmentFiltersKey(segments));
    expect(segmentFiltersKey(EMPTY_SEGMENT_FILTERS)).toBe("|||||[]");
  });

  it("tells pages and landing pages apart", () => {
    const page = withPageFilter(EMPTY_SEGMENT_FILTERS, { id: 4, name: "/" });
    const landing = withLandingPageFilter(EMPTY_SEGMENT_FILTERS, { id: 4, name: "/" });
    expect(segmentFiltersKey(page)).not.toBe(segmentFiltersKey(landing));
  });

  it("reports whether anything narrows the data", () => {
    expect(hasSegmentFilters(EMPTY_SEGMENT_FILTERS)).toBe(false);
    expect(hasSegmentFilters(withEventFilter(EMPTY_SEGMENT_FILTERS, { id: 1, name: "x" }))).toBe(
      true,
    );
  });
});

describe("landing attribute filters", () => {
  it("dedupes by attribute and value, not by label", () => {
    const segments = withAttributeFilter(EMPTY_SEGMENT_FILTERS, {
      attribute: "region",
      value: "US-NY",
      name: "NY",
    });
    expect(
      withAttributeFilter(segments, { attribute: "region", value: "US-NY", name: "New York" }),
    ).toBe(segments);
    expect(
      withAttributeFilter(segments, { attribute: "city", value: "US-NY", name: "NY" }).attributes,
    ).toHaveLength(2);
    expect(hasSegmentFilters(segments)).toBe(true);
    expect(withoutAttributeFilter(segments, "region:US-NY").attributes).toEqual([]);
  });

  it("keys values containing the key delimiters unambiguously", () => {
    const one = withAttributeFilter(EMPTY_SEGMENT_FILTERS, {
      attribute: "city",
      value: "US-A,os:b",
      name: "A",
    });
    const two = withAttributeFilter(
      withAttributeFilter(EMPTY_SEGMENT_FILTERS, { attribute: "city", value: "US-A", name: "A" }),
      { attribute: "os", value: "b", name: "b" },
    );
    expect(segmentFiltersKey(one)).not.toBe(segmentFiltersKey(two));
  });
});
