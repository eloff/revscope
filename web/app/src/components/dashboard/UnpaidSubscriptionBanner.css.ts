import { style } from "@vanilla-extract/css";

export const banner = style({
  display: "flex",
  flexWrap: "wrap",
  alignItems: "center",
  gap: "8px 16px",
  margin: "0 0 12px",
  padding: "10px 14px",
  color: "#92400e",
  background: "#fffbeb",
  border: "1px solid #fde68a",
  borderRadius: "6px",
  fontSize: "0.9rem",
});

export const bannerText = style({
  margin: 0,
  flex: "1 1 320px",
});

export const payLink = style({
  flex: "0 0 auto",
  padding: "6px 12px",
  color: "#ffffff",
  background: "#b45309",
  borderRadius: "6px",
  fontWeight: 600,
  textDecoration: "none",
  ":hover": {
    background: "#92400e",
  },
});
