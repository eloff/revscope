import { style } from "@vanilla-extract/css";

export const gridDrilldownTabsRoot = style({
  width: "100%",
});

export const gridDrilldownTabList = style({
  display: "inline-flex",
  gap: "4px",
  marginBottom: "12px",
  padding: "4px",
  border: "1px solid #e5e7eb",
  borderRadius: "999px",
  background: "#f8fafc",
});

export const gridDrilldownTabTrigger = style({
  padding: "6px 12px",
  border: "none",
  borderRadius: "999px",
  background: "transparent",
  color: "#6b7280",
  font: "inherit",
  fontSize: "0.85rem",
  fontWeight: 700,
  cursor: "pointer",
  selectors: {
    '&[data-selected=""], &[data-selected="true"], &[aria-selected="true"]': {
      background: "#ffffff",
      color: "#111827",
      boxShadow: "0 1px 3px rgba(15, 23, 42, 0.12)",
    },
    "&:focus-visible": {
      outline: "2px solid #2563eb",
      outlineOffset: "2px",
    },
  },
});

export const gridDrilldownTabContent = style({
  width: "100%",
});
