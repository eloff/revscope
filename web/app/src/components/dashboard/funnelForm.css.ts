import { style } from "@vanilla-extract/css";

export const funnelField = style({
  display: "flex",
  flexDirection: "column",
  gap: "6px",
});

export const funnelFieldLabel = style({
  color: "#374151",
  fontSize: "0.8rem",
  fontWeight: 600,
  letterSpacing: "0.01em",
});

export const funnelInput = style({
  width: "100%",
  boxSizing: "border-box",
  padding: "9px 12px",
  border: "1px solid #d1d5db",
  borderRadius: "8px",
  background: "#ffffff",
  color: "#111827",
  font: "inherit",
  fontSize: "0.92rem",
  transition: "border-color 0.15s ease, box-shadow 0.15s ease",
  selectors: {
    "&:hover:not(:focus)": {
      borderColor: "#9ca3af",
    },
    "&:focus": {
      outline: "none",
      borderColor: "#2563eb",
      boxShadow: "0 0 0 3px rgba(37, 99, 235, 0.12)",
    },
  },
});

// Chevron replaces the platform-native select arrow for a consistent look.
export const funnelSelect = style([
  funnelInput,
  {
    appearance: "none",
    paddingRight: "34px",
    backgroundImage: `url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='%236b7280' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'%3E%3Cpath d='m6 9 6 6 6-6'/%3E%3C/svg%3E")`,
    backgroundRepeat: "no-repeat",
    backgroundPosition: "right 10px center",
    backgroundSize: "16px",
    cursor: "pointer",
  },
]);

export const funnelTextarea = style([
  funnelInput,
  {
    minHeight: "42px",
    resize: "vertical",
  },
]);

export const funnelStepHeader = style({
  display: "flex",
  alignItems: "center",
  justifyContent: "space-between",
  gap: "12px",
});

export const funnelStepHeaderInfo = style({
  display: "flex",
  alignItems: "center",
  gap: "10px",
  minWidth: 0,
});

export const funnelStepIcon = style({
  width: "15px",
  height: "15px",
});
