import type { JSX } from "solid-js";

/** A 24×24 outline icon drawn in the current text color; children are its paths. */
export default function StrokeIcon(props: { class?: string; children: JSX.Element }) {
  return (
    <svg
      class={props.class}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      {props.children}
    </svg>
  );
}
