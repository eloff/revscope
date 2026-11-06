/// <reference types="solid-js" />
import type { JSX } from "solid-js";
import { gridInlineActionLabel } from "./grid.css";
import { gridInlineActionIcon } from "./icons.css";
import { gridInlineActionButton } from "./ViewDetailsButton.css";

type ViewDetailsButtonProps = {
  children: JSX.Element;
  title: string;
  onClick: JSX.EventHandler<HTMLButtonElement, MouseEvent>;
};

export default function ViewDetailsButton(props: ViewDetailsButtonProps) {
  return (
    <button
      type="button"
      class={gridInlineActionButton}
      title={props.title}
      aria-label={props.title}
      onClick={props.onClick}
    >
      <span class={gridInlineActionLabel}>{props.children}</span>
      <span class={gridInlineActionIcon} aria-hidden="true" />
    </button>
  );
}
