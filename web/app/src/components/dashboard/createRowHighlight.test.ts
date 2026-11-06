import { createEffect, createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createRowHighlight } from "./createRowHighlight";

function tbodyWithRows(...keys: string[]) {
  const tbody = document.createElement("tbody");
  for (const key of keys) {
    const row = document.createElement("tr");
    row.dataset.recordKey = key;
    row.scrollIntoView = vi.fn();
    tbody.append(row);
  }
  return tbody;
}

describe("createRowHighlight", () => {
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "requestAnimationFrame"] });
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("highlights the row, then fades even when the caller clears its target", () => {
    const tbody = tbodyWithRows("a", "b");
    const { highlightKey, target, dispose } = createRoot((dispose) => {
      const { highlightKey, scrollToRow } = createRowHighlight();
      const [target, setTarget] = createSignal<string | null>("b");
      createEffect(() => {
        const key = target();
        if (key) {
          scrollToRow(tbody, key, () => setTarget(null));
        }
      });
      return { highlightKey, target, dispose };
    });

    vi.advanceTimersToNextFrame();
    expect(highlightKey()).toBe("b");
    expect((tbody.children[1] as HTMLElement).scrollIntoView).toHaveBeenCalled();
    expect(target()).toBeNull();

    vi.advanceTimersByTime(2000);
    expect(highlightKey()).toBeNull();
    dispose();
  });

  it("reports a missing row without highlighting", () => {
    const tbody = tbodyWithRows("a");
    createRoot((dispose) => {
      const { highlightKey, scrollToRow } = createRowHighlight();
      const onDone = vi.fn();
      scrollToRow(tbody, "missing", onDone);
      vi.advanceTimersToNextFrame();
      expect(onDone).toHaveBeenCalledWith(false);
      expect(highlightKey()).toBeNull();
      dispose();
    });
  });
});
