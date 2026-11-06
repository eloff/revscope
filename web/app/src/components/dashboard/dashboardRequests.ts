import {
  type CacheEntry,
  readFreshCacheEntry,
  writeCacheEntry,
} from "../../lib/dashboardDataCache";
import type { DateRangeSeconds } from "../../lib/dateRange";

export type InFlightRunner = (key: string, run: () => Promise<void>) => Promise<void>;

/**
 * Reuse one in-flight promise per key so effect re-runs don't double-fetch.
 * Guard on this rather than on a loading signal: loading flips false on error
 * while the cache stays empty, so guarding on it from an effect can retry-loop.
 */
export function createInFlightRunner(): InFlightRunner {
  const current = new Map<string, Promise<void>>();
  return (key, run) => {
    const existing = current.get(key);
    if (existing) {
      return existing;
    }
    const promise = run().finally(() => {
      if (current.get(key) === promise) {
        current.delete(key);
      }
    });
    current.set(key, promise);
    return promise;
  };
}

/** Thrown from a fetch whose caches were cleared mid-request, so it isn't cached. */
export const STALE_RESPONSE = new Error("stale dashboard response");

export function dateRangeInFlightKey(range: DateRangeSeconds): string {
  return `${range.start}:${range.end}`;
}

export function errorMessage(error: unknown, fallback: string): string {
  return error instanceof Error ? error.message : fallback;
}

export type SlotRequest = {
  /** No cache clear has happened since the request began. */
  inGeneration: () => boolean;
  isCurrent: () => boolean;
  claimLoading: () => void;
  /** True once, for the request that owns the slot's loading flag. */
  releaseLoading: () => boolean;
};

/**
 * A keyed response cache whose generation advances on every clear. Cache keys
 * never encode the date range or segment filters; the dashboard clears its
 * caches whenever those change, and a response that began before the clear
 * must not be cached afterwards.
 */
export type GenerationalCache<T> = ReturnType<typeof createGenerationalCache<T>>;

export function createGenerationalCache<T>() {
  const entries = new Map<string, CacheEntry<T>>();
  let generation = 0;
  return {
    entries,
    runInFlight: createInFlightRunner(),
    generation: () => generation,
    clear: () => {
      generation += 1;
      entries.clear();
    },
    getOrFetch: async (key: string, fetch: () => Promise<T>): Promise<T> => {
      const cached = readFreshCacheEntry(entries, key);
      if (cached) {
        return cached;
      }
      const startedIn = generation;
      const data = await fetch();
      if (startedIn === generation) {
        writeCacheEntry(entries, key, data);
      }
      return data;
    },
  };
}

/**
 * A response is only shown while it is the newest request for its slot, and
 * only cached while no range or filter change has cleared its cache since it
 * was sent. Otherwise a slow answer for the old range lands last and wins.
 */
export type RequestTracker = ReturnType<typeof createRequestTracker>;

export function createRequestTracker() {
  let requestCounter = 0;
  const latestRequestBySlot = new Map<string, number>();
  // The request that turned a slot's loading flag on clears it, even once a
  // newer request has taken over the slot: a background refresh that takes
  // over never touches the flag, so nobody else would.
  const loadingOwnerBySlot = new Map<string, number>();

  const begin = (slot: string, generation: () => number): SlotRequest => {
    const id = ++requestCounter;
    latestRequestBySlot.set(slot, id);
    const startedIn = generation();
    const inGeneration = () => generation() === startedIn;
    return {
      inGeneration,
      isCurrent: () => inGeneration() && latestRequestBySlot.get(slot) === id,
      claimLoading: () => {
        loadingOwnerBySlot.set(slot, id);
      },
      releaseLoading: () => {
        if (loadingOwnerBySlot.get(slot) !== id) {
          return false;
        }
        loadingOwnerBySlot.delete(slot);
        return true;
      },
    };
  };

  return {
    begin,
    /**
     * Shows a slot's page from cache, superseding any request still in flight
     * for it. Call it on every fresh cache hit, even when that page is already
     * on screen: a request for a page the viewer has since left must not land.
     */
    settleFromCache: (slot: string) => {
      begin(slot, () => 0);
      loadingOwnerBySlot.delete(slot);
    },
  };
}
