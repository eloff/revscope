import { style } from "@vanilla-extract/css";

const actionBase = {
  border: "1px solid #d1d5db",
  background: "white",
  color: "#111827",
  padding: "0.5rem 0.875rem",
  borderRadius: "0.5rem",
  textDecoration: "none",
  fontSize: "0.95rem",
  lineHeight: 1.2,
  cursor: "pointer",
  selectors: {
    "&:hover": {
      background: "#f9fafb",
    },
  },
} as const;

export const nav = style({
  display: "flex",
  gap: "0.75rem",
  alignItems: "center",
  flexWrap: "wrap",
});

export const loginButton = style(actionBase);

export const link = style(actionBase);

export const userChip = style({
  display: "inline-flex",
  alignItems: "center",
  gap: "0.5rem",
  // Lets a narrow container (a phone header) shrink the chip, clipping the project name, rather
  // than the chip overflowing onto its neighbours.
  minWidth: 0,
  padding: "0.25rem 0.5rem 0.25rem 0.25rem",
  borderRadius: "999px",
  border: "1px solid #d1d5db",
  background: "#ffffff",
  color: "#111827",
  font: "inherit",
  cursor: "pointer",
  selectors: {
    "&:hover": {
      background: "#f9fafb",
    },
  },
});

export const avatar = style({
  width: "1.75rem",
  height: "1.75rem",
  borderRadius: "999px",
  objectFit: "cover",
  background: "#e5e7eb",
  color: "#374151",
  display: "grid",
  placeItems: "center",
  fontSize: "0.75rem",
  fontWeight: 700,
  flexShrink: 0,
});

export const userName = style({
  display: "inline-flex",
  alignItems: "baseline",
  gap: "0.35rem",
  minWidth: 0,
  maxWidth: "14rem",
  fontSize: "0.9rem",
  color: "#111827",
  overflow: "hidden",
});

export const chipProject = style({
  fontWeight: 500,
  minWidth: 0,
  overflow: "hidden",
  textOverflow: "ellipsis",
  whiteSpace: "nowrap",
});

export const chipSeparator = style({
  color: "#9ca3af",
  flexShrink: 0,
});

export const menuIcon = style({
  display: "block",
  width: "1rem",
  height: "1rem",
  color: "#6b7280",
  flexShrink: 0,
});

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

/** Reserves checkmark width so selected and unselected project rows stay aligned. */
export const menuItemIndicatorSlot = style({
  display: "grid",
  placeItems: "center",
  width: "0.85rem",
  flexShrink: 0,
  color: "#2563eb",
  fontSize: "0.75rem",
  lineHeight: 1,
});

export const menuItemText = style({
  flex: 1,
  minWidth: 0,
  overflow: "hidden",
  textOverflow: "ellipsis",
});

export const menuSeparator = style({
  height: "1px",
  margin: "0.3rem 0.4rem",
  border: 0,
  background: "#e5e7eb",
});
