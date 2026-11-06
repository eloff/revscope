import { globalStyle, style } from "@vanilla-extract/css";

export const funnelStepCard = style({
  display: "flex",
  flexDirection: "column",
  gap: "12px",
  padding: "14px 16px 16px",
  border: "1px solid #e5e7eb",
  borderRadius: "12px",
  background: "#ffffff",
  boxShadow: "0 1px 2px rgba(15, 23, 42, 0.04)",
  transition: "border-color 0.15s ease, box-shadow 0.15s ease",
  selectors: {
    "&:focus-within": {
      borderColor: "#bfdbfe",
      boxShadow: "0 1px 2px rgba(15, 23, 42, 0.04), 0 0 0 3px rgba(37, 99, 235, 0.06)",
    },
  },
});

export const funnelStepBadge = style({
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  width: "26px",
  height: "26px",
  flexShrink: 0,
  borderRadius: "999px",
  background: "#2563eb",
  color: "#ffffff",
  fontSize: "0.8rem",
  fontWeight: 700,
});

export const funnelStepActions = style({
  display: "flex",
  alignItems: "center",
  gap: "2px",
});

export const funnelStepIconButton = style({
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  width: "28px",
  height: "28px",
  padding: 0,
  border: "none",
  borderRadius: "7px",
  background: "transparent",
  color: "#9ca3af",
  cursor: "pointer",
  transition: "background 0.12s ease, color 0.12s ease",
  selectors: {
    "&:hover:not(:disabled)": {
      background: "#f3f4f6",
      color: "#111827",
    },
    "&:disabled": {
      color: "#e5e7eb",
      cursor: "default",
    },
  },
});

export const funnelStepIconButtonDanger = style({
  selectors: {
    "&:hover:not(:disabled)": {
      background: "#fef2f2",
      color: "#b91c1c",
    },
  },
});

export const funnelStepFields = style({
  display: "grid",
  gridTemplateColumns: "repeat(3, minmax(0, 1fr))",
  gap: "12px",
  "@media": {
    "screen and (max-width: 760px)": {
      gridTemplateColumns: "1fr",
    },
  },
});

export const funnelStepFullWidth = style({
  gridColumn: "1 / -1",
});

export const funnelCheckboxRow = style({
  display: "flex",
  alignItems: "center",
  gap: "8px",
  color: "#374151",
  fontSize: "0.88rem",
  fontWeight: 500,
  cursor: "pointer",
});

globalStyle(`${funnelCheckboxRow} input[type="checkbox"]`, {
  width: "15px",
  height: "15px",
  margin: 0,
  accentColor: "#2563eb",
  cursor: "pointer",
});
