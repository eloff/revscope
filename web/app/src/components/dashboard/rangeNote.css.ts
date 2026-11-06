import { style } from "@vanilla-extract/css";

export const rangeNote = style({
  margin: 0,
  fontSize: "1.125rem",
  lineHeight: 1.5,
  color: "#6b7280",
});

export const rangeNoteSpaced = style([
  rangeNote,
  {
    marginBottom: "16px",
  },
]);
