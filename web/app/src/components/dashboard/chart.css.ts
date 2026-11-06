import { globalStyle, style } from "@vanilla-extract/css";

export const chartMount = style({
  width: "100%",
  minHeight: "420px",
});

export const chartMountHidden = style({
  display: "none",
});

globalStyle(`${chartMount} .apexcharts-treemap-rect`, {
  cursor: "pointer",
});
