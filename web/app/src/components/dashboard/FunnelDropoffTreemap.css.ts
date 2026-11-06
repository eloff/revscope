import { globalStyle, style } from "@vanilla-extract/css";

export const funnelDropoffChartMount = style({
  width: "100%",
  position: "relative",
});

globalStyle(`${funnelDropoffChartMount} .apexcharts-treemap-rect`, {
  cursor: "pointer",
});

export const funnelDropoffTreemapMount = style({
  width: "100%",
  minHeight: "220px",
});
