import { globalStyle, style } from "@vanilla-extract/css";

export const viewport = style({
  width: "100%",
  overflow: "hidden",
  // Collapsing the off-screen slide changes height; don't let the browser
  // anchor the scroll position to a row in the list that just left.
  overflowAnchor: "none",
});

export const track = style({
  display: "flex",
  alignItems: "flex-start",
  width: "100%",
  willChange: "transform",
});

export const trackFast = style({ transition: "transform 0.32s ease" });

export const trackSlow = style({ transition: "transform 750ms ease" });

export const trackShowSecond = style({
  transform: "translateX(-100%)",
});

export const panel = style({
  flex: "0 0 100%",
  minWidth: 0,
  minHeight: 0,
});

// The hidden slide stays in the flex row, so without this the track is as
// tall as the taller sibling and the open panel gets a band of empty space
// under it.
globalStyle(`${track} > :last-child`, {
  height: 0,
  overflow: "hidden",
});

globalStyle(`${track}${trackShowSecond} > :first-child`, {
  height: 0,
  overflow: "hidden",
});

globalStyle(`${track}${trackShowSecond} > :last-child`, {
  height: "auto",
  overflow: "visible",
});
