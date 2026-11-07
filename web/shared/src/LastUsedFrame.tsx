/// <reference types="solid-js" />

import { type JSX, Show } from "solid-js";
import { frame, legend } from "./LastUsedFrame.css";

/** Draws a bordered "Last used" frame around its children when `active`. */
export default function LastUsedFrame(props: { active: boolean; children: JSX.Element }) {
  return (
    <div class={props.active ? frame : undefined}>
      <Show when={props.active}>
        <span class={legend}>Last used</span>
      </Show>
      {props.children}
    </div>
  );
}
