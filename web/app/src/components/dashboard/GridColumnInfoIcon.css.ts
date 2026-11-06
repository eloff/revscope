import { globalStyle, style } from "@vanilla-extract/css";
import { gridSortHeaderCell } from "./grid.css";

export const gridColumnInfoButton = style({
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  width: "16px",
  height: "16px",
  padding: 0,
  border: "none",
  background: "transparent",
  color: "#9ca3af",
  cursor: "help",
  flexShrink: 0,
  selectors: {
    "&:hover": {
      color: "#6b7280",
    },
  },
});

export const gridColumnWarningButton = style([
  gridColumnInfoButton,
  {
    color: "#d97706",
    selectors: {
      "&:hover": {
        color: "#b45309",
      },
    },
  },
]);

export const gridColumnInfoIcon = style({
  display: "block",
  width: "14px",
  height: "14px",
});

globalStyle(`${gridSortHeaderCell} ${gridColumnInfoButton}`, {
  marginLeft: "4px",
});
