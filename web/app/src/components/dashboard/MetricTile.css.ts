import { style } from "@vanilla-extract/css";

export const metricTile = style({
  display: "flex",
  flexDirection: "column",
  gap: "4px",
  minWidth: "132px",
  padding: "12px 14px",
  border: "2px solid transparent",
  borderRadius: "10px",
  background: "#f8fafc",
  boxSizing: "border-box",
  transition: "border-color 0.15s ease",
});

export const metricLabel = style({
  fontSize: "0.78rem",
  fontWeight: 600,
  color: "#6b7280",
  lineHeight: 1.2,
});

export const metricValueRow = style({
  display: "flex",
  alignItems: "baseline",
  gap: "4px",
  flexWrap: "wrap",
});

export const metricValue = style({
  fontSize: "1.15rem",
  fontWeight: 700,
  color: "#111827",
  lineHeight: 1.2,
});

export const metricUnit = style({
  fontSize: "0.82rem",
  fontWeight: 500,
  color: "#6b7280",
});
