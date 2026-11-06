import { style } from "@vanilla-extract/css";

const mobile = "screen and (max-width: 760px)";

export const header = style({
  position: "sticky",
  top: 0,
  zIndex: 50,
  boxSizing: "border-box",
  height: "3.75rem",
  background: "rgba(255, 255, 255, 0.92)",
  backdropFilter: "blur(8px)",
  borderBottom: "1px solid #e5e7eb",
});

export const inner = style({
  boxSizing: "border-box",
  maxWidth: "1120px",
  height: "100%",
  margin: "0 auto",
  padding: "0 1.5rem",
  display: "flex",
  alignItems: "center",
  gap: "1.5rem",
  "@media": {
    [mobile]: {
      padding: "0 1rem",
    },
  },
});

export const logoLink = style({
  display: "inline-flex",
  alignItems: "center",
  flexShrink: 0,
  color: "#111827",
  textDecoration: "none",
});

export const logoImage = style({
  display: "block",
  "@media": {
    [mobile]: {
      width: "auto",
      height: "1.5rem",
    },
  },
});

export const actions = style({
  marginLeft: "auto",
  display: "flex",
  alignItems: "center",
  flexShrink: 0,
});

export const loginTrigger = style({
  border: "1px solid #93c5fd",
  background: "white",
  color: "#1d4ed8",
  padding: "0.5rem 0.875rem",
  borderRadius: "0.5rem",
  fontSize: "0.95rem",
  fontWeight: 600,
  lineHeight: 1.2,
  cursor: "pointer",
  whiteSpace: "nowrap",
  selectors: {
    "&:hover": {
      background: "#f9fafb",
    },
  },
});
