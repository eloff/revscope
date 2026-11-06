import { style } from "@vanilla-extract/css";

/** Row above a tab panel holding its mode switch. */
export const segmentedControlRow = style({
  display: "flex",
  flexWrap: "wrap",
  alignItems: "center",
  justifyContent: "flex-start",
  gap: "12px",
  marginBottom: "12px",
});

export const root = style({
  display: "inline-flex",
  alignItems: "stretch",
  padding: "2px",
  border: "1px solid #d1d5db",
  borderRadius: "999px",
  background: "#f9fafb",
});

export const indicator = style({
  top: "var(--top)",
  left: "var(--left)",
  width: "var(--width)",
  height: "var(--height)",
  borderRadius: "999px",
  background: "#dbeafe",
  boxShadow: "0 1px 2px rgba(37, 99, 235, 0.12)",
  zIndex: 0,
});

export const item = style({
  position: "relative",
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  zIndex: 1,
  cursor: "pointer",
  userSelect: "none",
  selectors: {
    "&:focus-within": {
      outline: "none",
    },
    "&:has(:focus-visible)": {
      outline: "2px solid #2563eb",
      outlineOffset: "2px",
      borderRadius: "999px",
    },
  },
});

export const itemText = style({
  padding: "5px 11px",
  font: "inherit",
  fontSize: "13px",
  fontWeight: 500,
  color: "#6b7280",
  whiteSpace: "nowrap",
  lineHeight: 1.2,
  selectors: {
    '&[data-state="checked"]': {
      color: "#111827",
    },
  },
});

export const itemControl = style({
  position: "absolute",
  inset: 0,
  cursor: "pointer",
});
