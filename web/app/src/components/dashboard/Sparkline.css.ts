import { globalStyle, style } from "@vanilla-extract/css";

export const sparklineMount = style({
  width: "100%",
  height: "40px",
});

globalStyle(`${sparklineMount} .apexcharts-yaxis`, {
  display: "none",
});

globalStyle(`${sparklineMount} .apexcharts-xaxis`, {
  display: "none",
});
