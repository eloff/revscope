import { style } from "@vanilla-extract/css";

export const tabDocsLink = style({
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  gap: "6px",
  flexShrink: 0,
  height: "32px",
  padding: "0 10px",
  borderRadius: "8px",
  color: "#6b7280",
  fontSize: "0.82rem",
  fontWeight: 600,
  lineHeight: 1,
  whiteSpace: "nowrap",
  textDecoration: "none",
  selectors: {
    "&:hover": {
      color: "#2563eb",
      background: "#eff6ff",
    },
  },
});

export const tabDocsIcon = style({
  display: "block",
  width: "16px",
  height: "16px",
});
