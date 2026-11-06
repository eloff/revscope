/// <reference types="solid-js" />
import type JsVectorMap from "jsvectormap";
// Static CSS import so Vite emits/links the stylesheet in the client bundle.
// A dynamic `import("…css")` was preloading a hashed URL that 404'd in the
// Cloudflare production build, aborting map init before the constructor ran.
import "jsvectormap/dist/jsvectormap.css";
import { createEffect, createSignal, onCleanup, onMount, Show } from "solid-js";
import { audienceMapHost } from "./AudienceMap.css";
import { NO_DATA_REGION_FILL, REVENUE_BUCKET_COLORS } from "./audienceLogic";
import { chartMount } from "./chart.css";
import { errorMessage, message } from "./messages.css";

export type AudienceMapProps = {
  /** Country alpha-2 code → 1-based revenue bucket (1 lightest … N darkest). */
  buckets: Map<string, number>;
  loading: boolean;
  error: string;
  /**
   * Called as the cursor moves over the map. `code` is the hovered country's
   * alpha-2 code (or `null` over ocean / outside any region); `clientX`/`clientY`
   * are viewport coordinates used to anchor the hover card.
   */
  onHover: (code: string | null, clientX: number, clientY: number) => void;
  onLeave: () => void;
  onRegionSelect?: (code: string) => void;
};

/** Resolve the bucket color for a country, or the neutral no-data fill. */
function fillForCode(buckets: Map<string, number>, code: string): string {
  const bucket = buckets.get(code);
  if (bucket === undefined) {
    return NO_DATA_REGION_FILL;
  }
  return REVENUE_BUCKET_COLORS[bucket - 1] ?? NO_DATA_REGION_FILL;
}

export default function AudienceMap(props: AudienceMapProps) {
  let host: HTMLDivElement | undefined;
  let map: JsVectorMap | undefined;
  let ready = false;
  const [initError, setInitError] = createSignal("");

  /** Paint every region path from the current bucket assignment. */
  const applyFills = () => {
    if (!host) {
      return;
    }
    const regions = host.querySelectorAll<SVGPathElement>(".jvm-region[data-code]");
    for (const region of regions) {
      const code = region.getAttribute("data-code");
      if (!code) {
        continue;
      }
      region.style.setProperty("fill", fillForCode(props.buckets, code));
    }
  };

  const handleMove = (event: MouseEvent) => {
    const target = event.target;
    const region = target instanceof Element ? target.closest(".jvm-region[data-code]") : null;
    const code = region?.getAttribute("data-code") ?? null;
    props.onHover(code, event.clientX, event.clientY);
  };

  const handleLeave = () => {
    props.onLeave();
  };

  const handleClick = (event: MouseEvent) => {
    const target = event.target;
    const region = target instanceof Element ? target.closest(".jvm-region[data-code]") : null;
    const code = region?.getAttribute("data-code");
    if (code) {
      props.onRegionSelect?.(code);
    }
  };

  onMount(() => {
    let disposed = false;
    void (async () => {
      try {
        // jsvectormap registers its maps on a global it sets at import time, so
        // the core module must load before the world map. Both touch `window`,
        // so we only import them on the client (inside onMount).
        const { default: JsVectorMapCtor } = await import("jsvectormap");
        await import("jsvectormap/dist/maps/world.js");
        if (disposed || !host) {
          return;
        }
        map = new JsVectorMapCtor({
          selector: host,
          map: "world",
          backgroundColor: "transparent",
          draggable: true,
          zoomButtons: true,
          zoomOnScroll: true,
          // Keep the tooltip machinery alive (so teardown is clean) but suppress
          // the built-in tooltip — we render an ark-ui hover card instead.
          showTooltip: true,
          regionsSelectable: false,
          regionStyle: {
            initial: {
              fill: NO_DATA_REGION_FILL,
              stroke: "#ffffff",
              strokeWidth: 0.4,
            },
            hover: { fillOpacity: 0.85, cursor: "pointer" },
          },
          onLoaded: () => applyFills(),
          onRegionTooltipShow: (event: Event) => event.preventDefault(),
        } as ConstructorParameters<typeof JsVectorMapCtor>[0]);
        ready = true;
        applyFills();
        host.addEventListener("mousemove", handleMove);
        host.addEventListener("mouseleave", handleLeave);
        host.addEventListener("click", handleClick);
      } catch (error) {
        if (!disposed) {
          const message = error instanceof Error ? error.message : String(error);
          setInitError(message || "Failed to load map");
        }
      }
    })();

    onCleanup(() => {
      disposed = true;
      if (host) {
        host.removeEventListener("mousemove", handleMove);
        host.removeEventListener("mouseleave", handleLeave);
        host.removeEventListener("click", handleClick);
      }
      try {
        map?.destroy();
      } catch {
        // ignore teardown errors from the underlying library
      }
      map = undefined;
      ready = false;
      if (host) {
        host.innerHTML = "";
      }
    });
  });

  createEffect(() => {
    // Re-track buckets so colors refresh when the period changes.
    void props.buckets;
    if (ready) {
      applyFills();
    }
  });

  const errorText = () => props.error || initError();

  return (
    <div class={chartMount}>
      <Show when={props.loading && !errorText()}>
        <p class={message}>Loading map…</p>
      </Show>
      <Show when={!props.loading && errorText()}>
        <p class={errorMessage}>{errorText()}</p>
      </Show>
      {/* The host stays visible (never display:none) so jsvectormap can
          measure a non-zero width when it initializes. */}
      <div ref={host} class={audienceMapHost} />
    </div>
  );
}
