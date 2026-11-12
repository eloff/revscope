import { globalStyle, style } from "@vanilla-extract/css";

export const page = style({
  width: "100%",
  maxWidth: "860px",
  margin: "0 auto",
  padding: "24px 28px 64px",
  fontFamily: "Inter, Roboto, 'Helvetica Neue', Arial, sans-serif",
  color: "#111827",
});

export const header = style({
  marginBottom: "20px",
});

export const heading = style({
  margin: "0 0 4px",
  fontSize: "1.6rem",
});

export const subtitle = style({
  margin: 0,
  color: "#6b7280",
});

export const message = style({
  color: "#6b7280",
});

export const errorMessage = style({
  margin: "0 0 20px",
  fontSize: "0.9rem",
  color: "#dc2626",
});

export const section = style({
  border: "1px solid #e5e7eb",
  borderRadius: "12px",
  background: "#ffffff",
  padding: "20px",
  marginBottom: "20px",
});

export const sectionTitle = style({
  margin: "0 0 4px",
  fontSize: "1.15rem",
});

export const sectionDescription = style({
  margin: "0 0 16px",
  fontSize: "0.9rem",
  color: "#6b7280",
});

export const metric = style({
  margin: "0 0 2px",
  fontSize: "1.8rem",
  fontWeight: 700,
  lineHeight: 1.1,
  fontVariantNumeric: "tabular-nums",
});

export const metricLabel = style({
  margin: 0,
  fontSize: "0.95rem",
  color: "#6b7280",
});

export const field = style({
  display: "grid",
  gap: "6px",
  marginBottom: "12px",
});

export const fieldLabel = style({
  fontSize: "0.9rem",
  fontWeight: 600,
  color: "#374151",
});

export const inputRow = style({
  display: "flex",
  flexWrap: "wrap",
  gap: "8px",
});

export const input = style({
  flex: "1 1 240px",
  padding: "10px 12px",
  borderRadius: "8px",
  border: "1px solid #d1d5db",
  fontSize: "0.95rem",
  fontFamily: "inherit",
});

export const findings = style({
  listStyle: "none",
  margin: "12px 0 0",
  padding: 0,
  display: "grid",
  gap: "8px",
});

export const finding = style({
  margin: 0,
  padding: "10px 12px",
  borderRadius: "8px",
  fontSize: "0.9rem",
  lineHeight: 1.45,
});

export const findingOk = style({
  background: "#ecfdf5",
  color: "#065f46",
});

export const findingWarning = style({
  background: "#fffbeb",
  color: "#92400e",
});

export const findingError = style({
  background: "#fef2f2",
  color: "#991b1b",
});

export const findingWaiting = style({
  background: "#eff6ff",
  color: "#1e40af",
});

export const emptyState = style({
  margin: 0,
  fontSize: "0.9rem",
  color: "#6b7280",
});

export const tableScroll = style({
  overflowX: "auto",
});

export const table = style({
  width: "100%",
  borderCollapse: "collapse",
  fontSize: "0.9rem",
});

globalStyle(`${table} th`, {
  padding: "8px 12px 8px 0",
  borderBottom: "1px solid #e5e7eb",
  textAlign: "left",
  fontSize: "0.78rem",
  fontWeight: 700,
  color: "#4b5563",
  textTransform: "uppercase",
  letterSpacing: "0.02em",
  whiteSpace: "nowrap",
});

globalStyle(`${table} td`, {
  padding: "10px 12px 10px 0",
  borderBottom: "1px solid #f1f5f9",
  textAlign: "left",
  verticalAlign: "top",
});

globalStyle(`${table} tbody tr:last-child td`, {
  borderBottom: "none",
});

export const badge = style({
  display: "inline-block",
  padding: "2px 8px",
  borderRadius: "999px",
  background: "#f1f5f9",
  color: "#475569",
  fontSize: "0.75rem",
  fontWeight: 600,
  lineHeight: 1.5,
  whiteSpace: "nowrap",
});

export const badgeOk = style({
  background: "#dcfce7",
  color: "#15803d",
});

export const badgeError = style({
  background: "#fee2e2",
  color: "#b91c1c",
});

export const mutedCell = style({
  color: "#6b7280",
});

export const docsLink = style({
  color: "#1d4ed8",
  textDecoration: "underline",
});

export const followUp = style({
  margin: "12px 0 0",
  fontSize: "0.9rem",
  color: "#6b7280",
});
