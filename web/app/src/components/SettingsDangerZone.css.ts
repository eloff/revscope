import { style } from "@vanilla-extract/css";

export const dangerZone = style({
  border: "1px solid #fecaca",
  borderRadius: "12px",
  background: "#fff7f7",
  padding: "20px",
  marginTop: "32px",
});

export const dangerTitle = style({
  margin: "0 0 4px",
  fontSize: "1.15rem",
  color: "#991b1b",
});

export const dangerDescription = style({
  margin: "0 0 16px",
  fontSize: "0.9rem",
  color: "#7f1d1d",
});

export const dangerActions = style({
  display: "flex",
  alignItems: "center",
  gap: "16px",
});

export const dangerItem = style({
  paddingTop: "16px",
  borderTop: "1px solid #fecaca",
  marginTop: "16px",
});

export const dangerItemTitle = style({
  margin: "0 0 4px",
  fontSize: "1rem",
  color: "#991b1b",
});

export const confirmField = style({
  display: "grid",
  gap: "6px",
  marginTop: "12px",
});
