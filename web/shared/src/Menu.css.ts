// A dropdown menu's parts, shared by the account menu and the app's row menus.
import { style } from "@vanilla-extract/css";

export const menuPositioner = style({
  zIndex: 60,
});

export const menuContent = style({
  minWidth: "12rem",
  maxHeight: "24rem",
  overflowY: "auto",
  padding: "0.35rem",
  border: "1px solid #e5e7eb",
  borderRadius: "0.625rem",
  background: "#ffffff",
  boxShadow: "0 10px 30px rgba(0, 0, 0, 0.12)",
  outline: "none",
});

export const menuItem = style({
  display: "flex",
  alignItems: "center",
  gap: "0.5rem",
  width: "100%",
  boxSizing: "border-box",
  padding: "0.55rem 0.75rem",
  border: 0,
  borderRadius: "0.4rem",
  background: "transparent",
  color: "#111827",
  fontSize: "0.9rem",
  fontWeight: 500,
  lineHeight: 1.3,
  textAlign: "left",
  textDecoration: "none",
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

export const menuSeparator = style({
  height: "1px",
  margin: "0.3rem 0.4rem",
  border: 0,
  background: "#e5e7eb",
});
