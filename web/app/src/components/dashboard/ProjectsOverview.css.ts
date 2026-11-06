import { style } from "@vanilla-extract/css";

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

export const addButton = style({
  padding: "9px 14px",
  border: 0,
  borderRadius: "8px",
  background: "#111827",
  color: "white",
  fontSize: "0.9rem",
  fontWeight: 600,
  fontFamily: "inherit",
  cursor: "pointer",
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
  padding: 0,
  border: 0,
  background: "none",
  fontFamily: "inherit",
  textAlign: "left",
  cursor: "pointer",
});

export const cardTitle = style({
  overflow: "hidden",
  fontSize: "1rem",
  fontWeight: 600,
  color: "#111827",
  textOverflow: "ellipsis",
  whiteSpace: "nowrap",
});

export const cardOpen = style({
  flex: "0 0 auto",
  fontSize: "0.85rem",
  color: "#2563eb",
});

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
