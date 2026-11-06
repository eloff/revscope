import type { JSX } from "solid-js";
import { panel, track, trackFast, trackShowSecond, trackSlow, viewport } from "./SlidePanels.css";

interface SlidePanelsProps {
  showSecond: boolean;
  /** "fast" for list → detail drill-ins, "slow" for switching between modes. */
  speed: "fast" | "slow";
  first: JSX.Element;
  second: JSX.Element;
}

/** Two full-width panels side by side; slides horizontally to reveal the second. */
export default function SlidePanels(props: SlidePanelsProps) {
  return (
    <div class={viewport}>
      <div
        class={`${track} ${props.speed === "fast" ? trackFast : trackSlow} ${
          props.showSecond ? trackShowSecond : ""
        }`}
      >
        <div class={panel}>{props.first}</div>
        <div class={panel}>{props.second}</div>
      </div>
    </div>
  );
}
