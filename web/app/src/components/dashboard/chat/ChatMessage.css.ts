import { keyframes, style } from "@vanilla-extract/css";

export const userTurn = style({
  display: "flex",
  justifyContent: "flex-end",
  marginBottom: "14px",
});

export const userBubble = style({
  margin: 0,
  maxWidth: "80%",
  padding: "10px 14px",
  borderRadius: "14px 14px 2px 14px",
  background: "#2563eb",
  color: "#ffffff",
  lineHeight: 1.5,
  whiteSpace: "pre-wrap",
  overflowWrap: "anywhere",
});

export const assistantTurn = style({
  display: "flex",
  flexDirection: "column",
  gap: "12px",
  marginBottom: "28px",
  color: "#111827",
});

export const turnLabel = style({
  display: "flex",
  alignItems: "center",
  gap: "8px",
  margin: 0,
  color: "#6b7280",
  fontSize: "0.9rem",
});

const pulse = keyframes({
  "0%, 100%": { opacity: 0.25 },
  "50%": { opacity: 1 },
});

export const pendingDot = style({
  width: "8px",
  height: "8px",
  borderRadius: "50%",
  background: "#2563eb",
  animation: `${pulse} 1.2s ease-in-out infinite`,
});
