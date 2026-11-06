import { style, styleVariants } from "@vanilla-extract/css";

const triggerBase = style({
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "space-between",
  gap: "8px",
  minWidth: "180px",
  padding: "6px 10px",
  borderRadius: "8px",
  border: "1px solid #d1d5db",
  background: "#ffffff",
  color: "#111827",
  fontFamily: "inherit",
  fontSize: "0.9rem",
  lineHeight: 1.2,
  cursor: "pointer",
  selectors: {
    "&:hover": {
      background: "#f9fafb",
    },
  },
});

/** `toolbar` matches the height of the dashboard filter bar controls. */
export const trigger = styleVariants({
  default: [triggerBase],
  toolbar: [
    triggerBase,
    {
      height: "40px",
      padding: "0 12px",
      borderRadius: "10px",
      borderColor: "#e5e7eb",
      fontSize: "1rem",
      boxShadow: "0 1px 2px rgba(15, 23, 42, 0.04)",
    },
  ],
});

export const indicator = style({
  color: "#6b7280",
  fontSize: "0.75rem",
  lineHeight: 1,
});

export const positioner = style({
  zIndex: 60,
});

export const content = style({
  minWidth: "12rem",
  maxHeight: "18rem",
  overflowY: "auto",
  padding: "0.35rem",
  border: "1px solid #e5e7eb",
  borderRadius: "0.625rem",
  background: "#ffffff",
  boxShadow: "0 10px 30px rgba(0, 0, 0, 0.12)",
  outline: "none",
});

export const item = style({
  display: "block",
  padding: "0.5rem 0.7rem",
  borderRadius: "0.4rem",
  color: "#111827",
  fontSize: "0.9rem",
  fontWeight: 500,
  lineHeight: 1.3,
  cursor: "pointer",
  selectors: {
    "&[data-highlighted]": {
      background: "#f3f4f6",
    },
    "&[data-state='checked']": {
      color: "#2563eb",
      fontWeight: 600,
    },
  },
});
