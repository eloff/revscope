import { fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { describe, expect, it, vi } from "vitest";
import AudienceMap from "./AudienceMap";
import { NO_DATA_REGION_FILL, REVENUE_BUCKET_COLORS } from "./audienceLogic";

const maps = vi.hoisted(() => ({ created: [] as { destroyed: boolean }[], fail: false }));

// Stands in for jsvectormap: draws one path per region like the real library.
vi.mock("jsvectormap", () => ({
  default: class {
    destroyed = false;
    constructor(options: { selector: HTMLElement }) {
      if (maps.fail) {
        throw new Error("WebGL unavailable");
      }
      const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
      for (const code of ["US", "DE", "FR"]) {
        const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
        path.setAttribute("class", "jvm-region");
        path.setAttribute("data-code", code);
        svg.append(path);
      }
      options.selector.append(svg);
      maps.created.push(this);
    }
    destroy() {
      this.destroyed = true;
    }
  },
}));
vi.mock("jsvectormap/dist/maps/world.js", () => ({}));

const region = (code: string) =>
  document.querySelector<SVGPathElement>(`.jvm-region[data-code="${code}"]`) as SVGPathElement;

function renderMap(props: Partial<Parameters<typeof AudienceMap>[0]> = {}) {
  const handlers = { onHover: vi.fn(), onLeave: vi.fn(), onRegionSelect: vi.fn() };
  const result = render(() => (
    <AudienceMap
      buckets={
        new Map([
          ["US", 5],
          ["DE", 1],
        ])
      }
      loading={false}
      error=""
      {...handlers}
      {...props}
    />
  ));
  return { ...handlers, ...result };
}

describe("AudienceMap", () => {
  it("colors countries by revenue bucket, grey without data", async () => {
    renderMap();
    await vi.waitFor(() => expect(region("US")).toBeTruthy());
    expect(region("US")).toHaveStyle({ fill: REVENUE_BUCKET_COLORS[4] });
    expect(region("DE")).toHaveStyle({ fill: REVENUE_BUCKET_COLORS[0] });
    expect(region("FR")).toHaveStyle({ fill: NO_DATA_REGION_FILL });
  });

  it("recolors when the buckets change", async () => {
    const [buckets, setBuckets] = createSignal(new Map([["US", 5]]));
    render(() => (
      <AudienceMap
        buckets={buckets()}
        loading={false}
        error=""
        onHover={vi.fn()}
        onLeave={vi.fn()}
      />
    ));
    await vi.waitFor(() => expect(region("US")).toBeTruthy());
    setBuckets(new Map([["FR", 3]]));
    expect(region("US")).toHaveStyle({ fill: NO_DATA_REGION_FILL });
    expect(region("FR")).toHaveStyle({ fill: REVENUE_BUCKET_COLORS[2] });
  });

  it("reports hovering, leaving and selecting a country", async () => {
    const { onHover, onLeave, onRegionSelect, container } = renderMap();
    await vi.waitFor(() => expect(region("DE")).toBeTruthy());
    fireEvent.mouseMove(region("DE"), { clientX: 3, clientY: 4 });
    fireEvent.click(region("DE"));
    const host = container.querySelector("svg")?.parentElement as HTMLElement;
    fireEvent.mouseMove(host, { clientX: 1, clientY: 1 });
    fireEvent.mouseLeave(host);
    expect(onHover.mock.calls).toEqual([
      ["DE", 3, 4],
      [null, 1, 1],
    ]);
    expect(onRegionSelect).toHaveBeenCalledWith("DE");
    expect(onLeave).toHaveBeenCalled();
  });

  it("destroys the map on unmount", async () => {
    const { unmount } = renderMap();
    await vi.waitFor(() => expect(region("US")).toBeTruthy());
    const map = maps.created.at(-1);
    unmount();
    expect(map?.destroyed).toBe(true);
  });

  it("shows a map that fails to load", async () => {
    maps.fail = true;
    renderMap();
    expect(await screen.findByText("WebGL unavailable")).toBeInTheDocument();
    maps.fail = false;
  });

  it("shows loading and the tab error", () => {
    renderMap({ loading: true });
    expect(screen.getByText("Loading map…")).toBeInTheDocument();
  });
});
