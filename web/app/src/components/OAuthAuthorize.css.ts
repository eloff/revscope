import { style } from "@vanilla-extract/css";

export const card = style({
  maxWidth: "520px",
  margin: "0 auto",
  padding: "28px",
  border: "1px solid #e5e7eb",
  borderRadius: "14px",
  background: "#ffffff",
  boxShadow: "0 10px 30px rgba(15, 23, 42, 0.06)",
});

export const clientName = style({
  margin: "0 0 6px",
  fontSize: "1.35rem",
  fontWeight: 650,
  letterSpacing: "-0.01em",
  color: "#111827",
});

export const requestSummary = style({
  margin: "0 0 22px",
  color: "#4b5563",
  fontSize: "0.95rem",
  lineHeight: 1.6,
});

export const redirectHost = style({
  fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
  fontSize: "0.9em",
  color: "#111827",
});

/** The read-only access the token grants, spelled out before approval. */
export const scopeList = style({
  margin: "0 0 22px",
  padding: "14px 16px 14px 34px",
  borderRadius: "10px",
  background: "#f9fafb",
  color: "#374151",
  fontSize: "0.9rem",
  lineHeight: 1.7,
});

export const fieldHelp = style({
  margin: "0 0 22px",
  color: "#6b7280",
  fontSize: "0.82rem",
  lineHeight: 1.5,
});

export const settingsLink = style({
  color: "#2563eb",
  fontWeight: 600,
});

/** The account's current projects, named under the scope list. */
export const projectNames = style({
  fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
  fontSize: "0.9em",
  color: "#111827",
});

export const actionRow = style({
  display: "flex",
  gap: "12px",
  alignItems: "center",
});

const buttonBase = style({
  flex: 1,
  height: "44px",
  borderRadius: "10px",
  border: "1px solid transparent",
  fontFamily: "inherit",
  fontSize: "1rem",
  fontWeight: 600,
  cursor: "pointer",
});

export const approveButton = style([
  buttonBase,
  {
    background: "#2563eb",
    color: "#ffffff",
    selectors: {
      "&:hover:not(:disabled)": {
        background: "#1d4ed8",
      },
      "&:disabled": {
        opacity: 0.6,
        cursor: "not-allowed",
      },
    },
  },
]);

export const denyButton = style([
  buttonBase,
  {
    background: "#ffffff",
    borderColor: "#d1d5db",
    color: "#374151",
    selectors: {
      "&:hover:not(:disabled)": {
        background: "#f9fafb",
      },
      "&:disabled": {
        opacity: 0.6,
        cursor: "not-allowed",
      },
    },
  },
]);

export const statusMessage = style({
  margin: 0,
  color: "#4b5563",
  fontSize: "0.95rem",
  lineHeight: 1.6,
});

export const errorMessage = style({
  margin: "18px 0 0",
  color: "#b91c1c",
  fontSize: "0.9rem",
  lineHeight: 1.5,
});
