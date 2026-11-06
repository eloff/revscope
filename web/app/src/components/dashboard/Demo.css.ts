import { style } from "@vanilla-extract/css";
import { EMBED_CONTROLS_LAYER } from "./layers";

export const embed = style({
  position: "relative",
  margin: "28px 10%",
  width: "auto",
  "@media": {
    // The dashboard needs more width than a phone has, so /demo serves those visitors.
    "screen and (max-width: 699px)": {
      display: "none",
    },
    // Percentage side margins waste too much of a narrow screen.
    "screen and (max-width: 900px)": {
      margin: "20px 12px",
    },
  },
});

export const fullscreenLink = style({
  position: "absolute",
  top: "12px",
  right: "12px",
  zIndex: 5,
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  width: "36px",
  height: "36px",
  border: "1px solid #d1d5db",
  borderRadius: "8px",
  background: "#ffffff",
  color: "#374151",
  textDecoration: "none",
  boxShadow: "0 2px 8px rgba(0, 0, 0, 0.08)",
  selectors: {
    "&:hover": {
      background: "#f9fafb",
      color: "#111827",
    },
    "&:focus-visible": {
      outline: "2px solid #2563eb",
      outlineOffset: "2px",
    },
  },
});

export const fullscreenIcon = style({
  width: "18px",
  height: "18px",
});

/**
 * Catches every click in the frame while collapsed, so the first click expands
 * rather than acting. Fixed covers the whole frame, margins included, because the
 * collapsed frame doesn't scroll.
 */
export const expandOverlay = style({
  position: "fixed",
  inset: 0,
  zIndex: EMBED_CONTROLS_LAYER,
  display: "flex",
  alignItems: "flex-end",
  justifyContent: "center",
  padding: "0 0 20px",
  border: 0,
  background: "transparent",
  cursor: "pointer",
  selectors: {
    "&:focus-visible": {
      outline: "2px solid #2563eb",
      outlineOffset: "-2px",
    },
  },
});

export const expandHint = style({
  padding: "8px 14px",
  borderRadius: "999px",
  background: "#111827",
  color: "#ffffff",
  font: "500 14px Inter, Roboto, 'Helvetica Neue', Arial, sans-serif",
  boxShadow: "0 4px 14px rgba(0, 0, 0, 0.18)",
  opacity: 0,
  transform: "translateY(4px)",
  transition: "opacity 150ms ease, transform 150ms ease",
  pointerEvents: "none",
  selectors: {
    [`${expandOverlay}:hover &`]: {
      opacity: 1,
      transform: "none",
    },
    [`${expandOverlay}:focus-visible &`]: {
      opacity: 1,
      transform: "none",
    },
  },
});

export const closeButton = style({
  position: "fixed",
  top: "16px",
  right: "16px",
  zIndex: EMBED_CONTROLS_LAYER,
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  width: "48px",
  height: "48px",
  border: "1px solid #d1d5db",
  borderRadius: "999px",
  background: "#ffffff",
  color: "#111827",
  cursor: "pointer",
  boxShadow: "0 4px 14px rgba(0, 0, 0, 0.12)",
  selectors: {
    "&:hover": {
      background: "#f3f4f6",
    },
    "&:focus-visible": {
      outline: "2px solid #2563eb",
      outlineOffset: "2px",
    },
  },
});

export const closeIcon = style({
  width: "24px",
  height: "24px",
});
