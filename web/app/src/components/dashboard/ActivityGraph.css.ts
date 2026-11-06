import { style } from "@vanilla-extract/css";

export const activityCard = style({
  display: "flex",
  flexDirection: "column",
  alignItems: "center",
  background: "#ffffff",
  border: "1px solid #e5e7eb",
  borderRadius: "14px",
  padding: "20px",
});

export const activityContent = style({
  display: "flex",
  flexDirection: "column",
  gap: "16px",
  width: "fit-content",
  maxWidth: "100%",
});

export const activityHeader = style({
  display: "flex",
  flexWrap: "wrap",
  alignItems: "center",
  justifyContent: "space-between",
  gap: "12px",
});

export const activityTitle = style({
  margin: 0,
  fontSize: "18.75px",
  fontWeight: 600,
  color: "#111827",
});

export const activitySubtitle = style({
  margin: "2px 0 0",
  fontSize: "16.25px",
  color: "#6b7280",
});

export const toggleRow = style({
  display: "flex",
  flexWrap: "wrap",
  gap: "8px",
});

export const graphWithSidebar = style({
  display: "flex",
  alignItems: "flex-start",
  gap: "16px",
});

export const graphScroll = style({
  flex: "1 1 auto",
  minWidth: 0,
  overflowX: "auto",
  paddingBottom: "4px",
});

export const categoryToggleColumn = style({
  display: "flex",
  flexDirection: "column",
  gap: "10px",
  flexShrink: 0,
  paddingTop: "18px",
});

export const categoryCheckboxRow = style({
  display: "flex",
  alignItems: "center",
  gap: "8px",
  color: "#374151",
  fontSize: "13px",
  fontWeight: 500,
  lineHeight: 1.3,
  cursor: "pointer",
  userSelect: "none",
  whiteSpace: "nowrap",
});

export const categoryCheckboxInput = style({
  width: "15px",
  height: "15px",
  margin: 0,
  flexShrink: 0,
  cursor: "pointer",
  accentColor: "#16a34a",
});

export const graphHost = style({
  position: "relative",
});

export const hoverActivityTable = style({
  width: "100%",
  borderCollapse: "collapse",
  fontSize: "inherit",
});

export const hoverActivityTableCell = style({
  padding: "2px 0",
  font: "inherit",
});

export const hoverActivityTableCorner = style([
  hoverActivityTableCell,
  {
    paddingBottom: "4px",
  },
]);

export const hoverActivityTableMetricHeader = style([
  hoverActivityTableCell,
  {
    fontWeight: 500,
    color: "#4b5563",
    textAlign: "right",
    whiteSpace: "nowrap",
    paddingBottom: "4px",
    paddingLeft: "16px",
  },
]);

export const hoverActivityTableRowLabel = style([
  hoverActivityTableCell,
  {
    fontWeight: 500,
    color: "#4b5563",
    textAlign: "left",
    paddingRight: "12px",
    verticalAlign: "baseline",
  },
]);

export const hoverActivityTableValue = style([
  hoverActivityTableCell,
  {
    fontWeight: 600,
    color: "#111827",
    textAlign: "right",
    whiteSpace: "nowrap",
    paddingLeft: "16px",
    verticalAlign: "baseline",
  },
]);

export const hoverActivityTableBreakdownStart = style([
  hoverActivityTableCell,
  {
    paddingTop: "8px",
    borderTop: "1px solid #e5e7eb",
  },
]);

export const hoverBreakdownLabel = style({
  color: "#6b7280",
});

export const graphLayout = style({
  display: "inline-flex",
  flexDirection: "column",
  gap: "4px",
  minWidth: "min-content",
});

export const monthRow = style({
  display: "flex",
  marginLeft: "30px",
  height: "14px",
  position: "relative",
});

export const monthLabel = style({
  position: "absolute",
  fontSize: "11px",
  color: "#6b7280",
  whiteSpace: "nowrap",
});

export const gridBody = style({
  display: "flex",
  gap: "3px",
});

export const weekdayColumn = style({
  display: "flex",
  flexDirection: "column",
  gap: "3px",
  width: "27px",
  flexShrink: 0,
  marginRight: "0px",
});

export const weekdayLabel = style({
  height: "15px",
  fontSize: "10px",
  lineHeight: "15px",
  color: "#9ca3af",
});

export const weeksRow = style({
  display: "flex",
  gap: "3px",
});

export const weekColumn = style({
  display: "flex",
  flexDirection: "column",
  gap: "3px",
});

export const dayCell = style({
  width: "15px",
  height: "15px",
  borderRadius: "3px",
  outline: "1px solid rgba(27, 31, 35, 0.04)",
  outlineOffset: "-1px",
});

export const dayCellEmpty = style({
  background: "transparent",
  outline: "none",
});

export const legendRow = style({
  display: "flex",
  alignItems: "center",
  justifyContent: "flex-end",
  gap: "6px",
  marginTop: "10px",
  fontSize: "11px",
  color: "#6b7280",
});

export const legendSwatches = style({
  display: "flex",
  gap: "3px",
});

export const legendSwatch = style({
  width: "15px",
  height: "15px",
  borderRadius: "3px",
  outline: "1px solid rgba(27, 31, 35, 0.04)",
  outlineOffset: "-1px",
});
