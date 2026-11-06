import { style } from "@vanilla-extract/css";

const changeBase = {
  fontSize: "0.78rem",
  fontWeight: 600,
  lineHeight: 1.2,
};

export const changePositive = style({
  ...changeBase,
  color: "#15803d",
});

export const changeNegative = style({
  ...changeBase,
  color: "#b91c1c",
});

export const changeNeutral = style({
  ...changeBase,
  color: "#6b7280",
});
