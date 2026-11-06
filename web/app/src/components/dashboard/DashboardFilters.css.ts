import { globalStyle, style } from "@vanilla-extract/css";

export const dateRangeCluster = style({
  display: "flex",
  alignItems: "stretch",
  selectors: {
    "&[data-disabled]": {
      opacity: 0.55,
      cursor: "not-allowed",
    },
  },
});

export const dateRangeDayCount = style({
  marginTop: "16px",
  paddingTop: "12px",
  borderTop: "1px solid #e5e7eb",
  fontSize: "0.88rem",
  fontWeight: 500,
  color: "#6b7280",
  whiteSpace: "nowrap",
  textAlign: "right",
});

export const dateRangeTimezone = style({
  fontSize: "0.88rem",
  fontWeight: 500,
  color: "#6b7280",
  whiteSpace: "nowrap",
  textDecoration: "underline",
  textUnderlineOffset: "2px",
  selectors: {
    "&:link, &:visited": {
      color: "#6b7280",
    },
    "&:hover": {
      color: "#111827",
    },
    "&:focus-visible": {
      outline: "2px solid #2563eb",
      outlineOffset: "2px",
      borderRadius: "2px",
    },
  },
});

export const filtersBar = style({
  display: "flex",
  flexWrap: "wrap",
  gap: "6px",
  alignItems: "center",
  padding: "10px 12px",
  marginBottom: "16px",
  border: "1px solid #e5e7eb",
  borderRadius: "12px",
  background: "#ffffff",
});

export const filtersBarBrandLink = style({
  display: "inline-flex",
  alignItems: "center",
  gap: "10px",
  marginRight: "4px",
  flexShrink: 0,
  color: "#374151",
  textDecoration: "none",
  fontSize: "0.88rem",
  fontWeight: 500,
  whiteSpace: "nowrap",
  selectors: {
    "&:hover": {
      color: "#111827",
    },
  },
});

export const filtersBarLogoImage = style({
  display: "block",
});

export const filtersBarAccount = style({
  marginLeft: "auto",
  display: "inline-flex",
  alignItems: "center",
  gap: "8px",
  flexShrink: 0,
});

export const segmentFiltersCluster = style({
  display: "flex",
  flexWrap: "wrap",
  alignItems: "center",
  gap: "6px",
});

const periodNavButtonBase = {
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  width: "40px",
  border: "1px solid #e5e7eb",
  background: "#fff",
  color: "#111827",
  fontSize: "2.1rem",
  lineHeight: 1,
  cursor: "pointer",
  padding: 0,
  boxSizing: "border-box" as const,
};

export const periodNavButtonPrev = style({
  ...periodNavButtonBase,
  alignSelf: "stretch",
  borderRadius: "10px 0 0 10px",
  borderRight: "none",
  selectors: {
    "&:disabled": {
      color: "#9ca3af",
      background: "#f9fafb",
      cursor: "not-allowed",
    },
  },
});

export const periodNavButtonNext = style({
  ...periodNavButtonBase,
  alignSelf: "stretch",
  borderRadius: "0 10px 10px 0",
  borderLeft: "none",
  selectors: {
    "&:disabled": {
      color: "#9ca3af",
      background: "#f9fafb",
      cursor: "not-allowed",
    },
  },
});

export const dateRangeShell = style({
  display: "flex",
  alignItems: "stretch",
  alignSelf: "stretch",
  minHeight: "40px",
  overflow: "hidden",
  border: "1px solid #e5e7eb",
  borderRadius: 0,
  background: "#fff",
  boxShadow: "0 1px 2px rgba(15, 23, 42, 0.04)",
  selectors: {
    [`${dateRangeCluster}[data-disabled] &`]: {
      background: "#f9fafb",
      boxShadow: "none",
      color: "#9ca3af",
    },
  },
});

export const datePickerControl = style({
  display: "flex",
  alignItems: "center",
  height: "100%",
});

export const dateInput = style({
  width: "11ch",
  minWidth: "104px",
  height: "100%",
  padding: "0 4px",
  border: "none",
  background: "transparent",
  boxSizing: "border-box",
  fontSize: "1.08rem",
  textAlign: "center",
  color: "#111827",
  outline: "none",
  selectors: {
    "&:focus": {
      boxShadow: "inset 0 0 0 2px #bfdbfe",
    },
    [`${dateRangeCluster}[data-disabled] &`]: {
      color: "#9ca3af",
      cursor: "not-allowed",
    },
    [`${dateRangeCluster}[data-disabled] &:focus`]: {
      boxShadow: "none",
    },
  },
});

globalStyle(`${dateInput} input`, {
  width: "100%",
  padding: 0,
  border: "none",
  background: "transparent",
  fontSize: "inherit",
  textAlign: "inherit",
  color: "inherit",
  outline: "none",
});

export const dateRangeDivider = style({
  display: "block",
  flexShrink: 0,
  width: "24px",
  paddingRight: "32px",
  boxSizing: "border-box",
  color: "#6b7280",
  fontSize: "2.1rem",
  lineHeight: 1,
  textAlign: "center",
  transform: "translateY(-0.16em)",
  pointerEvents: "none",
  selectors: {
    [`${dateRangeCluster}[data-disabled] &`]: {
      color: "#d1d5db",
    },
  },
});

export const periodNavIcon = style({
  display: "block",
  lineHeight: 1,
  transform: "translateY(-0.06em)",
});

export const dateContent = style({
  background: "#fff",
  border: "1px solid #e5e7eb",
  borderRadius: "16px",
  boxShadow: "0 16px 48px rgba(0, 0, 0, 0.18)",
  padding: "20px",
  zIndex: 60,
});

export const dateContentLayout = style({
  display: "flex",
  alignItems: "stretch",
  gap: "20px",
});

export const datePresetList = style({
  display: "flex",
  flexDirection: "column",
  gap: "2px",
  minWidth: "148px",
  paddingRight: "20px",
  borderRight: "1px solid #e5e7eb",
});

export const datePresetButton = style({
  appearance: "none",
  display: "block",
  padding: "8px 12px",
  border: "none",
  borderRadius: "8px",
  background: "transparent",
  color: "#374151",
  font: "inherit",
  fontSize: "0.9rem",
  fontWeight: 500,
  textAlign: "left",
  whiteSpace: "nowrap",
  cursor: "pointer",
  selectors: {
    "&:hover": {
      background: "#f3f4f6",
      color: "#111827",
    },
    '&[data-selected], &[data-selected=""]': {
      background: "#111827",
      color: "#fff",
    },
  },
});

export const calendarMonths = style({
  position: "relative",
  display: "flex",
  gap: "36px",
});

const calendarNavBase = {
  position: "absolute" as const,
  top: 0,
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  width: "34px",
  height: "34px",
  padding: 0,
  border: "1px solid #e5e7eb",
  borderRadius: "999px",
  background: "#fff",
  color: "#111827",
  fontSize: "1.35rem",
  lineHeight: 1,
  cursor: "pointer",
  zIndex: 1,
};

export const calendarNavPrev = style({
  ...calendarNavBase,
  left: 0,
  selectors: {
    "&:hover:not(:disabled)": {
      background: "#f3f4f6",
    },
    "&:disabled": {
      color: "#d1d5db",
      cursor: "not-allowed",
    },
  },
});

export const calendarNavNext = style({
  ...calendarNavBase,
  right: 0,
  selectors: {
    "&:hover:not(:disabled)": {
      background: "#f3f4f6",
    },
    "&:disabled": {
      color: "#d1d5db",
      cursor: "not-allowed",
    },
  },
});

export const calendarMonth = style({
  display: "flex",
  flexDirection: "column",
  gap: "10px",
});

export const calendarMonthLabel = style({
  height: "34px",
  lineHeight: "34px",
  textAlign: "center",
  fontSize: "1rem",
  fontWeight: 600,
  color: "#111827",
});

export const calendarGrid = style({
  borderCollapse: "collapse",
});

globalStyle(`${calendarGrid} th`, {
  width: "40px",
  height: "32px",
  color: "#6b7280",
  fontWeight: 500,
  fontSize: "0.75rem",
  textAlign: "center",
});

globalStyle(`${calendarGrid} td`, {
  padding: 0,
  textAlign: "center",
});

/* Ark's TableCellTrigger renders a div with role="button", not a <button>. */
const dayCell = `${calendarGrid} td [data-part="table-cell-trigger"]`;

globalStyle(dayCell, {
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  width: "40px",
  height: "40px",
  padding: 0,
  border: "none",
  borderRadius: "999px",
  background: "transparent",
  color: "#111827",
  fontSize: "0.9rem",
  fontVariantNumeric: "tabular-nums",
  cursor: "pointer",
  userSelect: "none",
});

globalStyle(`${dayCell}[data-today]`, {
  fontWeight: 700,
});

globalStyle(`${dayCell}:hover:not([data-disabled]):not([data-outside-range])`, {
  boxShadow: "inset 0 0 0 1.5px #111827",
});

/* Range band: squared light background between endpoints (selected or hover preview). */
globalStyle(`${dayCell}[data-in-range], ${dayCell}[data-in-hover-range]`, {
  background: "#f3f4f6",
  borderRadius: 0,
});

globalStyle(`${dayCell}[data-range-start], ${dayCell}[data-hover-range-start]`, {
  borderRadius: "999px 0 0 999px",
});

globalStyle(`${dayCell}[data-range-end], ${dayCell}[data-hover-range-end]`, {
  borderRadius: "0 999px 999px 0",
});

globalStyle(
  `${dayCell}[data-range-start][data-range-end], ${dayCell}[data-hover-range-start][data-hover-range-end]`,
  {
    borderRadius: "999px",
  },
);

/* Selected endpoints: solid dark circles on top of the band. */
globalStyle(`${dayCell}[data-selected]`, {
  background: "#111827",
  color: "#fff",
  borderRadius: "999px",
});

globalStyle(`${dayCell}[data-disabled]`, {
  color: "#d1d5db",
  cursor: "not-allowed",
});

/* Hide adjacent-month days so each grid only shows its own month. */
globalStyle(`${dayCell}[data-outside-range]`, {
  visibility: "hidden",
});
