import { Show } from "solid-js";
import type { SortDirection } from "./columnSort";
import { gridSortHeader, gridSortHeaderRight, gridSortIndicator } from "./grid.css";
import { visuallyHidden } from "./visuallyHidden.css";

export type SortHeaderButtonProps = {
  label: string;
  /** The sort direction when sorted by this column. */
  sortDirection: SortDirection | undefined;
  numeric?: boolean;
  tooltip?: string;
  /** Replaces the default grid header class. */
  class?: string;
  /**
   * Says the sort direction to screen readers; for headers outside a table,
   * where no `th` carries `aria-sort`.
   */
  announceSort?: boolean;
  onSort: () => void;
};

/** A column label that sorts by its column when clicked, with the active direction's arrow. */
export default function SortHeaderButton(props: SortHeaderButtonProps) {
  return (
    <button
      type="button"
      class={props.class ?? (props.numeric ? gridSortHeaderRight : gridSortHeader)}
      title={props.tooltip}
      onClick={() => props.onSort()}
    >
      <span>{props.label}</span>
      <Show when={props.sortDirection}>
        {(direction) => (
          <span class={gridSortIndicator} aria-hidden="true">
            {direction() === "asc" ? "▲" : "▼"}
          </span>
        )}
      </Show>
      <Show when={props.announceSort && props.sortDirection}>
        {(direction) => (
          <span class={visuallyHidden}>
            {direction() === "asc" ? ", sorted ascending" : ", sorted descending"}
          </span>
        )}
      </Show>
    </button>
  );
}
