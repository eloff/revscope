import { style } from "@vanilla-extract/css";

export const loginPrompt = style({
  margin: "16px 0 0",
  color: "#4b5563",
  fontSize: "0.9rem",
  textAlign: "center",
});

export const loginPromptButton = style({
  padding: 0,
  border: 0,
  background: "none",
  color: "#2563eb",
  font: "inherit",
  fontWeight: 600,
  cursor: "pointer",
  selectors: {
    "&:hover": {
      textDecoration: "underline",
    },
  },
});
