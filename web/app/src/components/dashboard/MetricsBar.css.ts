import { style } from "@vanilla-extract/css";

export const metricsBar = style({
  marginBottom: "20px",
  width: "100%",
});

export const metricsBarScroll = style({
  display: "flex",
  flexWrap: "nowrap",
  justifyContent: "flex-start",
  alignItems: "stretch",
  gap: "12px",
  width: "100%",
  overflowX: "auto",
  paddingBottom: "20px",
});

export const metricTileToggle = style({
  display: "block",
  flexShrink: 0,
  padding: 0,
  margin: 0,
  border: "none",
  background: "transparent",
  font: "inherit",
  textAlign: "left",
  cursor: "pointer",
  borderRadius: "10px",
  selectors: {
    "&:focus-visible": {
      outline: "2px solid #2563eb",
      outlineOffset: "2px",
    },
  },
});
