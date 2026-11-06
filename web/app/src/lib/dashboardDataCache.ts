import type { DateRangeSeconds } from "./dateRange";

/** Refetch dashboard data in the background after this age. */
export const DASHBOARD_DATA_STALE_MS = 5 * 60 * 1000;

/**
 * A cached value plus the time it was fetched. Cache keys never encode the
 * selected date range: the whole cache is cleared whenever the range changes
 * (see callers), so a present entry always belongs to the current range.
 */
export type CacheEntry<T> = {
  data: T;
  fetchedAt: number;
};

export function isCacheEntryStale(fetchedAt: number, now = Date.now()): boolean {
  return now - fetchedAt >= DASHBOARD_DATA_STALE_MS;
}

/**
 * Read a cache entry, treating entries older than {@link DASHBOARD_DATA_STALE_MS}
 * as a miss so callers refetch.
 */
export function readFreshCacheEntry<K, T>(
  cache: Map<K, CacheEntry<T>>,
  key: K,
  now = Date.now(),
): T | undefined {
  const entry = cache.get(key);
  if (!entry || isCacheEntryStale(entry.fetchedAt, now)) {
    return undefined;
  }
  return entry.data;
}

export function writeCacheEntry<K, T>(
  cache: Map<K, CacheEntry<T>>,
  key: K,
  data: T,
  now = Date.now(),
): void {
  cache.set(key, { data, fetchedAt: now });
}

/**
 * Fetch `key` into `cache` if it is missing or stale. Concurrent callers with
 * the same in-flight key share one request; they must read the cache after
 * awaiting rather than relying on this helper to update UI signals.
 */
export async function ensureCachedPage<T>(
  cache: Map<string, CacheEntry<T>>,
  cacheKey: string,
  inFlightKey: string,
  runInFlight: (key: string, run: () => Promise<void>) => Promise<void>,
  fetchPage: () => Promise<T>,
): Promise<T | undefined> {
  await runInFlight(inFlightKey, async () => {
    if (readFreshCacheEntry(cache, cacheKey)) {
      return;
    }
    writeCacheEntry(cache, cacheKey, await fetchPage());
  });
  return cache.get(cacheKey)?.data;
}

export function rangesEqual(a: DateRangeSeconds, b: DateRangeSeconds): boolean {
  return a.start === b.start && a.end === b.end;
}
