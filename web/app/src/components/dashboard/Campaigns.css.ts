import { globalStyle, style } from "@vanilla-extract/css";
import { gridTable } from "./grid.css";

export const campaignsShell = style({
  display: "flex",
  flexDirection: "column",
  gap: "18px",
});

export const campaignsHeader = style({
  display: "flex",
  alignItems: "flex-start",
  justifyContent: "space-between",
  gap: "16px",
  flexWrap: "wrap",
});

export const campaignsTitle = style({
  margin: 0,
  color: "#111827",
  fontSize: "1.25rem",
  lineHeight: 1.3,
});

export const campaignsDescription = style({
  maxWidth: "640px",
  margin: "4px 0 0",
  color: "#6b7280",
  fontSize: "0.9rem",
  lineHeight: 1.5,
});

export const mutedText = style({
  margin: 0,
  color: "#6b7280",
  fontSize: "0.86rem",
  lineHeight: 1.45,
});

const buttonBase = style({
  display: "inline-flex",
  alignItems: "center",
  gap: "6px",
  padding: "8px 12px",
  border: "1px solid #d1d5db",
  borderRadius: "8px",
  background: "#ffffff",
  color: "#111827",
  font: "inherit",
  fontSize: "0.88rem",
  fontWeight: 600,
  whiteSpace: "nowrap",
  cursor: "pointer",
  selectors: {
    "&:disabled": {
      color: "#9ca3af",
      background: "#f9fafb",
      cursor: "not-allowed",
    },
    "&:hover:not(:disabled)": {
      background: "#f3f4f6",
    },
  },
});

export const secondaryButton = buttonBase;

export const primaryButton = style([
  buttonBase,
  {
    borderColor: "#2563eb",
    background: "#2563eb",
    color: "#ffffff",
    selectors: {
      "&:hover:not(:disabled)": {
        background: "#1d4ed8",
      },
      "&:disabled": {
        borderColor: "#93c5fd",
        background: "#93c5fd",
        color: "#ffffff",
      },
    },
  },
]);

export const dangerButton = style([
  buttonBase,
  {
    borderColor: "#fecaca",
    color: "#b91c1c",
    selectors: {
      "&:hover:not(:disabled)": {
        background: "#fef2f2",
      },
    },
  },
]);

export const suggestionsCard = style({
  display: "flex",
  flexDirection: "column",
  gap: "10px",
  padding: "14px 16px",
  border: "1px solid #e5e7eb",
  borderRadius: "12px",
  background: "#ffffff",
});

export const suggestionsTitle = style({
  margin: 0,
  color: "#111827",
  fontSize: "0.9rem",
  fontWeight: 700,
});

export const suggestionList = style({
  display: "flex",
  flexWrap: "wrap",
  gap: "8px",
  margin: 0,
  padding: 0,
  listStyle: "none",
});

export const suggestionChip = style({
  display: "inline-flex",
  alignItems: "center",
  gap: "8px",
  padding: "5px 6px 5px 10px",
  border: "1px solid #e5e7eb",
  borderRadius: "999px",
  background: "#f8fafc",
  color: "#111827",
  font: "inherit",
  fontSize: "0.84rem",
  cursor: "pointer",
  transition: "border-color 0.15s ease, background 0.15s ease",
  selectors: {
    "&:hover": {
      borderColor: "#93c5fd",
      background: "#eff6ff",
    },
    "&:focus-visible": {
      outline: "2px solid #2563eb",
      outlineOffset: "2px",
    },
  },
});

export const paramKey = style({
  color: "#6b7280",
  fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
  fontSize: "0.8rem",
});

export const paramValue = style({
  fontWeight: 600,
});

export const countPill = style({
  padding: "1px 8px",
  borderRadius: "999px",
  background: "#e5e7eb",
  color: "#374151",
  fontSize: "0.76rem",
  fontWeight: 700,
  fontVariantNumeric: "tabular-nums",
});

export const tableScroller = style({
  width: "100%",
  overflowX: "auto",
});

export const textHeader = style({
  display: "block",
  padding: "10px 12px",
  whiteSpace: "nowrap",
  textAlign: "left",
});

export const metricHeader = style({
  display: "block",
  padding: "10px 12px",
  whiteSpace: "nowrap",
  textAlign: "right",
});

export const metricHeaderWithIcon = style({
  display: "flex",
  alignItems: "center",
  justifyContent: "flex-end",
  gap: "4px",
  padding: "10px 12px",
  whiteSpace: "nowrap",
});

export const chevronCell = style({
  width: "20px",
  color: "#9ca3af",
});

globalStyle(`${gridTable} td${chevronCell}`, {
  paddingRight: 0,
});

globalStyle(`${gridTable} th${chevronCell}`, {
  paddingRight: 0,
});

export const chevronButton = style({
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  width: "22px",
  height: "22px",
  padding: 0,
  border: "none",
  borderRadius: "6px",
  background: "transparent",
  color: "inherit",
  cursor: "pointer",
  selectors: {
    "&:focus-visible": {
      outline: "2px solid #2563eb",
      outlineOffset: "1px",
    },
  },
});

export const chevron = style({
  display: "inline-block",
  fontSize: "0.7rem",
  transition: "transform 0.15s ease",
  selectors: {
    "&[data-open='true']": {
      transform: "rotate(90deg)",
      color: "#2563eb",
    },
  },
});

export const nameCell = style({
  minWidth: "220px",
});

export const campaignNameText = style({
  display: "block",
  color: "#111827",
  fontWeight: 600,
});

export const campaignSubtitle = style({
  display: "block",
  marginTop: "2px",
  color: "#6b7280",
  fontSize: "0.8rem",
});

export const placeholderValue = style({
  color: "#9ca3af",
});

export const dateCell = style({
  color: "#4b5563",
  whiteSpace: "nowrap",
});

export const detailsLayout = style({
  display: "flex",
  flexDirection: "column",
  gap: "14px",
});

export const detailsCard = style({
  padding: "12px 14px",
  border: "1px solid #e5e7eb",
  borderRadius: "10px",
  background: "#ffffff",
});

export const sectionLabel = style({
  margin: "0 0 8px",
  color: "#6b7280",
  fontSize: "0.74rem",
  fontWeight: 700,
  letterSpacing: "0.04em",
  textTransform: "uppercase",
});

export const conditionList = style({
  display: "flex",
  flexWrap: "wrap",
  alignItems: "center",
  gap: "8px",
});

export const conditionChip = style({
  display: "inline-flex",
  alignItems: "center",
  gap: "6px",
  padding: "4px 10px",
  border: "1px solid #dbeafe",
  borderRadius: "8px",
  background: "#eff6ff",
  color: "#1e3a8a",
  fontSize: "0.86rem",
});

export const conditionOperator = style({
  color: "#6b7280",
  fontSize: "0.8rem",
});

export const logicPill = style({
  padding: "2px 8px",
  borderRadius: "999px",
  background: "#f3f4f6",
  color: "#4b5563",
  fontSize: "0.72rem",
  fontWeight: 700,
  letterSpacing: "0.04em",
});

export const ruleNote = style([mutedText, { marginTop: "10px" }]);

export const breakdownGrid = style({
  display: "grid",
  gridTemplateColumns: "repeat(auto-fit, minmax(200px, 1fr))",
  gap: "12px",
});

export const breakdownList = style({
  display: "flex",
  flexDirection: "column",
  gap: "4px",
  margin: 0,
  padding: 0,
  listStyle: "none",
});

export const breakdownItem = style({
  position: "relative",
  display: "flex",
  justifyContent: "space-between",
  gap: "12px",
  padding: "4px 8px",
  borderRadius: "6px",
  color: "#374151",
  fontSize: "0.86rem",
  overflow: "hidden",
});

export const breakdownBar = style({
  position: "absolute",
  inset: "0 auto 0 0",
  background: "#eff6ff",
  pointerEvents: "none",
});

export const breakdownLabel = style({
  position: "relative",
  minWidth: 0,
  overflow: "hidden",
  textOverflow: "ellipsis",
  whiteSpace: "nowrap",
});

export const breakdownCount = style({
  position: "relative",
  color: "#111827",
  fontWeight: 600,
  fontVariantNumeric: "tabular-nums",
});

export const detailsActions = style({
  display: "flex",
  flexWrap: "wrap",
  gap: "8px",
});

export const editor = style({
  display: "flex",
  flexDirection: "column",
  gap: "16px",
  padding: "18px",
  border: "1px solid #e5e7eb",
  borderRadius: "12px",
  background: "#ffffff",
  boxShadow: "0 1px 2px rgba(15, 23, 42, 0.04)",
});

export const editorTitle = style({
  margin: 0,
  color: "#111827",
  fontSize: "1.05rem",
});

export const field = style({
  display: "flex",
  flexDirection: "column",
  gap: "6px",
  maxWidth: "360px",
});

export const fieldLabel = style({
  color: "#374151",
  fontSize: "0.86rem",
  fontWeight: 600,
});

export const textInput = style({
  width: "100%",
  boxSizing: "border-box",
  border: "1px solid #d1d5db",
  borderRadius: "8px",
  background: "#ffffff",
  color: "#111827",
  font: "inherit",
  fontSize: "0.9rem",
  padding: "8px 10px",
  selectors: {
    "&:focus": {
      outline: "2px solid #bfdbfe",
      borderColor: "#2563eb",
    },
  },
});

export const conditionsBlock = style({
  display: "flex",
  flexDirection: "column",
  gap: "8px",
});

export const ruleRow = style({
  display: "grid",
  gridTemplateColumns: "170px auto minmax(160px, 320px) auto",
  justifyContent: "start",
  alignItems: "center",
  gap: "10px",
});

export const equalsText = style({
  color: "#6b7280",
  fontSize: "0.86rem",
});

export const removeButton = style({
  width: "32px",
  height: "32px",
  padding: 0,
  border: "1px solid transparent",
  borderRadius: "8px",
  background: "transparent",
  color: "#6b7280",
  font: "inherit",
  fontSize: "1.1rem",
  lineHeight: 1,
  cursor: "pointer",
  selectors: {
    "&:hover:not(:disabled)": {
      borderColor: "#fecaca",
      background: "#fef2f2",
      color: "#b91c1c",
    },
    "&:disabled": {
      visibility: "hidden",
    },
  },
});

export const orDivider = style({
  display: "flex",
  alignItems: "center",
  gap: "10px",
  maxWidth: "620px",
  color: "#6b7280",
  fontSize: "0.72rem",
  fontWeight: 700,
  letterSpacing: "0.06em",
  selectors: {
    "&::before": {
      content: "''",
      flex: "0 0 24px",
      borderTop: "1px dashed #d1d5db",
    },
    "&::after": {
      content: "''",
      flex: "1 1 auto",
      borderTop: "1px dashed #d1d5db",
    },
  },
});

export const addConditionButton = style({
  alignSelf: "flex-start",
  padding: "6px 10px",
  border: "1px dashed #93c5fd",
  borderRadius: "8px",
  background: "transparent",
  color: "#1d4ed8",
  font: "inherit",
  fontSize: "0.86rem",
  fontWeight: 600,
  cursor: "pointer",
  selectors: {
    "&:hover:not(:disabled)": {
      background: "#eff6ff",
    },
    "&:disabled": {
      color: "#9ca3af",
      borderColor: "#e5e7eb",
      cursor: "not-allowed",
    },
  },
});

export const previewBox = style({
  padding: "10px 12px",
  borderRadius: "8px",
  background: "#f8fafc",
  border: "1px solid #e5e7eb",
  color: "#374151",
  fontSize: "0.88rem",
  lineHeight: 1.5,
});

globalStyle(`${previewBox} strong`, {
  color: "#111827",
});

export const statList = style({
  display: "flex",
  flexWrap: "wrap",
  gap: "8px 32px",
  margin: 0,
});

export const statItem = style({
  display: "flex",
  flexDirection: "column",
  gap: "2px",
});

globalStyle(`${statItem} dt`, {
  color: "#6b7280",
  fontSize: "0.8rem",
});

globalStyle(`${statItem} dd`, {
  margin: 0,
  color: "#111827",
  fontSize: "0.95rem",
  fontWeight: 600,
  fontVariantNumeric: "tabular-nums",
});

export const spendHeader = style({
  display: "flex",
  alignItems: "center",
  justifyContent: "space-between",
  gap: "12px",
  marginBottom: "8px",
});

globalStyle(`${spendHeader} h3`, {
  margin: 0,
});

export const spendList = style({
  display: "flex",
  flexDirection: "column",
  gap: "6px",
  margin: 0,
  padding: 0,
  listStyle: "none",
});

export const spendItem = style({
  display: "flex",
  justifyContent: "space-between",
  gap: "12px",
  color: "#374151",
  fontSize: "0.88rem",
});

export const spendToDate = style({
  color: "#111827",
  fontWeight: 600,
  fontVariantNumeric: "tabular-nums",
  whiteSpace: "nowrap",
});

export const spendForm = style({
  display: "flex",
  flexDirection: "column",
  gap: "10px",
});

export const spendRow = style({
  display: "flex",
  flexWrap: "wrap",
  alignItems: "center",
  gap: "8px",
  padding: "6px",
  borderRadius: "8px",
  border: "1px solid transparent",
  selectors: {
    "&[data-invalid='true']": {
      borderColor: "#fecaca",
      background: "#fef2f2",
    },
  },
});

export const spendKindInput = style([textInput, { width: "150px" }]);

export const spendDateInput = style([textInput, { width: "160px" }]);

export const spendAmountField = style({
  display: "inline-flex",
  alignItems: "center",
  gap: "6px",
});

export const spendAmountInput = style([textInput, { width: "120px", textAlign: "right" }]);

export const editorActions = style({
  display: "flex",
  flexWrap: "wrap",
  alignItems: "center",
  gap: "8px",
});
