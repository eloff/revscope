import { globalStyle, style } from "@vanilla-extract/css";

export const heading = style({
  margin: "0 0 4px",
  fontSize: "1.1rem",
  fontWeight: 700,
  color: "#111827",
});

export const subheading = style({
  margin: "0 0 16px",
  color: "#6b7280",
  fontSize: "0.9rem",
});

/** Lets the wide table scroll sideways on narrow screens instead of the page. */
export const tableScroll = style({
  width: "100%",
  overflowX: "auto",
});

export const accountsTable = style({
  width: "100%",
  minWidth: "960px",
  borderCollapse: "collapse",
  fontSize: "0.85rem",
});

globalStyle(`${accountsTable} th, ${accountsTable} td`, {
  padding: "10px 12px",
  borderBottom: "1px solid #f1f5f9",
  textAlign: "left",
  verticalAlign: "top",
});

globalStyle(`${accountsTable} thead th`, {
  background: "#f8fafc",
  fontSize: "0.75rem",
  fontWeight: 700,
  color: "#4b5563",
  textTransform: "uppercase",
  letterSpacing: "0.02em",
  whiteSpace: "nowrap",
});

export const primaryText = style({
  display: "block",
  color: "#111827",
  fontWeight: 600,
  overflowWrap: "anywhere",
});

export const secondaryText = style({
  display: "block",
  color: "#6b7280",
  fontSize: "0.78rem",
  overflowWrap: "anywhere",
});

export const monoText = style([secondaryText, { fontFamily: "ui-monospace, monospace" }]);

export const stack = style({
  display: "flex",
  flexDirection: "column",
  gap: "10px",
  margin: 0,
  padding: 0,
  listStyle: "none",
});

export const tagRow = style({
  display: "flex",
  flexWrap: "wrap",
  alignItems: "center",
  gap: "6px",
});

export const unverified = style({
  color: "#b45309",
});

export const actionsCell = style({
  width: "1%",
  textAlign: "right",
});

export const menuTrigger = style({
  display: "inline-grid",
  placeItems: "center",
  width: "32px",
  height: "32px",
  padding: 0,
  border: "1px solid #e5e7eb",
  borderRadius: "8px",
  background: "#ffffff",
  color: "#374151",
  fontSize: "1.1rem",
  lineHeight: 1,
  cursor: "pointer",
  selectors: {
    "&:hover": { background: "#f3f4f6" },
    "&:disabled": { cursor: "not-allowed", opacity: 0.5 },
  },
});

export const dangerMenuItem = style({
  color: "#b91c1c",
});
