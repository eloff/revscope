import { HoverCard } from "@ark-ui/solid/hover-card";
import type { JSX } from "solid-js";
import { Portal } from "solid-js/web";
import type { CursorHover } from "./createCursorHover";
import {
  hoverCardAnchor,
  hoverCardContent,
  hoverCardPositioner,
  wideHoverCardContent,
} from "./hoverCard.css";
import { hoverCardPortalMount } from "./hoverCardPortal";

/**
 * A hover card anchored at the cursor rather than at a trigger element. Callers
 * drive `hover` from their own pointer events; the trigger is a hidden
 * placeholder that ark requires.
 */
export default function CursorHoverCard<K>(props: {
  hover: CursorHover<K>;
  open: boolean;
  wide?: boolean;
  closeDelay?: number;
  children: JSX.Element;
}) {
  return (
    <HoverCard.Root
      open={props.open}
      onOpenChange={(details) => {
        if (!details.open) {
          props.hover.clear();
        }
      }}
      openDelay={0}
      closeDelay={props.closeDelay ?? 0}
      positioning={{
        placement: "top",
        strategy: "fixed",
        getAnchorRect: () => ({
          x: props.hover.x(),
          y: props.hover.y(),
          width: 0,
          height: 0,
        }),
        listeners: { animationFrame: true },
      }}
    >
      <HoverCard.Trigger class={hoverCardAnchor} aria-hidden="true" tabIndex={-1} />
      <Portal mount={hoverCardPortalMount()}>
        <HoverCard.Positioner class={hoverCardPositioner}>
          <HoverCard.Content class={props.wide ? wideHoverCardContent : hoverCardContent}>
            {props.children}
          </HoverCard.Content>
        </HoverCard.Positioner>
      </Portal>
    </HoverCard.Root>
  );
}
