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

const linkBase = style({
  display: "inline-flex",
  alignItems: "center",
  padding: "9px 14px",
  borderRadius: "8px",
  fontSize: "0.9rem",
  fontWeight: 600,
  textDecoration: "none",
});

export const primaryLink = style([
  linkBase,
  {
    border: "1px solid #2563eb",
    background: "#2563eb",
    color: "#ffffff",
    selectors: {
      "&:hover": {
        background: "#1d4ed8",
      },
    },
  },
]);

export const secondaryLink = style([
  linkBase,
  {
    border: "1px solid #d1d5db",
    background: "#ffffff",
    color: "#374151",
    selectors: {
      "&:hover": {
        background: "#f3f4f6",
      },
    },
  },
]);
