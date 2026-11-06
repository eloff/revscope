/// <reference types="solid-js" />
import { Show } from "solid-js";
import type { MetricChange } from "../../lib/dashboardApi";
import { changeToneForRatio, metricChangeClass } from "./changeTone";
import { metricLabel, metricTile, metricUnit, metricValue, metricValueRow } from "./MetricTile.css";
import { FLAT_CHANGE_TEXT, formatTileChangeRatio } from "./revenueSourcesLogic";

export type MetricTileProps = {
  label: string;
  value: string;
  unit?: string;
  /** Backend-computed, normalized period-over-period change. */
  change?: MetricChange;
  higherIsBetter?: boolean;
  loading?: boolean;
  /** Chart series color; border uses this when `active`. */
  accentColor?: string;
  active?: boolean;
};

export default function MetricTile(props: MetricTileProps) {
  const higherIsBetter = () => props.higherIsBetter ?? true;
  const ratio = () => {
    const value = props.change?.ratio;
    return value !== undefined && value !== null && Number.isFinite(value) ? value : null;
  };
  const changeText = () => formatTileChangeRatio(ratio());
  const tone = () => (changeText() ? changeToneForRatio(ratio(), higherIsBetter()) : "neutral");

  return (
    <div
      class={metricTile}
      style={{
        "border-color": props.active ? (props.accentColor ?? "#2563eb") : "transparent",
      }}
    >
      <div class={metricLabel}>{props.label}</div>
      <div class={metricValueRow}>
        <span class={metricValue}>{props.loading ? "—" : props.value}</span>
        <Show when={props.unit}>
          <span class={metricUnit}>{props.unit}</span>
        </Show>
      </div>
      <Show when={!props.loading && ratio() !== null}>
        <span class={metricChangeClass(tone())}>{changeText() || FLAT_CHANGE_TEXT}</span>
      </Show>
    </div>
  );
}
