import { globalStyle, style } from "@vanilla-extract/css";
import { gridTable } from "./grid.css";

export const crawlerDetailChartMount = style({
  width: "100%",
  height: "260px",
  marginBottom: "12px",
});

export const crawlerPageSparklineHeader = style({
  width: "42%",
  minWidth: "168px",
});

export const crawlerPageSparklineCell = style({
  width: "42%",
  minWidth: "168px",
});

export const crawlerPageViewsCell = style({
  width: "1%",
  whiteSpace: "nowrap",
});

globalStyle(`${gridTable} thead th${crawlerPageSparklineHeader}`, {
  padding: 0,
  width: "42%",
  minWidth: "168px",
});

globalStyle(`${gridTable} td${crawlerPageSparklineCell}`, {
  width: "42%",
  minWidth: "168px",
  paddingTop: "6px",
  paddingBottom: "6px",
  verticalAlign: "middle",
});

globalStyle(`${gridTable} th${crawlerPageViewsCell}`, {
  width: "1%",
  whiteSpace: "nowrap",
});

globalStyle(`${gridTable} td${crawlerPageViewsCell}`, {
  width: "1%",
  whiteSpace: "nowrap",
});
