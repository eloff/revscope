import { style } from "@vanilla-extract/css";

export const paginationBar = style({
  display: "flex",
  alignItems: "center",
  justifyContent: "flex-end",
  gap: "12px",
  padding: "4px 0",
});

export const paginationLabel = style({
  marginRight: "auto",
  color: "#6b7280",
  fontSize: "0.85rem",
});

export const paginationButton = style({
  padding: "6px 14px",
  border: "1px solid #d1d5db",
  borderRadius: "8px",
  background: "#ffffff",
  color: "#111827",
  fontSize: "0.85rem",
  cursor: "pointer",
  selectors: {
    "&:disabled": {
      color: "#9ca3af",
      background: "#f9fafb",
      cursor: "not-allowed",
    },
    "&:hover:not(:disabled)": {
      background: "#f3f4f6",
    },
  },
});
