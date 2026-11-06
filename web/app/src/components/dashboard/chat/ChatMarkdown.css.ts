import { globalStyle, style } from "@vanilla-extract/css";

export const markdownParagraph = style({
  margin: "0 0 10px",
  lineHeight: 1.6,
  selectors: {
    "&:last-child": { marginBottom: 0 },
  },
});

export const markdownList = style({
  margin: "0 0 10px",
  paddingLeft: "20px",
  lineHeight: 1.6,
  selectors: {
    "&:last-child": { marginBottom: 0 },
  },
});

export const markdownCode = style({
  margin: "0 0 10px",
  padding: "10px",
  borderRadius: "8px",
  background: "#111827",
  color: "#e5e7eb",
  fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
  fontSize: "0.8rem",
  lineHeight: 1.5,
  whiteSpace: "pre-wrap",
  overflowX: "auto",
  selectors: {
    "&:last-child": { marginBottom: 0 },
  },
});

export const markdownInlineCode = style({
  padding: "1px 5px",
  borderRadius: "4px",
  background: "#f1f5f9",
  color: "#111827",
  fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
  fontSize: "0.85em",
});

/** A wide answer table scrolls sideways rather than stretching the thread. */
export const markdownTableScroller = style({
  width: "100%",
  margin: "0 0 10px",
  overflowX: "auto",
  selectors: {
    "&:last-child": { marginBottom: 0 },
  },
});

export const markdownTable = style({
  width: "100%",
  borderCollapse: "collapse",
  fontSize: "0.9rem",
  lineHeight: 1.5,
  border: "1px solid #e5e7eb",
});

globalStyle(`${markdownTable} th, ${markdownTable} td`, {
  padding: "8px 12px",
  borderBottom: "1px solid #e5e7eb",
  textAlign: "left",
  verticalAlign: "top",
});

globalStyle(`${markdownTable} thead th`, {
  background: "#f8fafc",
  fontWeight: 600,
  color: "#374151",
});

globalStyle(`${markdownTable} tbody tr:last-child td`, {
  borderBottom: "none",
});

export const markdownCellRight = style({
  textAlign: "right",
  fontVariantNumeric: "tabular-nums",
  whiteSpace: "nowrap",
});

export const markdownCellCenter = style({
  textAlign: "center",
});

// The cell rule above sets left alignment at higher specificity than a class
// on the cell, so the divider's alignment has to win the same way.
globalStyle(`${markdownTable} th${markdownCellRight}, ${markdownTable} td${markdownCellRight}`, {
  textAlign: "right",
  fontVariantNumeric: "tabular-nums",
  whiteSpace: "nowrap",
});

globalStyle(`${markdownTable} th${markdownCellCenter}, ${markdownTable} td${markdownCellCenter}`, {
  textAlign: "center",
});
