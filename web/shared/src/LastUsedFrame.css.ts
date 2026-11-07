import { style } from "@vanilla-extract/css";

const FRAME_PADDING_PX = 10;
const BORDER_PX = 1;

export const frame = style({
  position: "relative",
  // Outset by the padding and border so what's framed keeps the width and
  // left edge of the sign-in options around it, which also sizes Google's button.
  margin: `10px -${FRAME_PADDING_PX + BORDER_PX}px 0`,
  padding: `14px ${FRAME_PADDING_PX}px ${FRAME_PADDING_PX}px`,
  border: `${BORDER_PX}px solid #2563eb`,
  borderRadius: "8px",
});

export const legend = style({
  position: "absolute",
  top: 0,
  left: "12px",
  transform: "translateY(-50%)",
  padding: "0 6px",
  // The dialog's background, which hides the border behind the text.
  background: "#ffffff",
  color: "#2563eb",
  fontSize: "0.75rem",
  fontWeight: 600,
  lineHeight: 1,
});
