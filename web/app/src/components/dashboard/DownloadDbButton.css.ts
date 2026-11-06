import { style } from "@vanilla-extract/css";

export const downloadButton = style({
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  flexShrink: 0,
  width: "36px",
  height: "36px",
  padding: 0,
  border: "1px solid #e5e7eb",
  borderRadius: "999px",
  background: "#ffffff",
  color: "#374151",
  cursor: "pointer",
  selectors: {
    "&:hover": {
      color: "#111827",
      background: "#f9fafb",
    },
  },
});

export const downloadIcon = style({
  display: "block",
  width: "17px",
  height: "17px",
});

export const downloadSize = style({
  margin: "0 0 14px",
  color: "#4b5563",
  fontSize: "0.9rem",
});

/** Makes the confirm anchor sit like the button styles it borrows expect. */
export const confirmLink = style({
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  textDecoration: "none",
});
