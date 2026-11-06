import { globalStyle, style } from "@vanilla-extract/css";

export const funnelChartCard = style({
  minHeight: "540px",
  padding: "8px",
  border: "1px solid #e5e7eb",
  borderRadius: "12px",
  background: "#ffffff",
});

export const funnelChartStack = style({
  display: "flex",
  flexDirection: "column",
  gap: "12px",
});

export const funnelChartMount = style({
  width: "100%",
});

globalStyle(`${funnelChartMount} .apexcharts-bar-area`, {
  cursor: "pointer",
});

export const funnelStepDetailsPanel = style({
  // Reserve room for step stats + drop-off treemap so hover mount/unmount
  // does not grow/shrink the page (and flash the scrollbar).
  minHeight: "286px",
  padding: "14px",
  border: "1px solid #e5e7eb",
  borderRadius: "12px",
  background: "#ffffff",
  boxShadow: "0 1px 2px rgba(15, 23, 42, 0.04)",
  boxSizing: "border-box",
});

export const funnelStepDetailsPlaceholder = style({
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  minHeight: "258px",
  color: "#6b7280",
  fontSize: "0.9rem",
  textAlign: "center",
  boxSizing: "border-box",
});

export const funnelStepDetailsGrid = style({
  display: "grid",
  gridTemplateColumns: "minmax(0, 384px) minmax(0, 1fr)",
  gap: "18px",
  alignItems: "start",
  "@media": {
    "screen and (max-width: 900px)": {
      gridTemplateColumns: "1fr",
    },
  },
});

export const funnelStepDetailColumn = style({
  minWidth: 0,
});

export const funnelDropoffHeading = style({
  margin: "0 0 6px",
  color: "#374151",
  fontSize: "0.82rem",
  fontWeight: 700,
});
