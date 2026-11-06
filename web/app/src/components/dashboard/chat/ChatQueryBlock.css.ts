import { style } from "@vanilla-extract/css";

export const queryBlock = style({
  display: "flex",
  flexDirection: "column",
  gap: "10px",
  padding: "12px",
  border: "1px solid #e5e7eb",
  borderRadius: "10px",
  background: "#f8fafc",
});

export const queryTrigger = style({
  display: "flex",
  alignItems: "center",
  justifyContent: "space-between",
  gap: "12px",
  width: "100%",
  padding: 0,
  border: "none",
  background: "none",
  color: "#2563eb",
  fontSize: "0.8rem",
  fontWeight: 600,
  cursor: "pointer",
});

export const queryStatus = style({
  color: "#4b5563",
  textAlign: "left",
});

export const querySql = style({
  margin: "10px 0 0",
  padding: "10px",
  borderRadius: "8px",
  background: "#111827",
  color: "#e5e7eb",
  fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
  fontSize: "0.8rem",
  lineHeight: 1.5,
  whiteSpace: "pre-wrap",
  overflowX: "auto",
});

export const queryError = style({
  margin: 0,
  color: "#b91c1c",
  fontSize: "0.85rem",
});
