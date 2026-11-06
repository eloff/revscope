/// <reference types="solid-js" />
import { For, Show } from "solid-js";
import { gridGroupBadge } from "./grid.css";
import {
  hoverCardDescriptionHighlights,
  hoverCardDescriptionRow,
  hoverCardDescriptionStar,
} from "./HoverHighlightDescriptions.css";
import { hoverCardDescription } from "./hoverCard.css";
import { hoverHighlightNegativeIcon } from "./icons.css";
import { type HoverHighlight, highlightMetricLabel, highlightTone } from "./metricHoverDetails";
import { SOURCE_GROUP_COLORS, SOURCE_GROUP_LABELS, type SourceGroup } from "./revenueSourcesLogic";

function sourceGroupBadgeStyle(group: SourceGroup): {
  background: string;
  color: string;
} {
  return {
    background: SOURCE_GROUP_COLORS[group],
    color: "#ffffff",
  };
}

function highlightDescription(highlight: HoverHighlight): string {
  return "description" in highlight
    ? highlight.description
    : highlightMetricLabel(highlight.metric);
}

function highlightIconClass(highlight: HoverHighlight): string | undefined {
  return "description" in highlight ? highlight.iconClass : undefined;
}

function HighlightIcon(props: { highlight: HoverHighlight }) {
  const customClass = () => highlightIconClass(props.highlight);
  return (
    <Show
      when={customClass()}
      fallback={
        <Show
          when={highlightTone(props.highlight) === "negative"}
          fallback={
            <span class={hoverCardDescriptionStar} aria-hidden="true">
              ★
            </span>
          }
        >
          <span class={hoverHighlightNegativeIcon} aria-hidden="true" />
        </Show>
      }
    >
      {(iconClass) => <span class={iconClass()} aria-hidden="true" />}
    </Show>
  );
}

export default function HoverHighlightDescriptions(props: { lines: HoverHighlight[] }) {
  return (
    <div class={hoverCardDescription}>
      <div class={hoverCardDescriptionHighlights}>
        <For each={props.lines}>
          {(highlight) => (
            <span class={hoverCardDescriptionRow}>
              <HighlightIcon highlight={highlight} />
              <span>
                {highlightDescription(highlight)}
                <Show when={"sourceGroup" in highlight ? highlight.sourceGroup : undefined}>
                  {(group) => (
                    <>
                      {" in "}
                      <span class={gridGroupBadge} style={sourceGroupBadgeStyle(group())}>
                        {SOURCE_GROUP_LABELS[group()]}
                      </span>
                    </>
                  )}
                </Show>
              </span>
            </span>
          )}
        </For>
      </div>
    </div>
  );
}
