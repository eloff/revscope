import { globalStyle, style } from "@vanilla-extract/css";
import { gridTable } from "../grid.css";

/** A wide result scrolls sideways rather than stretching the conversation. */
export const resultScroller = style({
  width: "100%",
  overflowX: "auto",
  maxHeight: "420px",
  overflowY: "auto",
});

// Square corners: a rounded table clips the first header cell's text against
// the corner, and the result already sits inside the rounded query block.
globalStyle(`${resultScroller} ${gridTable}`, {
  borderRadius: 0,
});

// The dashboard grids indent their headers with a sort button inside the cell.
// These headers are plain text, so the cell carries the padding itself.
globalStyle(`${resultScroller} ${gridTable} thead th`, {
  padding: "10px 12px",
});

export const resultCell = style({
  maxWidth: "320px",
  overflow: "hidden",
  textOverflow: "ellipsis",
  whiteSpace: "nowrap",
});

export const resultCaption = style({
  margin: 0,
  fontSize: "0.8rem",
  color: "#6b7280",
});
