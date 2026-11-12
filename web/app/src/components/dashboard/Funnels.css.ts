import { style } from "@vanilla-extract/css";

export const funnelsShell = style({
  display: "flex",
  flexDirection: "column",
  gap: "18px",
  minHeight: "420px",
});

export const funnelTabBar = style({
  display: "flex",
  alignItems: "stretch",
  gap: "4px",
  width: "100%",
  overflowX: "auto",
  borderBottom: "1px solid #e5e7eb",
});

export const funnelTabRail = style({
  display: "flex",
  gap: "4px",
  flex: "1 1 auto",
  minWidth: 0,
});

export const funnelTabButton = style({
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  padding: "9px 16px",
  border: "none",
  borderRadius: "8px 8px 0 0",
  background: "transparent",
  color: "#6b7280",
  fontFamily: "inherit",
  fontSize: "1.08rem",
  lineHeight: 1.25,
  fontWeight: 600,
  whiteSpace: "nowrap",
  cursor: "pointer",
  selectors: {
    '&[data-selected=""], &[data-selected="true"], &[aria-selected="true"]': {
      background: "#f8fafc",
      color: "#111827",
      boxShadow: "inset 0 -2px 0 #2563eb",
    },
    "&:hover:not([data-selected]):not([aria-selected='true'])": {
      color: "#111827",
    },
  },
});

export const funnelDocsLink = style({
  alignSelf: "center",
  marginBottom: "4px",
});

export const funnelAddTabButton = style({
  alignSelf: "center",
  flexShrink: 0,
  padding: "7px 12px",
  marginBottom: "4px",
  border: "1px dashed #93c5fd",
  borderRadius: "8px",
  background: "#eff6ff",
  color: "#1d4ed8",
  font: "inherit",
  fontSize: "0.88rem",
  fontWeight: 700,
  whiteSpace: "nowrap",
  cursor: "pointer",
  selectors: {
    "&:hover": {
      background: "#dbeafe",
    },
  },
});

export const funnelPanel = style({
  minWidth: 0,
});

export const funnelHeader = style({
  display: "flex",
  alignItems: "flex-start",
  justifyContent: "space-between",
  gap: "14px",
  marginBottom: "16px",
});

export const funnelTitle = style({
  margin: 0,
  fontSize: "1.25rem",
  lineHeight: 1.3,
  color: "#111827",
});

export const funnelDescription = style({
  margin: "4px 0 0",
  color: "#6b7280",
  fontSize: "0.9rem",
  lineHeight: 1.5,
});

export const funnelActions = style({
  display: "flex",
  flexWrap: "wrap",
  gap: "8px",
  justifyContent: "flex-end",
});

export const funnelEmptyState = style({
  padding: "42px 20px",
  border: "1px dashed #d1d5db",
  borderRadius: "12px",
  color: "#6b7280",
  textAlign: "center",
  background: "#f9fafb",
});
