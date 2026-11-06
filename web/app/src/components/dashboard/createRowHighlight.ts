import { createSignal, onCleanup } from "solid-js";

const HIGHLIGHT_DURATION_MS = 2000;

/** Scrolls a grid row into view and highlights it for a moment. */
export function createRowHighlight() {
  const [highlightKey, setHighlightKey] = createSignal<string | null>(null);
  // Owned by the component, not the calling effect: callers often clear their
  // scroll target in `onDone`, which reruns the effect, and the highlight must
  // still fade.
  let highlightTimer: number | undefined;
  const stopHighlightTimer = () => window.clearTimeout(highlightTimer);
  onCleanup(stopHighlightTimer);

  /**
   * Call from an effect. Waits a frame so rows rendered by the same update are
   * in the DOM, then finds `tr[data-record-key]` matching `key` in `tbody`.
   * Rerunning or disposing the effect cancels a pending search.
   */
  const scrollToRow = (tbody: HTMLElement, key: string, onDone?: (found: boolean) => void) => {
    const frame = requestAnimationFrame(() => {
      const row = [...tbody.querySelectorAll<HTMLTableRowElement>("tr[data-record-key]")].find(
        (candidate) => candidate.dataset.recordKey === key,
      );
      if (row) {
        row.scrollIntoView({ behavior: "smooth", block: "center" });
        stopHighlightTimer();
        setHighlightKey(key);
        highlightTimer = window.setTimeout(() => setHighlightKey(null), HIGHLIGHT_DURATION_MS);
      }
      onDone?.(row !== undefined);
    });
    onCleanup(() => cancelAnimationFrame(frame));
  };

  return { highlightKey, scrollToRow };
}
