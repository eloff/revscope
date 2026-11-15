import { style } from "@vanilla-extract/css";

export const metricsBar = style({
  marginBottom: "20px",
  width: "100%",
  overflowX: "auto",
  paddingBottom: "20px",
});

// At max-content width every 1fr column grows to the widest tile, so the tiles share one
// size without a fixed width that a long value could overflow.
export const metricsBarTiles = style({
  display: "grid",
  gridAutoFlow: "column",
  gridAutoColumns: "1fr",
  gap: "12px",
  width: "max-content",
});

export const metricTileToggle = style({
  display: "block",
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
