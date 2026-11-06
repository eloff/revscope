import { type JSX, Show } from "solid-js";
import { gridBestMetricValue, gridBestMetricValueStart, gridBestStar } from "./grid.css";
import { gridWorstIcon } from "./icons.css";

interface HighlightedValueProps {
  /** Marks the best value in its column with a star. */
  positive?: boolean;
  /** Marks the worst value in its column with a warning icon. */
  negative?: boolean;
  /** Left-align the icon and value; number cells stay right-aligned. */
  alignStart?: boolean;
  children: JSX.Element;
}

/** A grid cell value, prefixed with a best or worst marker when it stands out. */
export default function HighlightedValue(props: HighlightedValueProps) {
  return (
    <Show when={props.positive || props.negative} fallback={props.children}>
      <span
        class={
          props.alignStart
            ? `${gridBestMetricValue} ${gridBestMetricValueStart}`
            : gridBestMetricValue
        }
      >
        <Show when={props.positive}>
          <span class={gridBestStar} aria-hidden="true">
            ★
          </span>
        </Show>
        <Show when={props.negative}>
          <span class={gridWorstIcon} aria-hidden="true" />
        </Show>
        {props.children}
      </span>
    </Show>
  );
}
