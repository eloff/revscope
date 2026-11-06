import { style } from "@vanilla-extract/css";

export const journeyShell = style({
  display: "flex",
  flexDirection: "column",
  gap: "18px",
  width: "100%",
});

export const journeyHeader = style({
  display: "flex",
  alignItems: "flex-start",
  gap: "14px",
});

export const journeyHeaderTitle = style({
  flex: 1,
  minWidth: 0,
});

export const journeyIconButton = style({
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  flexShrink: 0,
  width: "32px",
  height: "32px",
  padding: 0,
  border: "1px solid #d1d5db",
  borderRadius: "8px",
  background: "#ffffff",
  color: "#111827",
  cursor: "pointer",
  selectors: {
    "&:hover": {
      background: "#f3f4f6",
    },
  },
});

export const journeyIconButtonIcon = style({
  display: "block",
  width: "16px",
  height: "16px",
});

export const journeyTitle = style({
  margin: 0,
  fontSize: "1.15rem",
  lineHeight: 1.3,
  color: "#111827",
});

export const journeySubtitle = style({
  margin: "3px 0 0",
  color: "#6b7280",
  fontSize: "0.88rem",
  overflowWrap: "anywhere",
});

export const journeyColumns = style({
  display: "grid",
  gridTemplateColumns: "minmax(220px, 0.78fr) minmax(0, 1.7fr)",
  gap: "18px",
  alignItems: "start",
  "@media": {
    "screen and (max-width: 900px)": {
      gridTemplateColumns: "1fr",
    },
  },
});

export const journeySummaryPanel = style({
  display: "flex",
  flexDirection: "column",
  gap: "14px",
  minWidth: 0,
  padding: "14px",
  border: "1px solid #e5e7eb",
  borderRadius: "12px",
  background: "#f8fafc",
});

export const journeyDetailTable = style({
  width: "100%",
  borderCollapse: "collapse",
});

export const journeyDetailRow = style({});

export const journeyDetailLabel = style({
  width: "1%",
  whiteSpace: "nowrap",
  padding: "4px 10px 4px 0",
  color: "#6b7280",
  fontSize: "0.76rem",
  fontWeight: 700,
  textTransform: "uppercase",
  letterSpacing: "0.02em",
  textAlign: "left",
  verticalAlign: "top",
});

export const journeyDetailValue = style({
  padding: "4px 0",
  color: "#111827",
  fontSize: "0.86rem",
  fontWeight: 500,
  overflowWrap: "anywhere",
  verticalAlign: "top",
});

export const journeyExternalLink = style({
  color: "#2563eb",
  textDecoration: "underline",
  ":hover": {
    color: "#1d4ed8",
  },
});

export const journeyDetailSectionHeader = style({
  padding: "12px 0 4px",
  color: "#4b5563",
  fontSize: "0.78rem",
  fontWeight: 700,
  textTransform: "uppercase",
  letterSpacing: "0.03em",
  textAlign: "center",
  borderTop: "1px solid #e5e7eb",
});

export const journeyActionMetadataTable = style({
  width: "100%",
  marginTop: "8px",
});

export const journeyTimeline = style({
  position: "relative",
  minWidth: 0,
  selectors: {
    "&::before": {
      content: '""',
      position: "absolute",
      top: "calc(0.78rem * 0.6)",
      bottom: 0,
      left: "13px",
      width: "2px",
      borderRadius: "999px",
      background: "#dbeafe",
      pointerEvents: "none",
    },
  },
});

export const journeyDayGroup = style({
  display: "flex",
  flexDirection: "column",
  gap: "10px",
  marginBottom: "20px",
});

export const journeyDayHeader = style({
  position: "relative",
  width: "fit-content",
  margin: "0 0 2px 48px",
  color: "#4b5563",
  fontSize: "0.78rem",
  lineHeight: 1.2,
  fontWeight: 700,
  textTransform: "uppercase",
  letterSpacing: "0.03em",
  selectors: {
    "&::before": {
      content: '""',
      position: "absolute",
      top: "50%",
      left: "-34px",
      width: "24px",
      height: "2px",
      borderRadius: "999px",
      background: "#dbeafe",
      transform: "translateY(-50%)",
      pointerEvents: "none",
    },
  },
});

export const journeyAction = style({
  position: "relative",
  zIndex: 1,
  display: "grid",
  gridTemplateColumns: "28px minmax(0, 1fr)",
  gap: "10px",
  alignItems: "start",
});

export const journeyActionIcon = style({
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  width: "28px",
  height: "28px",
  borderRadius: "999px",
  background: "#ffffff",
  color: "#1d4ed8",
  fontSize: "0.72rem",
  fontWeight: 800,
});

export const journeyActionBody = style({
  minWidth: 0,
  padding: "10px 12px",
  border: "1px solid #e5e7eb",
  borderRadius: "10px",
  background: "#ffffff",
});

export const journeyActionTitle = style({
  display: "flex",
  justifyContent: "space-between",
  gap: "12px",
  color: "#111827",
  fontSize: "0.92rem",
  fontWeight: 700,
});

export const journeyActionTime = style({
  flex: "0 0 auto",
  color: "#6b7280",
  fontSize: "0.78rem",
  fontWeight: 600,
});

export const journeyActionSubtitle = style({
  margin: "4px 0 0",
  color: "#4b5563",
  fontSize: "0.84rem",
  overflowWrap: "anywhere",
});

export const journeyActionMeta = style({
  margin: "6px 0 0",
  color: "#6b7280",
  fontSize: "0.76rem",
});

export const journeyStateMessage = style({
  margin: 0,
  color: "#6b7280",
  fontSize: "0.9rem",
});

export const journeyTimelineToggle = style({
  position: "relative",
  zIndex: 1,
  display: "flex",
  justifyContent: "center",
  padding: "4px 0 8px",
});
