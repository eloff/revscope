import { style } from "@vanilla-extract/css";

export const page = style({
  width: "100%",
  maxWidth: "860px",
  margin: "0 auto",
  padding: "24px 28px 64px",
  fontFamily: "Inter, Roboto, 'Helvetica Neue', Arial, sans-serif",
  color: "#111827",
});

export const header = style({
  marginBottom: "20px",
});

export const heading = style({
  margin: "0 0 4px",
  fontSize: "1.6rem",
});

export const subtitle = style({
  margin: 0,
  color: "#6b7280",
});

export const message = style({
  color: "#6b7280",
});

export const section = style({
  border: "1px solid #e5e7eb",
  borderRadius: "12px",
  background: "#ffffff",
  padding: "20px",
  marginBottom: "20px",
});

export const sectionTitle = style({
  margin: "0 0 4px",
  fontSize: "1.15rem",
});

export const sectionDescription = style({
  margin: "0 0 16px",
  fontSize: "0.9rem",
  color: "#6b7280",
});

export const subsectionTitle = style({
  margin: "28px 0 4px",
  fontSize: "1rem",
});

export const fieldGrid = style({
  display: "grid",
  gap: "16px",
  maxWidth: "520px",
});

export const field = style({
  display: "grid",
  gap: "6px",
});

export const timezoneField = style([
  field,
  {
    scrollMarginTop: "calc(var(--site-header-height) + 16px)",
  },
]);

export const fieldLabel = style({
  fontSize: "0.9rem",
  fontWeight: 600,
  color: "#374151",
});

export const optionalTag = style({
  marginLeft: "6px",
  fontSize: "0.75rem",
  fontWeight: 500,
  color: "#9ca3af",
});

export const input = style({
  padding: "10px 12px",
  borderRadius: "8px",
  border: "1px solid #d1d5db",
  fontSize: "0.95rem",
  fontFamily: "inherit",
  selectors: {
    "&:disabled": {
      background: "#f3f4f6",
      color: "#6b7280",
    },
  },
});

export const fieldHelp = style({
  margin: 0,
  fontSize: "0.8rem",
  color: "#9ca3af",
});

export const keyInput = style({
  width: "100%",
  minHeight: "96px",
  padding: "8px 10px",
  fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
  fontSize: "0.8rem",
  border: "1px solid #d1d5db",
  borderRadius: "6px",
  boxSizing: "border-box",
});

export const setupGuide = style({
  maxWidth: "620px",
  margin: "0 0 16px",
  fontSize: "0.9rem",
  color: "#374151",
});

export const setupSummary = style({
  cursor: "pointer",
  fontWeight: 600,
});

export const setupSteps = style({
  display: "grid",
  gap: "8px",
  margin: "10px 0 0",
  paddingLeft: "22px",
});

export const selectTrigger = style([
  input,
  {
    display: "inline-flex",
    alignItems: "center",
    justifyContent: "space-between",
    gap: "8px",
    background: "#ffffff",
    color: "#111827",
    lineHeight: 1.2,
    textAlign: "left",
    cursor: "pointer",
    selectors: {
      "&:hover": {
        background: "#f9fafb",
      },
    },
  },
]);

/** Above the site header so the option list isn't hidden behind it. */
export const selectPositioner = style({
  zIndex: 60,
});

export const selectIndicator = style({
  color: "#6b7280",
  fontSize: "0.75rem",
  lineHeight: 1,
});

export const secretRow = style({
  display: "flex",
  gap: "8px",
  alignItems: "center",
});

export const maskedValue = style({
  flex: "1 1 auto",
  padding: "10px 12px",
  borderRadius: "8px",
  border: "1px solid #e5e7eb",
  background: "#f9fafb",
  color: "#6b7280",
  fontFamily: "monospace",
  fontSize: "0.95rem",
  letterSpacing: "0.04em",
});

export const inlineButton = style({
  flexShrink: 0,
  padding: "8px 12px",
  borderRadius: "8px",
  border: "1px solid #d1d5db",
  background: "white",
  color: "#374151",
  fontSize: "0.85rem",
  cursor: "pointer",
  selectors: {
    "&:hover": {
      background: "#f9fafb",
    },
  },
});

export const tabsRoot = style({
  width: "100%",
});

export const tabList = style({
  display: "flex",
  flexWrap: "wrap",
  gap: "4px",
  borderBottom: "1px solid #e5e7eb",
  marginBottom: "20px",
});

export const tabTrigger = style({
  display: "inline-flex",
  alignItems: "center",
  gap: "8px",
  padding: "8px 14px",
  border: "none",
  borderRadius: "8px 8px 0 0",
  background: "transparent",
  color: "#6b7280",
  fontFamily: "inherit",
  fontSize: "0.95rem",
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

export const tabConnected = style({
  fontSize: "0.7rem",
  fontWeight: 600,
  letterSpacing: "0.02em",
  color: "#15803d",
  background: "#dcfce7",
  padding: "2px 6px",
  borderRadius: "999px",
  lineHeight: 1.2,
});

export const integrationStatus = style({
  margin: "0 0 16px",
});

export const connectionStatus = style({
  margin: 0,
  fontSize: "0.9rem",
  fontWeight: 600,
  color: "#374151",
});

export const connectionStatusConnected = style({
  color: "#15803d",
});

export const actionsBar = style({
  display: "flex",
  alignItems: "center",
  gap: "16px",
  position: "sticky",
  bottom: 0,
  padding: "16px 0",
  background: "linear-gradient(to top, #ffffff 70%, rgba(255, 255, 255, 0))",
});

export const saveButton = style({
  padding: "10px 18px",
  border: 0,
  borderRadius: "8px",
  background: "#111827",
  color: "white",
  fontSize: "0.95rem",
  fontWeight: 600,
  cursor: "pointer",
  selectors: {
    "&:disabled": {
      opacity: 0.6,
      cursor: "not-allowed",
    },
  },
});

export const statusMessage = style({
  margin: 0,
  fontSize: "0.9rem",
});

export const statusError = style({
  color: "#dc2626",
});

export const statusSuccess = style({
  color: "#16a34a",
});

export const syncStatus = style({
  margin: "4px 0 0",
  fontSize: "0.85rem",
  color: "#6b7280",
});

export const syncStatusError = style({
  margin: "4px 0 0",
  fontSize: "0.85rem",
  color: "#dc2626",
});

export const dialogActions = style({
  display: "flex",
  flexWrap: "wrap",
  gap: "8px",
  marginTop: "16px",
});

export const dialogPrimary = style({
  padding: "9px 14px",
  border: 0,
  borderRadius: "8px",
  background: "#111827",
  color: "white",
  fontSize: "0.9rem",
  cursor: "pointer",
});

export const dialogSecondary = style({
  padding: "9px 14px",
  borderRadius: "8px",
  border: "1px solid #d1d5db",
  background: "white",
  color: "#374151",
  fontSize: "0.9rem",
  cursor: "pointer",
});

export const dialogDanger = style({
  padding: "9px 14px",
  borderRadius: "8px",
  border: "1px solid #fca5a5",
  background: "white",
  color: "#dc2626",
  fontSize: "0.9rem",
  cursor: "pointer",
});
