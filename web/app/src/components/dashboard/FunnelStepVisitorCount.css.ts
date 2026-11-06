import { style } from "@vanilla-extract/css";

export const funnelStepVisitors = style({
  display: "inline-flex",
  alignItems: "baseline",
  gap: "5px",
  color: "#6b7280",
  fontSize: "0.82rem",
  fontWeight: 600,
  whiteSpace: "nowrap",
});

export const funnelStepVisitorsCount = style({
  fontSize: "0.98rem",
  fontWeight: 700,
  color: "#111827",
  fontVariantNumeric: "tabular-nums",
});

export const funnelStepVisitorsMuted = style({
  color: "#9ca3af",
  fontWeight: 500,
});
