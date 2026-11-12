import { style } from "@vanilla-extract/css";

export const container = style({
  display: "flex",
  flexDirection: "column",
  alignItems: "center",
  gap: "10px",
  padding: "56px 24px",
  border: "1px dashed #e5e7eb",
  borderRadius: "12px",
  background: "#f9fafb",
  textAlign: "center",
});

export const heading = style({
  margin: 0,
  fontSize: "1.05rem",
  fontWeight: 600,
  color: "#111827",
});

export const description = style({
  margin: 0,
  maxWidth: "52ch",
  fontSize: "0.92rem",
  lineHeight: 1.6,
  color: "#6b7280",
});

export const links = style({
  display: "flex",
  flexWrap: "wrap",
  justifyContent: "center",
  gap: "10px",
  marginTop: "6px",
});
