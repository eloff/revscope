import { type Accessor, createMemo, createSignal } from "solid-js";
import { type CacheEntry, ensureCachedPage, isCacheEntryStale } from "../../lib/dashboardDataCache";
import type { DateRangeSeconds } from "../../lib/dateRange";
import {
  createGenerationalCache,
  createInFlightRunner,
  dateRangeInFlightKey,
  errorMessage,
  type GenerationalCache,
  type RequestTracker,
  STALE_RESPONSE,
} from "./dashboardRequests";

/** One page of a tab's data: the cache key it lives under and how to fetch it. */
export type PageQuery<T> = {
  key: string;
  fetch: (range: DateRangeSeconds) => Promise<T>;
};

/**
 * What a tab hands its view: the page on screen and its request state.
 * `settledEmpty` is true once a fetch settled on a payload with nothing to render.
 */
export type LoadState<T> = {
  data: Accessor<T | undefined>;
  loading: Accessor<boolean>;
  error: Accessor<string>;
  settledEmpty: (hasData: (data: T) => boolean) => boolean;
};

function settledEmpty<T>(state: Omit<LoadState<T>, "settledEmpty">, hasData: (data: T) => boolean) {
  const data = state.data();
  return !state.loading() && !state.error() && data !== undefined && !hasData(data);
}

export type PageSlot<T> = LoadState<T> & {
  /** Cache key of the page on screen; null while nothing is shown. */
  loadedKey: Accessor<string | null>;
  clearPage: () => void;
  /** Shows the query's page from cache, or (re)fetches it when missing or stale. */
  sync: (range: DateRangeSeconds) => void;
};

/**
 * A signal showing one page from a shared cache. Several slots may read one
 * cache (a table and its treemap), each tracked as its own request slot so
 * only its newest request lands.
 */
export function createPageSlot<T>(options: {
  requests: RequestTracker;
  slot: string;
  cache: GenerationalCache<T>;
  query: () => PageQuery<T>;
  errorFallback: string;
  /** Blank the page while a new one loads, rather than keep the old one up. */
  clearOnFetch: boolean;
}): PageSlot<T> {
  const { requests, slot, cache } = options;
  const [data, setData] = createSignal<T | undefined>();
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal("");
  const [loadedKey, setLoadedKey] = createSignal<string | null>(null);

  const show = (key: string, page: T) => {
    setData(() => page);
    setLoadedKey(key);
  };
  const clearPage = () => {
    setData(undefined);
    setLoadedKey(null);
  };
  const showFromCache = (key: string, page: T) => {
    requests.settleFromCache(slot);
    if (data() !== page) {
      show(key, page);
    } else {
      setLoadedKey(key);
    }
    setError("");
    setLoading(false);
  };

  const load = async (range: DateRangeSeconds, query: PageQuery<T>, background: boolean) => {
    const request = requests.begin(slot, cache.generation);
    const cached = cache.entries.get(query.key);
    if (cached && !isCacheEntryStale(cached.fetchedAt)) {
      showFromCache(query.key, cached.data);
      return;
    }
    const keepExisting = background && data() !== undefined;
    if (!keepExisting) {
      request.claimLoading();
      setLoading(true);
      setError("");
      if (options.clearOnFetch) {
        clearPage();
      }
    }
    try {
      const page = await ensureCachedPage(
        cache.entries,
        query.key,
        `${cache.generation()}:${dateRangeInFlightKey(range)}:${query.key}`,
        cache.runInFlight,
        async () => {
          const fetched = await query.fetch(range);
          if (!request.inGeneration()) {
            // Thrown so ensureCachedPage doesn't cache the old range's page.
            throw STALE_RESPONSE;
          }
          return fetched;
        },
      );
      if (page && request.isCurrent()) {
        show(query.key, page);
      }
    } catch (loadError) {
      if (loadError === STALE_RESPONSE || !request.isCurrent()) {
        return;
      }
      if (!keepExisting) {
        if (options.clearOnFetch) {
          clearPage();
        }
        setError(errorMessage(loadError, options.errorFallback));
      }
    } finally {
      if (request.releaseLoading()) {
        setLoading(false);
      }
    }
  };

  const state = { data, loading, error };
  return {
    ...state,
    loadedKey,
    settledEmpty: (hasData) => settledEmpty(state, hasData),
    clearPage,
    sync: (range) => {
      const query = options.query();
      const entry = cache.entries.get(query.key);
      if (!entry) {
        void load(range, query, false);
      } else if (isCacheEntryStale(entry.fetchedAt)) {
        void load(range, query, true);
      } else {
        showFromCache(query.key, entry.data);
      }
    },
  };
}

export type OffsetTabData<T> = {
  table: PageSlot<T>;
  /** Shared by the table and treemap, and by detail views over the same rows. */
  cache: GenerationalCache<T>;
  offset: Accessor<number>;
  /** Cache key of the page the table should be showing. */
  currentKey: () => string;
  changeOffset: (offset: number) => void;
  sync: (range: DateRangeSeconds) => void;
  /** Back to the first page with the cache dropped, for a new range or filter. */
  reset: () => void;
};

type OffsetTabOptions<T, S> = {
  requests: RequestTracker;
  name: string;
  errorFallback: string;
  sort: () => S;
  cacheKey: (offset: number, sort: S) => string;
  fetch: (range: DateRangeSeconds, offset: number, sort: S) => Promise<T>;
  /** Blank the table while a new page loads; defaults to true. */
  clearOnFetch?: boolean;
};

/**
 * Data for a tab with an offset-paged, sortable table and optionally a
 * treemap of the first page in `treemapSort`.
 */
export function createOffsetTabData<T, S>(
  options: OffsetTabOptions<T, S> & { treemapSort: S },
): OffsetTabData<T> & { treemap: PageSlot<T> };
export function createOffsetTabData<T, S>(options: OffsetTabOptions<T, S>): OffsetTabData<T>;
export function createOffsetTabData<T, S>(
  options: OffsetTabOptions<T, S> & { treemapSort?: S },
): OffsetTabData<T> & { treemap?: PageSlot<T> } {
  const { requests, name, errorFallback } = options;
  const clearOnFetch = options.clearOnFetch ?? true;
  const cache = createGenerationalCache<T>();
  const [offset, setOffset] = createSignal(0);
  const queryAt = (pageOffset: number, sort: S): PageQuery<T> => ({
    key: options.cacheKey(pageOffset, sort),
    fetch: (range) => options.fetch(range, pageOffset, sort),
  });
  const table = createPageSlot({
    requests,
    slot: name,
    cache,
    errorFallback,
    clearOnFetch,
    query: () => queryAt(offset(), options.sort()),
  });
  const { treemapSort } = options;
  const treemap =
    treemapSort === undefined
      ? undefined
      : createPageSlot({
          requests,
          slot: `${name}Treemap`,
          cache,
          errorFallback,
          clearOnFetch: false,
          query: () => queryAt(0, treemapSort),
        });
  return {
    table,
    treemap,
    cache,
    offset,
    currentKey: () => options.cacheKey(offset(), options.sort()),
    changeOffset: (next) => {
      setOffset(next);
      if (clearOnFetch) {
        table.clearPage();
      }
    },
    sync: (range) => {
      table.sync(range);
      treemap?.sync(range);
    },
    reset: () => {
      cache.clear();
      setOffset(0);
      table.clearPage();
      treemap?.clearPage();
    },
  };
}

export type CursorTabData<T> = {
  table: PageSlot<T>;
  /** Row offset of the current page, derived from how many pages back it is. */
  offset: Accessor<number>;
  changePage: (direction: "previous" | "next") => void;
  sync: (range: DateRangeSeconds) => void;
  reset: () => void;
};

/** Data for a tab whose API pages by opaque cursor rather than offset. */
export function createCursorTabData<T extends { next_cursor?: string | null }>(options: {
  requests: RequestTracker;
  name: string;
  errorFallback: string;
  pageSize: number;
  fetch: (range: DateRangeSeconds, cursor: string | undefined) => Promise<T>;
}): CursorTabData<T> {
  const cache = createGenerationalCache<T>();
  /** Cursor used to fetch the current page; undefined for the first page. */
  const [cursor, setCursor] = createSignal<string | undefined>();
  /** Prior page cursors so Previous can walk back without offset support. */
  const [history, setHistory] = createSignal<Array<string | undefined>>([]);
  const table = createPageSlot({
    requests: options.requests,
    slot: options.name,
    cache,
    errorFallback: options.errorFallback,
    clearOnFetch: true,
    query: () => {
      const current = cursor();
      return {
        key: `cursor:${current ?? "start"}`,
        fetch: (range) => options.fetch(range, current),
      };
    },
  });
  const goTo = (next: string | undefined, nextHistory: Array<string | undefined>) => {
    setHistory(nextHistory);
    setCursor(next);
    table.clearPage();
  };
  return {
    table,
    offset: createMemo(() => history().length * options.pageSize),
    changePage: (direction) => {
      if (direction === "previous") {
        const past = history();
        if (past.length > 0) {
          goTo(past[past.length - 1], past.slice(0, -1));
        }
        return;
      }
      const nextCursor = table.data()?.next_cursor;
      if (nextCursor) {
        goTo(nextCursor, [...history(), cursor()]);
      }
    },
    sync: table.sync,
    reset: () => {
      cache.clear();
      goTo(undefined, []);
    },
  };
}

export type SnapshotData<T> = LoadState<T> & {
  entry: Accessor<CacheEntry<T> | undefined>;
  sync: (range: DateRangeSeconds) => void;
  clear: () => void;
};

/**
 * Data fetched whole rather than paged, cached as one entry that a background
 * refresh replaces once stale.
 */
export function createSnapshotData<T>(options: {
  requests: RequestTracker;
  slot: string;
  errorFallback: string;
  /** What the data depends on beyond the cache generation, so equal requests share a fetch. */
  inFlightKey: (range: DateRangeSeconds) => string;
  fetch: (range: DateRangeSeconds) => Promise<T>;
}): SnapshotData<T> {
  const [entry, setEntry] = createSignal<CacheEntry<T> | undefined>();
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal("");
  const runInFlight = createInFlightRunner();
  let generation = 0;
  const currentGeneration = () => generation;

  const load = (range: DateRangeSeconds, background: boolean) =>
    runInFlight(`${generation}:${options.inFlightKey(range)}`, async () => {
      const request = options.requests.begin(options.slot, currentGeneration);
      const replacing = !background || entry() === undefined;
      if (replacing) {
        request.claimLoading();
        setLoading(true);
        setError("");
      }
      try {
        const data = await options.fetch(range);
        if (request.isCurrent()) {
          setEntry({ data, fetchedAt: Date.now() });
        }
      } catch (loadError) {
        if (request.isCurrent() && replacing) {
          setEntry(undefined);
          setError(errorMessage(loadError, options.errorFallback));
        }
      } finally {
        if (request.releaseLoading()) {
          setLoading(false);
        }
      }
    });

  const data = createMemo(() => entry()?.data);
  const state = { data, loading, error };
  return {
    ...state,
    entry,
    settledEmpty: (hasData) => settledEmpty(state, hasData),
    sync: (range) => {
      const current = entry();
      if (!current) {
        void load(range, false);
      } else if (isCacheEntryStale(current.fetchedAt)) {
        void load(range, true);
      }
    },
    clear: () => {
      generation += 1;
      setEntry(undefined);
    },
  };
}
