/// <reference types="solid-js" />
import type { ActiveFilterChip } from "./ActiveFilters";
import {
  activeFilterChip,
  activeFilterChipLabel,
  activeFilterChipRemove,
  activeFilterChipRemoveIcon,
} from "./FilterChip.css";

/** One removable filter chip. */
export default function FilterChip(props: ActiveFilterChip) {
  return (
    <span class={activeFilterChip}>
      <span class={activeFilterChipLabel}>{props.label}</span>
      <button
        type="button"
        class={activeFilterChipRemove}
        aria-label={`Remove filter: ${props.label}`}
        onClick={() => props.onRemove()}
      >
        <svg
          class={activeFilterChipRemoveIcon}
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2.4"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true"
        >
          <path d="M18 6L6 18" />
          <path d="M6 6l12 12" />
        </svg>
      </button>
    </span>
  );
}
