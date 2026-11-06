import { style } from "@vanilla-extract/css";

export const buttons = style({
  display: "grid",
  gap: "10px",
  justifyItems: "stretch",
});

/** Google draws its own button inside; this only reserves its height so the form doesn't jump. */
export const googleButton = style({
  display: "flex",
  justifyContent: "center",
  minHeight: "44px",
});

/** Black with the white logo, as Apple's guidelines ask for a custom button. */
export const appleButton = style({
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  gap: "8px",
  height: "40px",
  border: 0,
  borderRadius: "4px",
  background: "#000000",
  color: "#ffffff",
  fontSize: "0.95rem",
  fontWeight: 500,
  cursor: "pointer",
  selectors: {
    "&:disabled": {
      opacity: 0.6,
      cursor: "wait",
    },
  },
});

export const divider = style({
  display: "flex",
  alignItems: "center",
  gap: "10px",
  margin: "14px 0",
  color: "#6b7280",
  fontSize: "0.85rem",
  selectors: {
    "&::before": {
      content: '""',
      flex: 1,
      borderTop: "1px solid #e5e7eb",
    },
    "&::after": {
      content: '""',
      flex: 1,
      borderTop: "1px solid #e5e7eb",
    },
  },
});

export const error = style({
  margin: 0,
  color: "#dc2626",
  fontSize: "0.9rem",
});
