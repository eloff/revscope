import { style } from "@vanilla-extract/css";

export const breakdownPanel = style({
  minWidth: 0,
  overflow: "hidden",
  border: "1px solid #e5e7eb",
  borderRadius: "12px",
  background: "#ffffff",
});

export const nestedTabList = style({
  display: "flex",
  gap: "4px",
  padding: "8px 8px 0",
  borderBottom: "1px solid #e5e7eb",
  background: "#ffffff",
});

export const nestedTabTrigger = style({
  flex: "1 1 auto",
  padding: "8px 10px",
  border: "none",
  borderRadius: "8px 8px 0 0",
  background: "transparent",
  color: "#6b7280",
  fontFamily: "inherit",
  fontSize: "0.88rem",
  lineHeight: 1.25,
  fontWeight: 600,
  cursor: "pointer",
  selectors: {
    '&[data-selected=""], &[data-selected="true"], &[aria-selected="true"]': {
      background: "#f8fafc",
      color: "#111827",
      boxShadow: "inset 0 -2px 0 #2563eb",
    },
  },
});

export const nestedTabContent = style({
  minHeight: "260px",
  padding: "12px",
  overflowX: "auto",
  boxSizing: "border-box",
});
