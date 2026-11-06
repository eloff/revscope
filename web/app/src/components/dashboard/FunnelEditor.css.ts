import { globalStyle, style } from "@vanilla-extract/css";

export const funnelAddButton = style({
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  gap: "8px",
  width: "100%",
  padding: "12px",
  border: "1px dashed #93c5fd",
  borderRadius: "12px",
  background: "transparent",
  color: "#1d4ed8",
  font: "inherit",
  fontSize: "0.9rem",
  fontWeight: 700,
  textAlign: "center",
  cursor: "pointer",
  transition: "background 0.15s ease, border-color 0.15s ease",
  selectors: {
    "&:hover": {
      background: "#eff6ff",
      borderColor: "#60a5fa",
    },
  },
});

export const funnelEditor = style({
  display: "flex",
  flexDirection: "column",
  gap: "18px",
  width: "100%",
  maxWidth: "860px",
  margin: "0 auto",
});

export const funnelFormGrid = style({
  display: "grid",
  gridTemplateColumns: "minmax(0, 1fr) minmax(0, 1.6fr)",
  gap: "14px",
  "@media": {
    "screen and (max-width: 760px)": {
      gridTemplateColumns: "1fr",
    },
  },
});

export const funnelSectionLabel = style({
  margin: "4px 0 -8px",
  color: "#111827",
  fontSize: "0.95rem",
  fontWeight: 700,
});

// Connectors between cards provide the vertical rhythm, so the list itself
// has no gap.
export const funnelStepList = style({
  display: "flex",
  flexDirection: "column",
});

export const funnelPaymentStepTitle = style({
  color: "#14532d",
  fontSize: "0.92rem",
  fontWeight: 700,
});

export const funnelStepConnector = style({
  display: "flex",
  flexDirection: "column",
  alignItems: "center",
  gap: "2px",
  padding: "4px 0",
});

export const funnelStepConnectorLine = style({
  width: "2px",
  height: "10px",
  borderRadius: "1px",
  background: "#e2e8f0",
});

export const funnelStepConnectorBadge = style({
  display: "inline-flex",
  alignItems: "center",
  gap: "5px",
  padding: "3px 10px",
  border: "1px solid #e5e7eb",
  borderRadius: "999px",
  background: "#ffffff",
  color: "#6b7280",
  fontSize: "0.75rem",
  fontWeight: 600,
  whiteSpace: "nowrap",
});

export const funnelStepConnectorValue = style({
  color: "#111827",
  fontWeight: 700,
  fontVariantNumeric: "tabular-nums",
});

export const funnelToggleRow = style({
  display: "flex",
  alignItems: "flex-start",
  gap: "12px",
  padding: "14px 16px",
  border: "1px solid #e5e7eb",
  borderRadius: "12px",
  background: "#ffffff",
  cursor: "pointer",
  transition: "border-color 0.15s ease",
  selectors: {
    "&:hover": {
      borderColor: "#bfdbfe",
    },
  },
});

export const funnelToggleInput = style({
  position: "absolute",
  width: "1px",
  height: "1px",
  margin: 0,
  padding: 0,
  opacity: 0,
  pointerEvents: "none",
});

export const funnelToggleTrack = style({
  position: "relative",
  flexShrink: 0,
  width: "40px",
  height: "22px",
  marginTop: "1px",
  borderRadius: "999px",
  background: "#d1d5db",
  transition: "background 0.15s ease",
  selectors: {
    [`${funnelToggleInput}:checked + &`]: {
      background: "#2563eb",
    },
    [`${funnelToggleInput}:focus-visible + &`]: {
      boxShadow: "0 0 0 2px #ffffff, 0 0 0 4px #93c5fd",
    },
  },
});

export const funnelToggleThumb = style({
  position: "absolute",
  top: "2px",
  left: "2px",
  width: "18px",
  height: "18px",
  borderRadius: "999px",
  background: "#ffffff",
  boxShadow: "0 1px 2px rgba(15, 23, 42, 0.2)",
  transition: "transform 0.15s ease",
});

globalStyle(`${funnelToggleInput}:checked + ${funnelToggleTrack} ${funnelToggleThumb}`, {
  transform: "translateX(18px)",
});

export const funnelToggleText = style({
  display: "flex",
  flexDirection: "column",
  gap: "2px",
});

export const funnelToggleTitle = style({
  color: "#111827",
  fontSize: "0.92rem",
  fontWeight: 600,
});

export const funnelToggleHint = style({
  color: "#6b7280",
  fontSize: "0.82rem",
  lineHeight: 1.4,
});

export const funnelChipGroup = style({
  display: "flex",
  flexWrap: "wrap",
  gap: "8px",
});

export const funnelChip = style({
  position: "relative",
  display: "inline-flex",
});

export const funnelChipInput = style({
  position: "absolute",
  width: "1px",
  height: "1px",
  margin: 0,
  padding: 0,
  opacity: 0,
  pointerEvents: "none",
});

export const funnelChipLabel = style({
  display: "inline-flex",
  alignItems: "center",
  gap: "6px",
  padding: "7px 14px",
  border: "1px solid #d1d5db",
  borderRadius: "999px",
  background: "#ffffff",
  color: "#4b5563",
  fontSize: "0.85rem",
  fontWeight: 600,
  cursor: "pointer",
  userSelect: "none",
  transition: "background 0.12s ease, border-color 0.12s ease, color 0.12s ease",
  selectors: {
    "&:hover": {
      borderColor: "#93c5fd",
    },
    [`${funnelChipInput}:checked + &`]: {
      borderColor: "#16a34a",
      background: "#f0fdf4",
      color: "#15803d",
    },
    [`${funnelChipInput}:focus-visible + &`]: {
      boxShadow: "0 0 0 2px #ffffff, 0 0 0 4px #93c5fd",
    },
  },
});

export const funnelPaymentStepCard = style({
  display: "flex",
  flexDirection: "column",
  gap: "12px",
  padding: "14px 16px 16px",
  border: "1px solid #bbf7d0",
  borderRadius: "12px",
  background: "#f0fdf4",
});

export const funnelPaymentStepFields = style({
  display: "grid",
  gridTemplateColumns: "repeat(2, minmax(0, 1fr))",
  gap: "12px",
  alignItems: "start",
  "@media": {
    "screen and (max-width: 760px)": {
      gridTemplateColumns: "1fr",
    },
  },
});
