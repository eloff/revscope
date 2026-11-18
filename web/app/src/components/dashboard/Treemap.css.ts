import { globalStyle, style } from "@vanilla-extract/css";

export const treemapRoot = style({
  width: "100%",
});

export const treemapChartArea = style({
  position: "relative",
});

/** Floats over the top-right tile, above the chart's SVG. */
export const treemapMetricToggle = style({
  position: "absolute",
  top: "6px",
  right: "6px",
  zIndex: 2,
  borderRadius: "999px",
  boxShadow: "0 1px 3px rgba(17, 24, 39, 0.18)",
});

globalStyle(`${treemapRoot} .apexcharts-treemap-rect`, {
  cursor: "pointer",
});
