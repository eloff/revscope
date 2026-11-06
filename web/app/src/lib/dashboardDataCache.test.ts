import { describe, expect, it, vi } from "vitest";
import {
  type CacheEntry,
  DASHBOARD_DATA_STALE_MS,
  ensureCachedPage,
  isCacheEntryStale,
  rangesEqual,
  readFreshCacheEntry,
  writeCacheEntry,
} from "./dashboardDataCache";

describe("dashboard data cache", () => {
  it("treats entries as stale from the five-minute mark", () => {
    expect(isCacheEntryStale(0, DASHBOARD_DATA_STALE_MS - 1)).toBe(false);
    expect(isCacheEntryStale(0, DASHBOARD_DATA_STALE_MS)).toBe(true);
  });

  it("reads fresh entries and misses stale ones", () => {
    const cache = new Map<string, CacheEntry<number>>();
    writeCacheEntry(cache, "a", 1, 1000);
    expect(readFreshCacheEntry(cache, "a", 1000 + DASHBOARD_DATA_STALE_MS - 1)).toBe(1);
    expect(readFreshCacheEntry(cache, "a", 1000 + DASHBOARD_DATA_STALE_MS)).toBeUndefined();
    expect(readFreshCacheEntry(cache, "missing")).toBeUndefined();
  });

  it("fetches a missing page once and serves it from the cache after", async () => {
    const cache = new Map<string, CacheEntry<string>>();
    const runInFlight = (_key: string, run: () => Promise<void>) => run();
    const fetchPage = vi.fn(async () => "page");
    await expect(ensureCachedPage(cache, "k", "k", runInFlight, fetchPage)).resolves.toBe("page");
    await expect(ensureCachedPage(cache, "k", "k", runInFlight, fetchPage)).resolves.toBe("page");
    expect(fetchPage).toHaveBeenCalledTimes(1);
  });

  it("refetches a stale page", async () => {
    const cache = new Map<string, CacheEntry<string>>();
    writeCacheEntry(cache, "k", "old", Date.now() - DASHBOARD_DATA_STALE_MS);
    const runInFlight = (_key: string, run: () => Promise<void>) => run();
    await expect(ensureCachedPage(cache, "k", "k", runInFlight, async () => "new")).resolves.toBe(
      "new",
    );
  });

  it("compares ranges by value", () => {
    expect(rangesEqual({ start: 1, end: 2 }, { start: 1, end: 2 })).toBe(true);
    expect(rangesEqual({ start: 1, end: 2 }, { start: 1, end: 3 })).toBe(false);
  });
});
