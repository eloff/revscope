/// <reference types="solid-js" />
import { For, Show } from "solid-js";
import { activeFiltersBar } from "./ActiveFilters.css";
import FilterChip from "./FilterChip";

export type ActiveFilterChip = {
  /** Text shown inside the chip describing the applied filter. */
  label: string;
  /** Removes this filter; wired to the chip's X button. */
  onRemove: () => void;
};

export type ActiveFiltersProps = {
  filters: ActiveFilterChip[];
};

/**
 * Renders the filters currently applied to a tab as removable chips. Tab-scoped
 * filters are passed through to the backend route, so this sits at the top of
 * the tab. It renders nothing (and takes no space) when there are no filters.
 */
export default function ActiveFilters(props: ActiveFiltersProps) {
  return (
    <Show when={props.filters.length > 0}>
      <div class={activeFiltersBar}>
        <For each={props.filters}>{(chip) => <FilterChip {...chip} />}</For>
      </div>
    </Show>
  );
}
