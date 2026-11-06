import { style } from "@vanilla-extract/css";

export const runwayPanel = style({
  display: "flex",
  flexDirection: "column",
  gap: "20px",
  width: "100%",
});

export const runwayIntro = style({
  margin: 0,
  color: "#6b7280",
  fontSize: "0.95rem",
  lineHeight: 1.5,
});

export const inputCard = style({
  display: "flex",
  flexDirection: "column",
  gap: "16px",
  padding: "20px",
  background: "#ffffff",
  border: "1px solid #e5e7eb",
  borderRadius: "14px",
});

export const inputGrid = style({
  display: "grid",
  gridTemplateColumns: "repeat(auto-fit, minmax(200px, 1fr))",
  gap: "16px",
});

export const inputField = style({
  display: "flex",
  flexDirection: "column",
  gap: "6px",
});

export const inputLabel = style({
  display: "flex",
  alignItems: "center",
  gap: "6px",
  fontSize: "0.85rem",
  fontWeight: 600,
  color: "#374151",
});

export const inputLabelIcon = style({
  width: "16px",
  height: "16px",
  flex: "0 0 auto",
  color: "#2563eb",
});

export const inputLabelIconPositive = style([
  inputLabelIcon,
  {
    color: "#059669",
  },
]);

export const inputLabelIconBurn = style([
  inputLabelIcon,
  {
    color: "#f97316",
  },
]);

export const inputHint = style({
  fontSize: "0.75rem",
  color: "#9ca3af",
  lineHeight: 1.3,
});

export const inputControl = style({
  display: "flex",
  alignItems: "center",
  gap: "6px",
  padding: "9px 12px",
  border: "1px solid #d1d5db",
  borderRadius: "10px",
  background: "#ffffff",
  transition: "border-color 0.15s ease, box-shadow 0.15s ease",
  selectors: {
    "&:focus-within": {
      borderColor: "#2563eb",
      boxShadow: "0 0 0 3px rgba(37, 99, 235, 0.12)",
    },
  },
});

export const inputAffix = style({
  fontSize: "1rem",
  fontWeight: 600,
  color: "#6b7280",
  flexShrink: 0,
});

export const inputBox = style({
  flex: "1 1 auto",
  minWidth: 0,
  border: "none",
  outline: "none",
  background: "transparent",
  font: "inherit",
  fontSize: "1.05rem",
  fontWeight: 600,
  color: "#111827",
  appearance: "textfield",
  MozAppearance: "textfield",
  textAlign: "right",
  selectors: {
    "&::-webkit-outer-spin-button": {
      WebkitAppearance: "none",
      margin: 0,
    },
    "&::-webkit-inner-spin-button": {
      WebkitAppearance: "none",
      margin: 0,
    },
  },
});

export const inputDefaultNote = style({
  fontSize: "0.72rem",
  fontWeight: 600,
  color: "#059669",
});

export const inputLabelRow = style({
  display: "flex",
  alignItems: "center",
  justifyContent: "space-between",
  gap: "8px",
});

export const growthWindowSelect = style({
  border: "1px solid #d1d5db",
  borderRadius: "8px",
  background: "#f9fafb",
  color: "#374151",
  font: "inherit",
  fontSize: "0.72rem",
  fontWeight: 600,
  padding: "3px 6px",
  cursor: "pointer",
  selectors: {
    "&:focus-visible": {
      outline: "2px solid #2563eb",
      outlineOffset: "1px",
    },
  },
});

export const resetButton = style({
  alignSelf: "flex-start",
  display: "inline-flex",
  alignItems: "center",
  gap: "6px",
  padding: "6px 12px",
  border: "1px solid #d1d5db",
  borderRadius: "999px",
  background: "#f9fafb",
  color: "#374151",
  font: "inherit",
  fontSize: "0.8rem",
  fontWeight: 600,
  cursor: "pointer",
  selectors: {
    "&:hover": {
      background: "#f3f4f6",
      color: "#111827",
    },
    "&:disabled": {
      opacity: 0.5,
      cursor: "default",
    },
    "&:focus-visible": {
      outline: "2px solid #2563eb",
      outlineOffset: "2px",
    },
  },
});

export const summaryBar = style({
  display: "grid",
  gridTemplateColumns: "repeat(auto-fit, minmax(180px, 1fr))",
  gap: "12px",
});

export const summaryCard = style({
  display: "flex",
  flexDirection: "column",
  gap: "4px",
  padding: "14px 16px",
  border: "1px solid #e5e7eb",
  borderRadius: "12px",
  background: "#ffffff",
});

export const summaryLabel = style({
  fontSize: "0.78rem",
  fontWeight: 600,
  color: "#6b7280",
  lineHeight: 1.2,
});

export const summaryValue = style({
  fontSize: "1.5rem",
  fontWeight: 700,
  color: "#111827",
  lineHeight: 1.2,
});

export const summaryValuePositive = style({
  color: "#059669",
});

export const summaryValueNegative = style({
  color: "#dc2626",
});

export const summaryDetail = style({
  fontSize: "0.75rem",
  color: "#9ca3af",
  lineHeight: 1.3,
});

export const adviceCard = style({
  display: "flex",
  flexDirection: "column",
  gap: "6px",
  padding: "16px 18px",
  borderRadius: "12px",
  borderLeft: "4px solid transparent",
});

export const adviceCritical = style({
  background: "#fef2f2",
  borderLeftColor: "#dc2626",
});

export const adviceWarning = style({
  background: "#fffbeb",
  borderLeftColor: "#d97706",
});

export const adviceInfo = style({
  background: "#eff6ff",
  borderLeftColor: "#2563eb",
});

export const adviceHealthy = style({
  background: "#ecfdf5",
  borderLeftColor: "#059669",
});

export const adviceTitle = style({
  margin: 0,
  fontSize: "0.95rem",
  fontWeight: 700,
  color: "#111827",
});

export const adviceBody = style({
  margin: 0,
  fontSize: "0.88rem",
  lineHeight: 1.5,
  color: "#374151",
});

export const chartCard = style({
  display: "flex",
  flexDirection: "column",
  gap: "8px",
  padding: "20px",
  background: "#ffffff",
  border: "1px solid #e5e7eb",
  borderRadius: "14px",
});

export const chartTitle = style({
  margin: 0,
  fontSize: "1.05rem",
  fontWeight: 600,
  color: "#111827",
});

export const chartMount = style({
  width: "100%",
  minHeight: "420px",
});

export const emptyState = style({
  margin: 0,
  padding: "48px 16px",
  textAlign: "center",
  color: "#6b7280",
  fontSize: "0.95rem",
});
