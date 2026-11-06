import { globalStyle, style } from "@vanilla-extract/css";

export const countryLinkButton = style({
  appearance: "none",
  display: "inline-flex",
  alignItems: "center",
  gap: "8px",
  maxWidth: "100%",
  border: "none",
  padding: 0,
  background: "transparent",
  color: "#111827",
  cursor: "pointer",
  font: "inherit",
  fontWeight: 500,
  lineHeight: 1.5,
  textAlign: "left",
  verticalAlign: "baseline",
});

export const countryLinkButtonPill = style({
  display: "inline-flex",
  alignItems: "center",
  gap: "4px",
  minWidth: 0,
  maxWidth: "100%",
  border: "1px solid #d1d5db",
  borderRadius: "999px",
  padding: "1px 6px",
  background: "#ffffff",
  color: "#111827",
  font: "inherit",
  fontWeight: 500,
  lineHeight: 1.5,
  whiteSpace: "nowrap",
});

globalStyle(`${countryLinkButton}:hover ${countryLinkButtonPill}`, {
  background: "#f3f4f6",
  borderColor: "#9ca3af",
});

globalStyle(`${countryLinkButton}:focus-visible ${countryLinkButtonPill}`, {
  outline: "2px solid #93c5fd",
  outlineOffset: "2px",
});
