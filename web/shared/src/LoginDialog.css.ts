import { style } from "@vanilla-extract/css";

export const backdrop = style({
  position: "fixed",
  inset: 0,
  zIndex: 100,
  background: "rgba(0, 0, 0, 0.4)",
});

export const positioner = style({
  position: "fixed",
  inset: 0,
  zIndex: 101,
  display: "grid",
  placeItems: "center",
  padding: "20px",
});

export const content = style({
  position: "relative",
  width: "min(420px, 100%)",
  border: "1px solid #e5e7eb",
  borderRadius: "12px",
  padding: "20px",
  background: "#ffffff",
  boxShadow: "0 20px 80px rgba(0, 0, 0, 0.2)",
});

export const title = style({
  margin: "0 0 6px",
  fontSize: "1.2rem",
});

export const description = style({
  margin: "0 0 14px",
  color: "#4b5563",
});

export const linkTrigger = style({
  display: "inline",
  border: 0,
  background: "transparent",
  padding: 0,
  color: "#2563eb",
  cursor: "pointer",
  font: "inherit",
  fontWeight: 600,
  textDecoration: "underline",
});

export const textButton = style({
  width: "fit-content",
  border: 0,
  background: "transparent",
  padding: 0,
  color: "#2563eb",
  cursor: "pointer",
  fontSize: "0.9rem",
  textAlign: "left",
  textDecoration: "underline",
});

export const rememberMeRow = style({
  display: "flex",
  alignItems: "center",
  gap: "8px",
  color: "#374151",
  fontSize: "0.9rem",
  cursor: "pointer",
  userSelect: "none",
});

export const rememberMeInput = style({
  width: "15px",
  height: "15px",
  margin: 0,
  flexShrink: 0,
  cursor: "pointer",
  accentColor: "#2563eb",
});

export const actionRow = style({
  display: "flex",
  alignItems: "center",
  gap: "8px",
  marginTop: "6px",
});

export const actionPrimaryButton = style({
  flex: 1,
  marginTop: 0,
});

export const secondaryButton = style({
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  padding: "10px 14px",
  borderRadius: "8px",
  border: "1px solid #d1d5db",
  background: "white",
  color: "#374151",
  fontSize: "0.95rem",
  fontWeight: 600,
  textDecoration: "none",
  cursor: "pointer",
  selectors: {
    "&:hover": {
      background: "#f9fafb",
    },
  },
});

export const iconCloseButton = style({
  position: "absolute",
  top: "12px",
  right: "12px",
  display: "grid",
  placeItems: "center",
  width: "32px",
  height: "32px",
  borderRadius: "999px",
  border: "1px solid #d1d5db",
  background: "white",
  color: "#374151",
  fontSize: "1rem",
  fontWeight: 700,
  lineHeight: 1,
  cursor: "pointer",
  selectors: {
    "&:hover": {
      background: "#f9fafb",
    },
  },
});
