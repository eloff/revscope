import { style } from "@vanilla-extract/css";

export const connectOptions = style({
  display: "grid",
  gap: "16px",
  marginTop: "16px",
});

export const keyInput = style({
  width: "100%",
  minHeight: "96px",
  padding: "8px 10px",
  fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
  fontSize: "0.8rem",
  border: "1px solid #d1d5db",
  borderRadius: "6px",
  boxSizing: "border-box",
});

export const form = style({
  display: "grid",
  gap: "14px",
  marginTop: "16px",
});

export const dateRow = style({
  display: "flex",
  flexWrap: "wrap",
  gap: "14px",
});

export const warning = style({
  margin: 0,
  padding: "10px 12px",
  color: "#92400e",
  background: "#fffbeb",
  border: "1px solid #fde68a",
  borderRadius: "6px",
});

export const importList = style({
  listStyle: "none",
  margin: "16px 0 0",
  padding: 0,
  display: "grid",
  gap: "10px",
});

export const importRow = style({
  display: "flex",
  flexWrap: "wrap",
  justifyContent: "space-between",
  alignItems: "center",
  gap: "10px",
  padding: "10px 12px",
  border: "1px solid #e5e7eb",
  borderRadius: "6px",
});

export const importName = style({
  fontWeight: 600,
});

export const importDetail = style({
  margin: "2px 0 0",
  fontSize: "0.85rem",
  color: "#6b7280",
});

export const importError = style({
  margin: "2px 0 0",
  fontSize: "0.85rem",
  color: "#b91c1c",
});

export const importActions = style({
  display: "flex",
  gap: "8px",
});

export const toggleRow = style({
  display: "flex",
  alignItems: "center",
  gap: "8px",
  marginTop: "16px",
});
