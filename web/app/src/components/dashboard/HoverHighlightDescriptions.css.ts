import { style } from "@vanilla-extract/css";

export const hoverCardDescriptionRow = style({
  display: "flex",
  alignItems: "flex-start",
  gap: "6px",
});

export const hoverCardDescriptionHighlights = style({
  display: "flex",
  flexDirection: "column",
  gap: "4px",
});

export const hoverCardDescriptionStar = style({
  color: "#ca8a04",
  fontSize: "1.05rem",
  lineHeight: 1.5,
  flexShrink: 0,
});
