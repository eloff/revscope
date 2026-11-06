import { globalStyle, style } from "@vanilla-extract/css";

export const chatLayout = style({
  display: "grid",
  gridTemplateColumns: "220px minmax(0, 1fr)",
  gap: "20px",
  // A starting guess only: the component measures the space actually left in
  // the viewport and sets an exact height, so the thread scrolls and the page
  // does not.
  height: "min(62vh, 760px)",
  "@media": {
    "(max-width: 900px)": {
      gridTemplateColumns: "minmax(0, 1fr)",
    },
  },
});

export const conversationRail = style({
  display: "flex",
  flexDirection: "column",
  gap: "8px",
  overflowY: "auto",
  paddingRight: "4px",
  "@media": {
    "(max-width: 900px)": {
      display: "none",
    },
  },
});

export const railHeading = style({
  margin: "10px 0 2px",
  fontSize: "0.72rem",
  fontWeight: 700,
  letterSpacing: "0.06em",
  textTransform: "uppercase",
  color: "#6b7280",
});

export const newChatButton = style({
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  padding: "10px 14px",
  border: "1px solid #d1d5db",
  borderRadius: "8px",
  background: "white",
  color: "#374151",
  fontSize: "0.95rem",
  fontWeight: 600,
  cursor: "pointer",
  selectors: {
    "&:hover": { background: "#f9fafb" },
  },
});

export const conversationItem = style({
  display: "flex",
  alignItems: "center",
  gap: "4px",
  borderRadius: "8px",
  selectors: {
    "&:hover": { background: "#f1f5f9" },
  },
});

export const conversationItemActive = style({
  background: "#e0e7ff",
});

globalStyle(`${conversationItem} > button:first-child`, {
  flex: 1,
  minWidth: 0,
  padding: "8px 10px",
  border: "none",
  background: "none",
  color: "#111827",
  fontSize: "0.85rem",
  textAlign: "left",
  whiteSpace: "nowrap",
  overflow: "hidden",
  textOverflow: "ellipsis",
  cursor: "pointer",
});

export const conversationDelete = style({
  padding: "4px 8px",
  border: "none",
  background: "none",
  color: "#9ca3af",
  fontSize: "1rem",
  lineHeight: 1,
  cursor: "pointer",
  selectors: {
    "&:hover": { color: "#b91c1c" },
  },
});

export const thread = style({
  display: "flex",
  flexDirection: "column",
  minWidth: 0,
  minHeight: 0,
});

export const threadHeader = style({
  display: "flex",
  flexWrap: "wrap",
  alignItems: "center",
  justifyContent: "space-between",
  gap: "4px 12px",
  padding: "0 4px 8px",
});

export const threadHeaderOpen = style({
  borderBottom: "1px solid #e5e7eb",
});

export const threadHeaderText = style({
  display: "flex",
  flexWrap: "wrap",
  alignItems: "baseline",
  justifyContent: "space-between",
  gap: "4px 12px",
  minWidth: 0,
  flex: "1 1 auto",
});

export const threadDocsLink = style({
  marginLeft: "auto",
});

export const threadHeaderTitle = style({
  minWidth: 0,
  fontSize: "0.9rem",
  fontWeight: 600,
  color: "#111827",
  overflow: "hidden",
  textOverflow: "ellipsis",
  whiteSpace: "nowrap",
});

export const threadHeaderMeta = style({
  fontSize: "0.78rem",
  color: "#6b7280",
});

export const threadScroller = style({
  flex: 1,
  overflowY: "auto",
  padding: "4px 4px 16px",
});

export const emptyState = style({
  display: "flex",
  flexDirection: "column",
  alignItems: "flex-start",
  gap: "10px",
  maxWidth: "560px",
  margin: "48px auto 0",
  textAlign: "left",
});

export const emptyStateHint = style({
  padding: "10px 14px",
  border: "1px solid #e5e7eb",
  borderRadius: "10px",
  background: "#ffffff",
  color: "#2563eb",
  fontSize: "0.88rem",
  textAlign: "left",
  cursor: "pointer",
  selectors: {
    "&:hover": { background: "#f8fafc" },
  },
});

export const composer = style({
  paddingTop: "12px",
  borderTop: "1px solid #e5e7eb",
});

export const composerHint = style({
  margin: "0 0 8px",
  fontSize: "0.8rem",
  color: "#6b7280",
});

export const composerHintLink = style({
  color: "#2563eb",
  textDecoration: "none",
  selectors: {
    "&:hover": { textDecoration: "underline" },
  },
});

export const composerRow = style({
  display: "flex",
  alignItems: "flex-end",
  gap: "10px",
});

export const composerInput = style({
  flex: 1,
  padding: "10px 12px",
  border: "1px solid #d1d5db",
  borderRadius: "10px",
  fontFamily: "inherit",
  fontSize: "0.92rem",
  lineHeight: 1.5,
  resize: "vertical",
  selectors: {
    "&:focus": { outline: "2px solid #2563eb", outlineOffset: "-1px" },
  },
});

export const sendButton = style({
  padding: "10px 14px",
  border: 0,
  borderRadius: "8px",
  background: "#2563eb",
  color: "#ffffff",
  fontSize: "0.95rem",
  fontWeight: 600,
  cursor: "pointer",
  selectors: {
    "&:hover": { background: "#1d4ed8" },
    "&:disabled": { opacity: 0.7, cursor: "not-allowed" },
  },
});
