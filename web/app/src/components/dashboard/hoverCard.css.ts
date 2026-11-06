import { style } from "@vanilla-extract/css";

export const hoverCardPositioner = style({
  zIndex: 1000,
});

export const hoverCardContent = style({
  boxSizing: "border-box",
  width: "320px",
  maxWidth: "calc(100vw - 24px)",
  padding: "12px 14px",
  border: "1px solid #e5e7eb",
  borderRadius: "10px",
  background: "#ffffff",
  boxShadow: "0 10px 28px rgba(15, 23, 42, 0.14)",
  fontSize: "0.96rem",
  lineHeight: 1.45,
  color: "#374151",
  overflowWrap: "anywhere",
});

export const hoverCardTitle = style({
  marginBottom: "8px",
  fontSize: "0.984rem",
  fontWeight: 700,
  color: "#111827",
  whiteSpace: "normal",
  overflowWrap: "anywhere",
  wordBreak: "break-word",
});

export const hoverCardStats = style({
  margin: "0 0 10px",
  padding: 0,
});

export const hoverCardStatRow = style({
  display: "flex",
  justifyContent: "space-between",
  alignItems: "baseline",
  gap: "12px",
  marginBottom: "4px",
  selectors: {
    "&:last-child": {
      marginBottom: 0,
    },
  },
});

export const hoverCardStatLabel = style({
  margin: 0,
  flex: "1 1 auto",
  fontWeight: 500,
  color: "#4b5563",
});

export const hoverCardStatValue = style({
  margin: 0,
  flex: "0 0 auto",
  fontWeight: 600,
  color: "#111827",
  textAlign: "right",
  whiteSpace: "nowrap",
});

export const hoverCardDescription = style({
  margin: 0,
  paddingTop: "8px",
  borderTop: "1px solid #f3f4f6",
  fontSize: "0.9rem",
  lineHeight: 1.5,
  color: "#6b7280",
  whiteSpace: "normal",
  overflowWrap: "anywhere",
  wordBreak: "break-word",
});

/** Overview metric tiles use a wider hover card. */
export const overviewMetricHoverCardContent = style([hoverCardContent, { width: "416px" }]);

/** Grid row and treemap hover cards (MetricHoverCard size="revenueSource" and the treemaps). */
export const wideHoverCardContent = style([hoverCardContent, { width: "384px" }]);

// Zero-size anchor for the ark-ui hover card; positioning is driven by the
// cursor via the hover card's getAnchorRect, so this element is never seen.
export const hoverCardAnchor = style({
  position: "absolute",
  top: 0,
  left: 0,
  width: 0,
  height: 0,
  padding: 0,
  margin: 0,
  border: "none",
  background: "transparent",
  pointerEvents: "none",
  opacity: 0,
});
