/**
 * Minimal type surface for the subset of jsvectormap we use. The package ships
 * no type declarations, so this covers only the constructor options and the
 * instance methods touched by the Audience map.
 */
declare module "jsvectormap" {
  interface RegionStyleStates {
    initial?: Record<string, string | number>;
    hover?: Record<string, string | number>;
    selected?: Record<string, string | number>;
    selectedHover?: Record<string, string | number>;
  }

  interface RegionSeries {
    attribute?: string;
    /** Ordinal scale: maps a value (e.g. bucket index) to a style value (color). */
    scale?: Record<string | number, string>;
    /** Per-region value keyed by region code (e.g. alpha-2 → bucket index). */
    values?: Record<string, number>;
  }

  interface JsVectorMapOptions {
    selector: HTMLElement | string;
    map?: string;
    backgroundColor?: string;
    draggable?: boolean;
    zoomButtons?: boolean;
    zoomOnScroll?: boolean;
    showTooltip?: boolean;
    regionsSelectable?: boolean;
    regionStyle?: RegionStyleStates;
    series?: {
      regions?: RegionSeries[];
      markers?: RegionSeries[];
    };
    onLoaded?: (map: JsVectorMap) => void;
  }

  export default class JsVectorMap {
    constructor(options: JsVectorMapOptions);
    destroy(destroyInstance?: boolean): void;
    container: HTMLElement;
  }
}

declare module "jsvectormap/dist/maps/world.js";
declare module "jsvectormap/dist/jsvectormap.css";
