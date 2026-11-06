import {
  type CacheEntry,
  readFreshCacheEntry,
  writeCacheEntry,
} from "../../lib/dashboardDataCache";
import type { FunnelDataResponse } from "../../lib/funnelsApi";

/**
 * Funnel data keyed by funnel id alone. The generation is bumped on every
 * clear, so a response sent before a range or filter change isn't cached as
 * the new range's data.
 */
export function createFunnelDataCache(contextKey: () => string) {
  const entries = new Map<number, CacheEntry<FunnelDataResponse>>();
  let generation = 0;
  let context = "";

  const clear = () => {
    generation += 1;
    entries.clear();
  };

  return {
    clear,
    /**
     * Drops the cache if the range or filters it was filled under changed, and
     * returns the generation a request starting now belongs to. Called by each
     * load rather than from an effect, which could run after the load effect
     * and leave it a stale hit.
     */
    sync: (): number => {
      const current = contextKey();
      if (current !== context) {
        context = current;
        clear();
      }
      return generation;
    },
    read: (funnelId: number) => readFreshCacheEntry(entries, funnelId),
    /** Caches a response unless a clear happened since its request began. */
    write: (requestGeneration: number, data: FunnelDataResponse) => {
      if (requestGeneration === generation) {
        writeCacheEntry(entries, data.funnel_id, data);
      }
    },
    /**
     * Forgets one funnel's data and makes every in-flight response stale, for
     * when that funnel's steps change underneath them. Returns the new generation.
     */
    invalidate: (funnelId: number): number => {
      generation += 1;
      entries.delete(funnelId);
      return generation;
    },
  };
}
