import { style } from "@vanilla-extract/css";
import { buttonIcon, secondaryButton } from "../buttons.css";

export const header = style({
  display: "flex",
  flexWrap: "wrap",
  alignItems: "center",
  justifyContent: "space-between",
  gap: "12px",
  marginBottom: "16px",
});

export const headerTitle = style({
  margin: 0,
  fontSize: "1.05rem",
  fontWeight: 600,
  color: "#111827",
});

export const headerNote = style({
  margin: 0,
  fontSize: "0.9rem",
  color: "#6b7280",
});

export const grid = style({
  display: "grid",
  gridTemplateColumns: "repeat(2, minmax(0, 1fr))",
  gap: "16px",
  "@media": {
    "(max-width: 900px)": {
      gridTemplateColumns: "minmax(0, 1fr)",
    },
  },
});

export const card = style({
  display: "flex",
  flexDirection: "column",
  gap: "12px",
  minWidth: 0,
  padding: "16px",
  border: "1px solid #e5e7eb",
  borderRadius: "12px",
  background: "#ffffff",
});

export const currentCard = style({
  borderColor: "#93c5fd",
});

export const cardHeader = style({
  display: "flex",
  alignItems: "center",
  justifyContent: "space-between",
  gap: "8px",
  margin: "-6px -6px 0",
  padding: "6px",
  border: 0,
  borderRadius: "8px",
  background: "none",
  fontFamily: "inherit",
  textAlign: "left",
  cursor: "pointer",
  selectors: {
    "&:focus-visible": {
      outline: "2px solid #2563eb",
      outlineOffset: "0",
    },
  },
});

export const cardTitle = style({
  overflow: "hidden",
  selectors: {
    [`${cardHeader}:hover &`]: {
      color: "#1d4ed8",
    },
  },
  fontSize: "1rem",
  fontWeight: 600,
  color: "#111827",
  textOverflow: "ellipsis",
  whiteSpace: "nowrap",
});

// The whole header is the button, so the chip only looks like one and follows the header's hover.
export const cardOpen = style([
  secondaryButton,
  {
    gap: "4px",
    padding: "5px 8px 5px 10px",
    fontSize: "0.82rem",
    color: "#374151",
    transition: "background 120ms ease, border-color 120ms ease, color 120ms ease",
    selectors: {
      [`${cardHeader}:hover &`]: {
        borderColor: "#93c5fd",
        background: "#eff6ff",
        color: "#1d4ed8",
      },
    },
  },
]);

export const cardOpenIcon = style([
  buttonIcon,
  {
    transition: "transform 120ms ease",
    selectors: {
      [`${cardHeader}:hover &`]: {
        transform: "translateX(2px)",
      },
    },
  },
]);

export const cardTiles = style({
  display: "grid",
  gridTemplateColumns: "repeat(2, minmax(0, 1fr))",
  gap: "12px",
});

export const cardEmpty = style({
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  minHeight: "260px",
  margin: 0,
  color: "#6b7280",
  fontSize: "0.9rem",
});

export const dialogField = style({
  display: "grid",
  gap: "6px",
  marginTop: "12px",
});

export const dialogPreview = style({
  margin: 0,
  fontSize: "0.85rem",
  color: "#6b7280",
});
