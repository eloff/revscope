import { style } from "@vanilla-extract/css";

export const activeFilterChip = style({
  display: "inline-flex",
  alignItems: "center",
  gap: "6px",
  maxWidth: "100%",
  padding: "5px 6px 5px 12px",
  borderRadius: "999px",
  border: "1px solid #bfdbfe",
  background: "#eff6ff",
  color: "#1d4ed8",
  fontSize: "0.85rem",
  fontWeight: 600,
  lineHeight: 1.3,
});

export const activeFilterChipLabel = style({
  overflow: "hidden",
  textOverflow: "ellipsis",
  whiteSpace: "nowrap",
});

export const activeFilterChipRemove = style({
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  flexShrink: 0,
  width: "20px",
  height: "20px",
  padding: 0,
  border: "none",
  borderRadius: "999px",
  background: "transparent",
  color: "inherit",
  cursor: "pointer",
  selectors: {
    "&:hover": {
      background: "#dbeafe",
      color: "#1e3a8a",
    },
  },
});

export const activeFilterChipRemoveIcon = style({
  display: "block",
  width: "12px",
  height: "12px",
});
