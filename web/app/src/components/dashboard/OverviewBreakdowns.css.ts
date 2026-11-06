import { style } from "@vanilla-extract/css";
import { gridSortHeader, gridSortHeaderRight } from "./grid.css";

const VISITORS_COLOR = "#dbeafe";
const REVENUE_COLOR = "#f59e0b";
const COLUMNS = "minmax(0, 1fr) 64px 72px";

export const breakdownGrid = style({
  display: "grid",
  gridTemplateColumns: "repeat(2, minmax(0, 1fr))",
  gap: "16px",
  width: "100%",
  marginTop: "20px",
  "@media": {
    "screen and (max-width: 960px)": {
      gridTemplateColumns: "1fr",
    },
  },
});

export const breakdownHeader = style({
  display: "flex",
  alignItems: "flex-end",
  justifyContent: "space-between",
  gap: "8px",
  borderBottom: "1px solid #e5e7eb",
});

export const breakdownTabList = style({
  display: "flex",
  gap: "4px",
  padding: "8px 8px 0",
  minWidth: 0,
});

export const breakdownColumns = style({
  display: "grid",
  gridTemplateColumns: COLUMNS,
  gap: "8px",
  padding: "8px 12px 4px",
  color: "#6b7280",
  fontSize: "0.75rem",
  fontWeight: 600,
  textTransform: "uppercase",
  letterSpacing: "0.03em",
});

export const breakdownSortHeader = style([gridSortHeader, { padding: 0, minWidth: 0 }]);

export const breakdownSortHeaderRight = style([gridSortHeaderRight, { padding: 0 }]);

export const breakdownRows = style({
  display: "flex",
  flexDirection: "column",
  gap: "2px",
  height: "340px",
  overflowY: "auto",
  padding: "0 6px 8px",
});

export const breakdownRow = style({
  position: "relative",
  display: "grid",
  gridTemplateColumns: COLUMNS,
  alignItems: "center",
  gap: "8px",
  width: "100%",
  minHeight: "32px",
  flexShrink: 0,
  padding: "0 6px",
  border: "none",
  borderRadius: "6px",
  background: "transparent",
  color: "#111827",
  fontFamily: "inherit",
  fontSize: "0.88rem",
  textAlign: "left",
  cursor: "pointer",
  overflow: "hidden",
  // The bars sit at z-index -1 within the row so they paint under the
  // selected outline; positioned children otherwise paint over its edges.
  isolation: "isolate",
  selectors: {
    "&:hover": {
      background: "#f8fafc",
    },
    '&[aria-pressed="true"]': {
      outline: "1.5px solid #2563eb",
      outlineOffset: "-1.5px",
    },
  },
});

export const breakdownVisitorsBar = style({
  position: "absolute",
  top: "2px",
  bottom: "2px",
  left: 0,
  zIndex: -1,
  borderRadius: "5px",
  background: VISITORS_COLOR,
  pointerEvents: "none",
});

export const breakdownRevenueBar = style({
  position: "absolute",
  bottom: "2px",
  left: 0,
  height: "3px",
  zIndex: -1,
  borderRadius: "2px",
  background: REVENUE_COLOR,
  pointerEvents: "none",
});

export const breakdownLabel = style({
  position: "relative",
  display: "flex",
  alignItems: "center",
  gap: "8px",
  minWidth: 0,
  overflow: "hidden",
  textOverflow: "ellipsis",
  whiteSpace: "nowrap",
});

export const breakdownLabelText = style({
  overflow: "hidden",
  textOverflow: "ellipsis",
});

export const breakdownFlag = style({
  flexShrink: 0,
  transform: "scale(0.6)",
  margin: "-8px -6px",
});

export const breakdownFilterHint = style({
  flexShrink: 0,
  color: "#2563eb",
  fontSize: "0.75rem",
  fontWeight: 600,
  opacity: 0,
  selectors: {
    [`${breakdownRow}:hover &`]: {
      opacity: 1,
    },
    [`${breakdownRow}:focus-visible &`]: {
      opacity: 1,
    },
  },
});

export const breakdownNumber = style({
  position: "relative",
  textAlign: "right",
  fontVariantNumeric: "tabular-nums",
  fontWeight: 600,
});

export const breakdownRevenueNumber = style([
  breakdownNumber,
  {
    color: "#b45309",
  },
]);

export const breakdownEmpty = style({
  padding: "24px 12px",
  color: "#6b7280",
  fontSize: "0.88rem",
  textAlign: "center",
});

export const breakdownLegend = style({
  display: "flex",
  gap: "16px",
  padding: "0 12px 10px",
  color: "#6b7280",
  fontSize: "0.75rem",
});

export const breakdownLegendSwatch = style({
  display: "inline-block",
  width: "10px",
  height: "10px",
  marginRight: "6px",
  borderRadius: "2px",
  verticalAlign: "-1px",
});

export const breakdownLegendVisitors = style([
  breakdownLegendSwatch,
  { background: VISITORS_COLOR },
]);

export const breakdownLegendRevenue = style([breakdownLegendSwatch, { background: REVENUE_COLOR }]);
