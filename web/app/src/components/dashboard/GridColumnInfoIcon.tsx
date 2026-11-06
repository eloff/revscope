/// <reference types="solid-js" />
import { HoverCard } from "@ark-ui/solid/hover-card";
import { Show } from "solid-js";
import { Portal } from "solid-js/web";
import {
  gridColumnInfoButton,
  gridColumnInfoIcon,
  gridColumnWarningButton,
} from "./GridColumnInfoIcon.css";
import {
  hoverCardContent,
  hoverCardDescription,
  hoverCardPositioner,
  hoverCardTitle,
} from "./hoverCard.css";

export type GridColumnInfoIconProps = {
  title: string;
  description: string;
  /** `warning` flags a caveat about the column's current values. */
  tone?: "info" | "warning";
};

export default function GridColumnInfoIcon(props: GridColumnInfoIconProps) {
  const warning = () => props.tone === "warning";
  return (
    <HoverCard.Root
      openDelay={300}
      closeDelay={150}
      positioning={{ placement: "top", strategy: "fixed" }}
    >
      <HoverCard.Trigger
        class={warning() ? gridColumnWarningButton : gridColumnInfoButton}
        aria-label={warning() ? `Warning: ${props.title}` : `About ${props.title}`}
      >
        <svg
          class={gridColumnInfoIcon}
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true"
        >
          <Show
            when={warning()}
            fallback={
              <>
                <circle cx="12" cy="12" r="10" />
                <path d="M12 16v-4" />
                <path d="M12 8h.01" />
              </>
            }
          >
            <path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
            <path d="M12 9v4" />
            <path d="M12 17h.01" />
          </Show>
        </svg>
      </HoverCard.Trigger>
      <Portal>
        <HoverCard.Positioner class={hoverCardPositioner}>
          <HoverCard.Content class={hoverCardContent}>
            <div class={hoverCardTitle}>{props.title}</div>
            <p class={hoverCardDescription}>{props.description}</p>
          </HoverCard.Content>
        </HoverCard.Positioner>
      </Portal>
    </HoverCard.Root>
  );
}
