import { For, type JSX, Show } from "solid-js";
import { gridChangeClass } from "./changeTone";
import HoverHighlightDescriptions from "./HoverHighlightDescriptions";
import {
  hoverCardDescription,
  hoverCardStatLabel,
  hoverCardStatRow,
  hoverCardStats,
  hoverCardStatValue,
  hoverCardTitle,
} from "./hoverCard.css";
import type { HoverStatLine, MetricHoverContent } from "./metricHoverDetails";
import PeriodComparisonNote from "./PeriodComparisonNote";
import { formatTileChangeRatio } from "./revenueSourcesLogic";

function renderStatValue(line: HoverStatLine): JSX.Element {
  const changeRatio = line.changeRatio;
  if (changeRatio == null || changeRatio === 0) {
    return <>{line.value}</>;
  }
  const changeText = formatTileChangeRatio(changeRatio);
  if (!changeText) {
    return <>{line.value}</>;
  }
  return (
    <>
      {line.value} <span class={gridChangeClass(changeRatio)}>{changeText}</span>
    </>
  );
}

/** Hover card body for an audience row: its stats with period change, and highlights. */
export default function AudienceHoverCardContent(props: {
  title: JSX.Element;
  detail: MetricHoverContent;
}) {
  return (
    <>
      <div class={hoverCardTitle}>{props.title}</div>
      <dl class={hoverCardStats}>
        <For each={props.detail.stats}>
          {(line) => (
            <div class={hoverCardStatRow}>
              <dt class={hoverCardStatLabel}>{line.label}</dt>
              <dd class={hoverCardStatValue}>{renderStatValue(line)}</dd>
            </div>
          )}
        </For>
      </dl>
      <Show when={props.detail.highlightDescriptions}>
        {(lines) => <HoverHighlightDescriptions lines={lines()} />}
      </Show>
      <Show when={props.detail.description}>
        {(description) => <p class={hoverCardDescription}>{description()}</p>}
      </Show>
      <PeriodComparisonNote variant="inline" />
    </>
  );
}
