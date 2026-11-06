import { style } from "@vanilla-extract/css";

export const page = style({
  width: "100%",
  minHeight: "100vh",
  padding: "24px 28px 48px",
  boxSizing: "border-box",
  fontFamily: "Inter, Roboto, 'Helvetica Neue', Arial, sans-serif",
  color: "#111827",
  background: "#f8fafc",
});

export const embeddedShell = style({
  width: "100%",
  padding: "16px 0 0",
  boxSizing: "border-box",
  fontFamily: "Inter, Roboto, 'Helvetica Neue', Arial, sans-serif",
  color: "#111827",
  background: "#f8fafc",
  border: "1px solid #e5e7eb",
  borderRadius: "12px",
});

export const fullscreenShell = style({
  width: "100%",
  minHeight: "100vh",
  height: "100vh",
  padding: "20px 24px 32px",
  boxSizing: "border-box",
  fontFamily: "Inter, Roboto, 'Helvetica Neue', Arial, sans-serif",
  color: "#111827",
  background: "#f8fafc",
  overflow: "auto",
});

export const tabsRoot = style({
  width: "100%",
});

export const tabList = style({
  display: "flex",
  flexWrap: "wrap",
  gap: "4px",
  width: "100%",
  borderBottom: "1px solid #e5e7eb",
  marginBottom: "0",
  background: "#ffffff",
  borderRadius: "12px 12px 0 0",
  padding: "8px 8px 0",
  boxSizing: "border-box",
});

export const tabTrigger = style({
  flex: "0 1 auto",
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
  },
});

/** Homepage overview island: other tabs are real links to `/demo`. */
export const tabLink = style([
  tabTrigger,
  {
    textDecoration: "none",
    selectors: {
      "&:link, &:visited": {
        color: "#6b7280",
      },
      "&:hover": {
        color: "#111827",
      },
    },
  },
]);

export const tabIcon = style({
  opacity: 0.78,
  selectors: {
    [`${tabTrigger}[data-selected=""] &`]: {
      opacity: 1,
    },
    [`${tabTrigger}[data-selected="true"] &`]: {
      opacity: 1,
    },
    [`${tabTrigger}[aria-selected="true"] &`]: {
      opacity: 1,
    },
  },
});

export const tabContent = style({
  width: "100%",
  padding: "20px",
  border: "1px solid #e5e7eb",
  borderTop: "none",
  borderRadius: "0 0 12px 12px",
  background: "#ffffff",
  boxSizing: "border-box",
});

export const tabContentStack = style({
  display: "flex",
  flexDirection: "column",
  gap: "20px",
});
