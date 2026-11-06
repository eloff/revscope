import { style } from "@vanilla-extract/css";

export const gridInlineActionButton = style({
  appearance: "none",
  display: "inline-flex",
  alignItems: "center",
  gap: "4px",
  maxWidth: "100%",
  border: "1px solid #d1d5db",
  borderRadius: "999px",
  padding: "1px 6px",
  background: "#ffffff",
  color: "#111827",
  cursor: "pointer",
  font: "inherit",
  fontWeight: 500,
  lineHeight: 1.5,
  textAlign: "left",
  verticalAlign: "baseline",
  whiteSpace: "nowrap",
  selectors: {
    "&:hover": {
      background: "#f3f4f6",
      borderColor: "#9ca3af",
    },
    "&:focus-visible": {
      outline: "2px solid #93c5fd",
      outlineOffset: "2px",
    },
  },
});
