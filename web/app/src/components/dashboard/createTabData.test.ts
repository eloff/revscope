import { createRoot, createSignal } from "solid-js";
import { describe, expect, it, vi } from "vitest";
import type { ColumnSort } from "./columnSort";
import { createOffsetTabData } from "./createTabData";
import { createRequestTracker } from "./dashboardRequests";
import type { TreemapMetric } from "./treemapLogic";

type Sort = ColumnSort<string>;

function createTab(sort: () => Sort) {
  return createOffsetTabData<object, Sort>({
    requests: createRequestTracker(),
    name: "rows",
    errorFallback: "",
    sort,
    cacheKey: (offset, current) => `${offset}:${current.column}:${current.direction}`,
    fetch: async () => ({}),
  });
}

describe("createOffsetTabData", () => {
  it("starts from the first page when the sort changes from elsewhere", () => {
    createRoot((dispose) => {
      const [sort, setSort] = createSignal<Sort>({ column: "revenue", direction: "desc" });
      const tab = createTab(sort);
      tab.changeOffset(200);
      setSort({ column: "visitors", direction: "desc" });
      expect(tab.offset()).toBe(0);
      // Switching back doesn't bring back the page it left.
      setSort({ column: "revenue", direction: "desc" });
      expect(tab.offset()).toBe(0);
      dispose();
    });
  });

  it("keeps the page when the sort is recomputed but unchanged", () => {
    createRoot((dispose) => {
      const [sort, setSort] = createSignal<Sort>({ column: "bounce", direction: "desc" });
      const tab = createTab(sort);
      tab.changeOffset(200);
      setSort({ column: "bounce", direction: "desc" });
      expect(tab.offset()).toBe(200);
      dispose();
    });
  });

  it("reports the ranking of the treemap page on show, not the one still loading", async () => {
    const [metric, setMetric] = createSignal<TreemapMetric>("revenue");
    const range = { start: 0, end: 1 };
    let release = () => {};
    const { tab, dispose } = createRoot((dispose) => ({
      dispose,
      tab: createOffsetTabData<object, Sort>({
        requests: createRequestTracker(),
        name: "rows",
        errorFallback: "",
        sort: () => ({ column: "bounce", direction: "desc" }),
        cacheKey: (offset, current) => `${offset}:${current.column}:${current.direction}`,
        fetch: (_range, _offset, sort) =>
          sort.column === "visitors"
            ? new Promise((resolve) => {
                release = () => resolve({});
              })
            : Promise.resolve({}),
        treemap: {
          metric,
          sort: (ranking) => ({
            column: ranking === "revenue" ? "revenue" : "visitors",
            direction: "desc",
          }),
        },
      }),
    }));
    tab.sync(range);
    await vi.waitFor(() => expect(tab.treemap.rankedBy()).toBe("revenue"));
    setMetric("traffic");
    tab.sync(range);
    expect(tab.treemap.rankedBy()).toBe("revenue");
    release();
    await vi.waitFor(() => expect(tab.treemap.rankedBy()).toBe("traffic"));
    dispose();
  });
});
