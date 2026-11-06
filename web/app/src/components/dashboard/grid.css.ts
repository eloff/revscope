import { globalStyle, style } from "@vanilla-extract/css";

export const gridIconLabelCell = style({
  display: "inline-flex",
  alignItems: "center",
  gap: "8px",
});

export const gridShell = style({
  display: "flex",
  flexDirection: "column",
  gap: "12px",
  width: "100%",
});

export const clickableRow = style({
  cursor: "pointer",
  selectors: {
    "&:focus-visible": {
      outline: "2px solid #2563eb",
      outlineOffset: "-2px",
    },
  },
});

export const gridTable = style({
  width: "100%",
  borderCollapse: "collapse",
  fontSize: "0.9rem",
  background: "#ffffff",
  border: "1px solid #e5e7eb",
  borderRadius: "10px",
  overflow: "hidden",
});

globalStyle(`${gridTable} th, ${gridTable} td`, {
  padding: "10px 12px",
  borderBottom: "1px solid #f1f5f9",
  textAlign: "left",
  verticalAlign: "top",
});

globalStyle(`${gridTable} thead th`, {
  background: "#f8fafc",
  fontSize: "0.78rem",
  fontWeight: 700,
  color: "#4b5563",
  textTransform: "uppercase",
  letterSpacing: "0.02em",
  padding: 0,
});

export const gridSummaryRow = style({});

globalStyle(`${gridTable} thead tr${gridSummaryRow} th`, {
  padding: "10px 12px",
  background: "#ffffff",
  borderTop: "1px solid #e5e7eb",
  color: "#111827",
  fontSize: "0.82rem",
  fontWeight: 700,
  letterSpacing: "normal",
  textTransform: "none",
});

globalStyle(`${gridTable} thead tr${gridSummaryRow} td`, {
  background: "#ffffff",
  borderTop: "1px solid #e5e7eb",
  color: "#111827",
  fontSize: "0.82rem",
  fontWeight: 700,
});

export const gridSortHeader = style({
  display: "inline-flex",
  alignItems: "center",
  gap: "4px",
  width: "100%",
  padding: "10px 12px",
  margin: 0,
  border: "none",
  background: "transparent",
  font: "inherit",
  color: "inherit",
  textTransform: "inherit",
  letterSpacing: "inherit",
  textAlign: "left",
  cursor: "pointer",
  selectors: {
    "&:hover": {
      color: "#111827",
    },
  },
});

export const gridHeaderLabel = style({
  display: "inline-flex",
  alignItems: "center",
  padding: "10px 12px",
});

export const gridSortHeaderRight = style([
  gridSortHeader,
  {
    justifyContent: "flex-end",
    textAlign: "right",
    padding: "10px 0 10px 12px",
  },
]);

export const gridSortIndicator = style({
  display: "inline-block",
  fontSize: "0.72rem",
  lineHeight: 1,
  color: "#2563eb",
});

export const gridSortHeaderCell = style({
  display: "inline-flex",
  alignItems: "center",
  gap: 0,
  width: "100%",
});

export const gridSortHeaderCellRight = style([
  gridSortHeaderCell,
  {
    justifyContent: "flex-end",
  },
]);

globalStyle(`${gridSortHeaderCell} button`, {
  width: "auto",
});

globalStyle(`${gridTable} tbody tr[data-record-key] td`, {
  borderBottom: "1px solid var(--row-border)",
});

globalStyle(`${gridTable} tbody tr:hover td`, {
  background: "#f8fafc",
});

export const gridRowHighlighted = style({});

globalStyle(`${gridTable} tbody tr${gridRowHighlighted} td`, {
  background: "#fef9c3",
});

export const gridExpandedRow = style({});

export const gridExpandedCell = style({
  background: "#f8fafc",
});

export const gridSubtableShell = style({
  position: "relative",
  zIndex: 1,
  padding: "12px",
  border: "1px solid #e5e7eb",
  borderRadius: "10px",
  background: "#ffffff",
});

globalStyle(`${gridTable} tbody tr${gridExpandedRow}:hover > td`, {
  background: "#f8fafc",
});

globalStyle(`${gridTable} tbody tr${gridExpandedRow} > td${gridExpandedCell}`, {
  padding: "14px 18px 18px",
  background: "#f8fafc",
});

globalStyle(`${gridSubtableShell} ${gridTable}`, {
  overflow: "visible",
});

globalStyle(`${gridSubtableShell} ${gridTable} tbody tr td`, {
  background: "#ffffff",
});

globalStyle(`${gridSubtableShell} ${gridTable} tbody tr:hover td`, {
  background: "#f8fafc",
});

globalStyle(`${gridTable} tbody tr:last-child td`, {
  borderBottom: "none",
});

export const gridGroupBadge = style({
  display: "inline-block",
  padding: "2px 8px",
  borderRadius: "999px",
  fontSize: "0.72rem",
  fontWeight: 700,
  letterSpacing: "0.02em",
  whiteSpace: "nowrap",
});

export const gridReferrerCell = style({
  maxWidth: "240px",
  overflowWrap: "anywhere",
  color: "#111827",
  fontWeight: 500,
});

export const gridLandingPathsCell = style({
  maxWidth: "320px",
  color: "#4b5563",
  fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
  fontSize: "0.82rem",
});

export const gridLandingPathsText = style({
  display: "block",
  maxWidth: "320px",
  overflow: "hidden",
  textOverflow: "ellipsis",
  whiteSpace: "nowrap",
});

export const gridInlineActionLabel = style({
  minWidth: 0,
  overflow: "hidden",
  textOverflow: "ellipsis",
});

export const gridFilterActionCell = style({
  width: "1%",
  whiteSpace: "nowrap",
  textAlign: "right",
});

export const gridFilterButton = style({
  appearance: "none",
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  width: "26px",
  height: "26px",
  padding: 0,
  border: "1px solid transparent",
  borderRadius: "8px",
  background: "transparent",
  color: "#6b7280",
  cursor: "pointer",
  transition: "background 0.15s ease, border-color 0.15s ease, color 0.15s ease",
  selectors: {
    "&:hover": {
      background: "#eff6ff",
      borderColor: "#bfdbfe",
      color: "#2563eb",
    },
    "&:focus-visible": {
      color: "#2563eb",
      outline: "2px solid #93c5fd",
      outlineOffset: "2px",
    },
  },
});

export const gridFilterButtonActive = style({
  background: "#eff6ff",
  borderColor: "#bfdbfe",
  color: "#2563eb",
  selectors: {
    "&:hover": {
      background: "#dbeafe",
      borderColor: "#93c5fd",
      color: "#1d4ed8",
    },
  },
});

export const gridNumberCell = style({
  textAlign: "right",
  fontVariantNumeric: "tabular-nums",
  whiteSpace: "nowrap",
});

/** Metric value immediately left of a change cell (no gap between columns). */
export const gridMetricValueCell = style([
  gridNumberCell,
  {
    paddingRight: 0,
    width: "1%",
  },
]);

globalStyle(`${gridTable} th${gridNumberCell}, ${gridTable} td${gridNumberCell}`, {
  textAlign: "right",
  fontVariantNumeric: "tabular-nums",
  whiteSpace: "nowrap",
});

globalStyle(`${gridTable} td${gridMetricValueCell}`, {
  textAlign: "right",
  fontVariantNumeric: "tabular-nums",
  whiteSpace: "nowrap",
  paddingRight: 0,
  width: "1%",
});

export const gridBestMetricValue = style({
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "flex-end",
  gap: "4px",
  width: "100%",
});

export const gridBestMetricValueStart = style({
  justifyContent: "flex-start",
});

export const gridBestStar = style({
  color: "#ca8a04",
  fontSize: "1.7em",
  lineHeight: 1,
});

export const gridChangePositive = style({
  color: "#15803d",
  fontWeight: 600,
});

export const gridChangeNegative = style({
  color: "#b91c1c",
  fontWeight: 600,
});

export const gridChangeNeutral = style({
  color: "#6b7280",
});
