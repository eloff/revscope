import { Show } from "solid-js";
import GridColumnInfoIcon from "./GridColumnInfoIcon";
import { gridNumberCell, gridSortHeaderCell, gridSortHeaderCellRight } from "./grid.css";
import SortHeaderButton, { type SortHeaderButtonProps } from "./SortHeaderButton";

type SortableHeaderCellProps = Omit<SortHeaderButtonProps, "class" | "announceSort"> & {
  info?: { title: string; description: string };
  /** Extra class for the header cell. */
  class?: string;
};

/** A grid column header that sorts by its column when clicked. */
export default function SortableHeaderCell(props: SortableHeaderCellProps) {
  const ariaSort = () => {
    if (props.sortDirection === undefined) {
      return "none";
    }
    return props.sortDirection === "asc" ? "ascending" : "descending";
  };
  const button = (
    <SortHeaderButton
      label={props.label}
      sortDirection={props.sortDirection}
      numeric={props.numeric}
      tooltip={props.tooltip}
      onSort={props.onSort}
    />
  );
  return (
    <th
      aria-sort={ariaSort()}
      class={
        [props.numeric ? gridNumberCell : undefined, props.class].filter(Boolean).join(" ") ||
        undefined
      }
    >
      <Show when={props.info} fallback={button}>
        {(info) => (
          // The wrapper keeps the info icon beside the label; it also shrinks
          // the button to its label, so it is only used when there is an icon.
          <span class={props.numeric ? gridSortHeaderCellRight : gridSortHeaderCell}>
            {button}
            <GridColumnInfoIcon title={info().title} description={info().description} />
          </span>
        )}
      </Show>
    </th>
  );
}
