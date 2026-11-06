import { style } from "@vanilla-extract/css";

export const summaryPanel = style({
  display: "flex",
  flexDirection: "column",
  gap: "20px",
  width: "100%",
});

export const summaryContent = style({
  position: "relative",
});

export const summaryBar = style({
  display: "flex",
  flexWrap: "wrap",
  gap: "12px",
});

export const summaryCard = style({
  display: "flex",
  flexDirection: "column",
  gap: "4px",
  flex: "1 1 140px",
  minWidth: "140px",
  padding: "12px 14px",
  border: "2px solid transparent",
  borderRadius: "10px",
  background: "#f8fafc",
  boxSizing: "border-box",
  cursor: "pointer",
  userSelect: "none",
  transition: "opacity 0.15s ease, background 0.15s ease",
  appearance: "none",
  font: "inherit",
  color: "inherit",
  textAlign: "left",
});

export const summaryCardDisabled = style({
  opacity: 0.45,
  background: "#f1f5f9",
});

export const summaryCardLabel = style({
  fontSize: "0.78rem",
  fontWeight: 600,
  color: "#6b7280",
  lineHeight: 1.2,
});

export const summaryCardMetric = style({
  display: "flex",
  alignItems: "baseline",
  gap: "6px",
  flexWrap: "wrap",
});

export const summaryCardValue = style({
  fontSize: "1.25rem",
  fontWeight: 700,
  color: "#111827",
  lineHeight: 1.2,
});

export const summaryCardSecondaryValue = style({
  fontSize: "1rem",
  fontWeight: 600,
  color: "#374151",
  lineHeight: 1.2,
});

export const summaryCardChange = style({
  fontSize: "0.78rem",
  fontWeight: 600,
  lineHeight: 1.2,
});
