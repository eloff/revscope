import { style } from "@vanilla-extract/css";

export const form = style({
  display: "grid",
  gap: "10px",
});

export const label = style({
  fontSize: "0.9rem",
  color: "#374151",
});

export const input = style({
  padding: "10px 12px",
  borderRadius: "8px",
  border: "1px solid #d1d5db",
  fontSize: "0.95rem",
});

export const primaryButton = style({
  marginTop: "6px",
  padding: "10px 14px",
  border: 0,
  borderRadius: "8px",
  background: "#2563eb",
  color: "#ffffff",
  fontSize: "0.95rem",
  fontWeight: 600,
  cursor: "pointer",
  selectors: {
    "&:hover": {
      background: "#1d4ed8",
    },
    "&:disabled": {
      opacity: 0.7,
      cursor: "wait",
    },
  },
});

export const error = style({
  margin: "6px 0 0",
  color: "#dc2626",
});

export const success = style({
  margin: "6px 0 0",
  color: "#16a34a",
});

export const formCard = style({
  border: "1px solid #e5e7eb",
  borderRadius: "12px",
  background: "#ffffff",
  padding: "24px",
});

export const formTitle = style({
  margin: "0 0 16px",
  fontSize: "1.2rem",
  color: "#111827",
});
