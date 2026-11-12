import { style } from "@vanilla-extract/css";

/**
 * Text action buttons for forms, dialogs and page headers: outlined secondary, blue primary,
 * red-outlined danger, and filled-red destructive for confirming what can't be undone. Icon buttons,
 * tabs, chips and the OAuth consent screen's full-width pair are styled where they're used.
 */
export const secondaryButton = style({
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  gap: "6px",
  padding: "8px 12px",
  border: "1px solid #d1d5db",
  borderRadius: "8px",
  background: "#ffffff",
  color: "#111827",
  font: "inherit",
  fontSize: "0.88rem",
  fontWeight: 600,
  lineHeight: 1.25,
  whiteSpace: "nowrap",
  cursor: "pointer",
  textDecoration: "none",
  selectors: {
    "&:disabled": {
      color: "#9ca3af",
      background: "#f9fafb",
      cursor: "not-allowed",
    },
    "&:hover:not(:disabled)": {
      background: "#f3f4f6",
    },
    "&:focus-visible": {
      outline: "2px solid #2563eb",
      outlineOffset: "2px",
    },
  },
});

export const primaryButton = style([
  secondaryButton,
  {
    borderColor: "#2563eb",
    background: "#2563eb",
    color: "#ffffff",
    selectors: {
      "&:hover:not(:disabled)": {
        background: "#1d4ed8",
      },
      "&:disabled": {
        borderColor: "#93c5fd",
        background: "#93c5fd",
        color: "#ffffff",
      },
    },
  },
]);

export const dangerButton = style([
  secondaryButton,
  {
    borderColor: "#fecaca",
    color: "#b91c1c",
    selectors: {
      "&:hover:not(:disabled)": {
        background: "#fef2f2",
      },
    },
  },
]);

export const destructiveButton = style([
  secondaryButton,
  {
    borderColor: "#dc2626",
    background: "#dc2626",
    color: "#ffffff",
    selectors: {
      "&:hover:not(:disabled)": {
        background: "#b91c1c",
      },
      "&:disabled": {
        borderColor: "#fca5a5",
        background: "#fca5a5",
        color: "#ffffff",
      },
    },
  },
]);

/** Sizes an inline SVG icon to the button's text. */
export const buttonIcon = style({
  width: "1em",
  height: "1em",
  flexShrink: 0,
});
