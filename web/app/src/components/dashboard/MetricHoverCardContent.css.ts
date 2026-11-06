import { style } from "@vanilla-extract/css";

export const hoverCardStatRowThreeCol = style({
  display: "grid",
  gridTemplateColumns: "minmax(0, 1fr) auto auto",
  alignItems: "baseline",
  columnGap: "12px",
  marginBottom: "4px",
  selectors: {
    "&:last-child": {
      marginBottom: 0,
    },
  },
});

export const hoverCardStatValuePositive = style({
  color: "#15803d",
});

export const hoverCardStatValueNegative = style({
  color: "#b91c1c",
});

export const hoverCardStatValueNeutral = style({
  color: "#6b7280",
});

export const hoverCardLandingPathsSection = style({
  marginTop: "4px",
  paddingTop: "8px",
  borderTop: "1px solid #f3f4f6",
});

export const hoverCardStatSectionHeader = style({
  margin: 0,
  fontWeight: 500,
  color: "#4b5563",
});

export const hoverCardLandingPathsHeaderRow = style({
  marginBottom: "4px",
});

export const hoverCardLandingPathsVisitorsHeader = style({
  textAlign: "right",
});

export const hoverCardStatChange = style({
  margin: 0,
  minWidth: "5.1rem",
  fontWeight: 600,
  textAlign: "right",
  whiteSpace: "nowrap",
});
