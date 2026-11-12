import { style } from "@vanilla-extract/css";

export const resendButton = style({
  flex: "0 0 auto",
  padding: "6px 12px",
  color: "#ffffff",
  background: "#b45309",
  border: "none",
  borderRadius: "6px",
  font: "inherit",
  fontWeight: 600,
  cursor: "pointer",
  ":hover": {
    background: "#92400e",
  },
  ":disabled": {
    opacity: 0.7,
    cursor: "default",
  },
});
