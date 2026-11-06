import { style } from "@vanilla-extract/css";

export const audiencePanel = style({
  display: "flex",
  flexDirection: "column",
  gap: "16px",
  width: "100%",
});

export const audienceMapWrapper = style({
  position: "relative",
  width: "100%",
});

export const audienceLegend = style({
  display: "flex",
  alignItems: "center",
  gap: "10px",
  flexWrap: "wrap",
});

export const audienceLegendLabel = style({
  fontSize: "0.78rem",
  fontWeight: 600,
  color: "#6b7280",
});

export const audienceLegendScale = style({
  display: "flex",
  alignItems: "center",
  gap: "4px",
});

export const audienceLegendCaption = style({
  fontSize: "0.72rem",
  color: "#9ca3af",
  margin: "0 2px",
});

export const audienceLegendSwatch = style({
  display: "inline-block",
  width: "26px",
  height: "12px",
  borderRadius: "2px",
  border: "1px solid rgba(15, 23, 42, 0.08)",
});

export const audienceBreakdownGrid = style({
  display: "grid",
  gridTemplateColumns: "minmax(0, 3fr) minmax(0, 2fr)",
  gap: "16px",
  width: "100%",
  "@media": {
    "screen and (max-width: 960px)": {
      gridTemplateColumns: "1fr",
    },
  },
});
