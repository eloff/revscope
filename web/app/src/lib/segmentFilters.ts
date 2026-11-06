/**
 * Global segment filters applied across every analytics route alongside the
 * date range. Multiple values of the same dimension combine with OR; different
 * dimensions combine with AND (enforced server-side). A filter never narrows
 * its own breakdown view.
 *
 * Dimensions:
 * - country (ISO 3166-1 alpha-2, uppercase) — `country IN (...)`.
 * - source (revenue source: a campaign, a plain referrer, or the direct
 *   bucket) — restricts to sessions with a matching landing source.
 * - page (`paths.id`) — restricts to sessions containing a pageview on it.
 * - landing page (`paths.id`) — restricts to sessions that landed on it.
 * - event (`event_names.id`) — restricts to sessions containing that event.
 * - landing attributes (channel, region, city, browser, OS, device) — restrict
 *   to sessions whose landing has that value.
 */

export type CountryFilter = {
  /** ISO 3166-1 alpha-2 code, uppercase. */
  code: string;
  /** Display name for the chip (falls back to the code when unknown). */
  name: string;
};

/**
 * A revenue-source segment value. Mirrors the revenue-sources grid identity:
 * a campaign (`campaigns.id`), a plain referrer (`referrers.id`), or the direct
 * bucket. `name` is the human label shown on the filter chip.
 */
export type SourceFilter =
  | { kind: "campaign"; id: number; name: string }
  | { kind: "referrer"; id: number; name: string }
  | { kind: "direct"; name: string };

/** A page segment filter: a `paths.id` plus its path for the chip label. */
export type PageFilter = {
  /** Stable `paths.id`. */
  id: number;
  /** Display path for the chip. */
  name: string;
};

/**
 * A landing-page segment filter: a `paths.id` plus its path for the chip label.
 * Restricts to sessions whose landing pageview is on that path.
 */
export type LandingPageFilter = {
  /** Stable `paths.id`. */
  id: number;
  /** Display path for the chip. */
  name: string;
};

/**
 * An event segment filter: an `event_names.id` plus its name for the chip
 * label. Restricts to sessions that contain an event with that name.
 */
export type EventFilter = {
  /** Stable `event_names.id`. */
  id: number;
  /** Display event name for the chip. */
  name: string;
};

/**
 * A segment dimension read off the session's landing row. The name is the
 * backend query key and request-body `dimension`.
 */
export type LandingAttribute = "channel" | "region" | "city" | "browser" | "os" | "device";

export const LANDING_ATTRIBUTE_LABELS: Record<LandingAttribute, string> = {
  channel: "Channel",
  region: "Region",
  city: "City",
  browser: "Browser",
  os: "OS",
  device: "Device",
};

/**
 * A landing attribute filter. `value` is the backend's value for the
 * attribute, e.g. `search`, `US-NY`, `US-New York` or `macos`, exactly as the
 * overview breakdown rows report it.
 */
export type AttributeFilter = {
  attribute: LandingAttribute;
  value: string;
  /** Display text for the chip. */
  name: string;
};

export type SegmentFilters = {
  countries: CountryFilter[];
  sources: SourceFilter[];
  pages: PageFilter[];
  landingPages: LandingPageFilter[];
  events: EventFilter[];
  attributes: AttributeFilter[];
};

export const EMPTY_SEGMENT_FILTERS: SegmentFilters = {
  countries: [],
  sources: [],
  pages: [],
  landingPages: [],
  events: [],
  attributes: [],
};

/**
 * Serializes a source filter to the backend `source` query value. Uses a dash
 * delimiter (not a colon) so `URLSearchParams` never percent-encodes it, which
 * keeps backend query parsing decode-free.
 */
export function sourceFilterParam(source: SourceFilter): string {
  switch (source.kind) {
    case "campaign":
      return `campaign-${source.id}`;
    case "referrer":
      return `referrer-${source.id}`;
    case "direct":
      return "direct";
  }
}

/** Stable identity for a source filter, used for dedup and toggling. */
export function sourceFilterKey(source: SourceFilter): string {
  return sourceFilterParam(source);
}

/** Stable identity for a page filter, used for dedup and toggling. */
export function pageFilterKey(page: PageFilter): string {
  return String(page.id);
}

/** Stable identity for a landing-page filter, used for dedup and toggling. */
export function landingPageFilterKey(page: LandingPageFilter): string {
  return String(page.id);
}

/** Stable identity for an event filter, used for dedup and toggling. */
export function eventFilterKey(event: EventFilter): string {
  return String(event.id);
}

/** Stable identity for a landing attribute filter, used for dedup and toggling. */
export function attributeFilterKey(filter: Pick<AttributeFilter, "attribute" | "value">): string {
  return `${filter.attribute}:${filter.value}`;
}

/** Whether any dimension is narrowing the data currently on screen. */
export function hasSegmentFilters(segments: SegmentFilters): boolean {
  return (
    segments.countries.length > 0 ||
    segments.sources.length > 0 ||
    segments.pages.length > 0 ||
    segments.landingPages.length > 0 ||
    segments.events.length > 0 ||
    segments.attributes.length > 0
  );
}

/** Whether a value is a valid ISO 3166-1 alpha-2 country code (uppercased). */
export function isIsoCountryCode(code: string): boolean {
  return /^[A-Z]{2}$/.test(code.toUpperCase());
}

/** Appends a repeated query param per active filter, keyed by dimension. */
export function appendSegmentParams(url: URL, segments: SegmentFilters): void {
  for (const country of segments.countries) {
    url.searchParams.append("country", country.code);
  }
  for (const source of segments.sources) {
    url.searchParams.append("source", sourceFilterParam(source));
  }
  for (const page of segments.pages) {
    url.searchParams.append("page", String(page.id));
  }
  for (const landingPage of segments.landingPages) {
    url.searchParams.append("landing_page", String(landingPage.id));
  }
  for (const event of segments.events) {
    url.searchParams.append("event", String(event.id));
  }
  for (const filter of segments.attributes) {
    url.searchParams.append(filter.attribute, filter.value);
  }
}

/**
 * Stable serialization of the active segment filters, used to compare state for
 * cache invalidation and effects. Order-independent within each dimension.
 */
export function segmentFiltersKey(segments: SegmentFilters): string {
  const countries = segments.countries
    .map((country) => country.code)
    .slice()
    .sort()
    .join(",");
  const sources = segments.sources.map(sourceFilterKey).slice().sort().join(",");
  const pages = segments.pages.map(pageFilterKey).slice().sort().join(",");
  const landingPages = segments.landingPages.map(landingPageFilterKey).slice().sort().join(",");
  const events = segments.events.map(eventFilterKey).slice().sort().join(",");
  // JSON-quoted because city names may contain the delimiters.
  const attributes = JSON.stringify(segments.attributes.map(attributeFilterKey).slice().sort());
  return `${countries}|${sources}|${pages}|${landingPages}|${events}|${attributes}`;
}

/**
 * Returns a copy of `segments` with the given country added (deduped by code).
 * Invalid/non-ISO codes are ignored and the input is returned unchanged.
 */
export function withCountryFilter(
  segments: SegmentFilters,
  code: string,
  name: string,
): SegmentFilters {
  const normalized = code.toUpperCase();
  if (!isIsoCountryCode(normalized)) {
    return segments;
  }
  if (segments.countries.some((country) => country.code === normalized)) {
    return segments;
  }
  return {
    ...segments,
    countries: [...segments.countries, { code: normalized, name }],
  };
}

/** Returns a copy of `segments` with the given country removed. */
export function withoutCountryFilter(segments: SegmentFilters, code: string): SegmentFilters {
  const normalized = code.toUpperCase();
  return {
    ...segments,
    countries: segments.countries.filter((country) => country.code !== normalized),
  };
}

/** Returns a copy of `segments` with the given source added (deduped by key). */
export function withSourceFilter(segments: SegmentFilters, source: SourceFilter): SegmentFilters {
  const key = sourceFilterKey(source);
  if (segments.sources.some((existing) => sourceFilterKey(existing) === key)) {
    return segments;
  }
  return {
    ...segments,
    sources: [...segments.sources, source],
  };
}

/** Returns a copy of `segments` with the source matching `key` removed. */
export function withoutSourceFilter(segments: SegmentFilters, key: string): SegmentFilters {
  return {
    ...segments,
    sources: segments.sources.filter((source) => sourceFilterKey(source) !== key),
  };
}

/** Returns a copy of `segments` with the given page added (deduped by id). */
export function withPageFilter(segments: SegmentFilters, page: PageFilter): SegmentFilters {
  if (segments.pages.some((existing) => existing.id === page.id)) {
    return segments;
  }
  return {
    ...segments,
    pages: [...segments.pages, page],
  };
}

/** Returns a copy of `segments` with the page matching `key` removed. */
export function withoutPageFilter(segments: SegmentFilters, key: string): SegmentFilters {
  return {
    ...segments,
    pages: segments.pages.filter((page) => pageFilterKey(page) !== key),
  };
}

/**
 * Returns a copy of `segments` with the given landing page added (deduped by
 * id).
 */
export function withLandingPageFilter(
  segments: SegmentFilters,
  landingPage: LandingPageFilter,
): SegmentFilters {
  if (segments.landingPages.some((existing) => existing.id === landingPage.id)) {
    return segments;
  }
  return {
    ...segments,
    landingPages: [...segments.landingPages, landingPage],
  };
}

/** Returns a copy of `segments` with the landing page matching `key` removed. */
export function withoutLandingPageFilter(segments: SegmentFilters, key: string): SegmentFilters {
  return {
    ...segments,
    landingPages: segments.landingPages.filter((page) => landingPageFilterKey(page) !== key),
  };
}

/** Returns a copy of `segments` with the given event added (deduped by id). */
export function withEventFilter(segments: SegmentFilters, event: EventFilter): SegmentFilters {
  if (segments.events.some((existing) => existing.id === event.id)) {
    return segments;
  }
  return {
    ...segments,
    events: [...segments.events, event],
  };
}

/** Returns a copy of `segments` with the event matching `key` removed. */
export function withoutEventFilter(segments: SegmentFilters, key: string): SegmentFilters {
  return {
    ...segments,
    events: segments.events.filter((event) => eventFilterKey(event) !== key),
  };
}

/** Returns a copy of `segments` with the given landing attribute added (deduped). */
export function withAttributeFilter(
  segments: SegmentFilters,
  filter: AttributeFilter,
): SegmentFilters {
  const key = attributeFilterKey(filter);
  if (segments.attributes.some((existing) => attributeFilterKey(existing) === key)) {
    return segments;
  }
  return {
    ...segments,
    attributes: [...segments.attributes, filter],
  };
}

/** Returns a copy of `segments` with the landing attribute matching `key` removed. */
export function withoutAttributeFilter(segments: SegmentFilters, key: string): SegmentFilters {
  return {
    ...segments,
    attributes: segments.attributes.filter((filter) => attributeFilterKey(filter) !== key),
  };
}
