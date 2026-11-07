import { createSignal, onCleanup, onMount } from "solid-js";

/** Sent to the host page; the site's EmbedFrame.astro listens for it and must match. */
export const EMBED_EXPAND_MESSAGE = "revscope-embed-expand";
/** Sent by the host page when Esc is pressed while it has focus. */
export const EMBED_COLLAPSE_MESSAGE = "revscope-embed-collapse";
/** Dispatched on the window to make EmbedLayout report the embed's height to the host right away. */
export const EMBED_REPORT_HEIGHT_EVENT = "revscope-embed-report-height";

/**
 * Ark-ui popovers and menus close on Esc, so Esc closes them first and only then
 * collapses the embed. Their own listener runs after this one (we listen in the
 * capture phase), so the layer still reads as open here.
 */
function hasOpenLayer(): boolean {
  // Not every open "content" part is a layer the visitor opened: tab panels and
  // collapsibles report open too, and hover cards open just from the resting pointer.
  return (
    document.querySelector(
      '[data-part="positioner"] [data-state="open"]:not([data-scope="hover-card"]):not([data-scope="tooltip"]), [data-scope="dialog"][data-part="content"][data-state="open"]',
    ) !== null
  );
}

/** Free text such as an Ask question; the date picker's short inputs don't count. */
function isTextEntry(target: EventTarget | null): boolean {
  return (
    target instanceof HTMLTextAreaElement ||
    (target instanceof HTMLElement && target.isContentEditable)
  );
}

/**
 * The embed sits collapsed in the host page with no scrolling of its own, so the
 * wheel scrolls the host. Expanding asks the host to show the frame over the whole
 * viewport, where the embed scrolls as a normal page.
 */
export function createEmbedExpansion(embedId: string) {
  // Starts enabled so the server-rendered page already covers the demo with the
  // expand overlay; until hydration a click would otherwise follow a tab's link.
  // A page opened directly turns it off on mount.
  const [enabled, setEnabled] = createSignal(true);
  const [expanded, setExpanded] = createSignal(false);

  const setScrollable = (scrollable: boolean) => {
    document.documentElement.style.overflow = scrollable ? "" : "hidden";
  };
  const notifyHost = (value: boolean) => {
    window.parent.postMessage({ type: EMBED_EXPAND_MESSAGE, embed: embedId, expanded: value }, "*");
  };

  const expand = () => {
    if (!enabled() || expanded()) {
      return;
    }
    setExpanded(true);
    setScrollable(true);
    notifyHost(true);
  };

  const collapse = () => {
    if (!expanded()) {
      return;
    }
    setExpanded(false);
    window.scrollTo(0, 0);
    setScrollable(false);
    // The host measures the slot to animate into as soon as it hears of the
    // collapse, so it must first hear the collapsed height, which changes when
    // an expanded-only view (another tab) gives way to the overview.
    window.dispatchEvent(new Event(EMBED_REPORT_HEIGHT_EVENT));
    notifyHost(false);
  };

  onMount(() => {
    // Opened directly (not framed) the page scrolls normally and there is nothing to expand into.
    if (window.parent === window) {
      setEnabled(false);
      return;
    }
    // EmbedLayout already stopped the frame scrolling; this undoes any scrolling
    // done before then.
    window.scrollTo(0, 0);
    setScrollable(false);
    const onMessage = (event: MessageEvent) => {
      const data = event.data;
      if (
        event.source === window.parent &&
        data?.type === EMBED_COLLAPSE_MESSAGE &&
        data.embed === embedId
      ) {
        collapse();
      }
    };
    const onKeyDown = (event: KeyboardEvent) => {
      // Esc also cancels an input method's candidates, and collapsing would
      // discard what's typed (an Ask question); the close button still works.
      if (event.isComposing || isTextEntry(event.target)) {
        return;
      }
      if (event.key === "Escape" && expanded() && !hasOpenLayer()) {
        collapse();
      }
    };
    window.addEventListener("message", onMessage);
    window.addEventListener("keydown", onKeyDown, true);
    onCleanup(() => {
      window.removeEventListener("message", onMessage);
      window.removeEventListener("keydown", onKeyDown, true);
      setScrollable(true);
    });
  });

  return { enabled, expanded, expand, collapse };
}
