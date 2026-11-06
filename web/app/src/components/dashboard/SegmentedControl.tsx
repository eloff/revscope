import { SegmentGroup } from "@ark-ui/solid/segment-group";
import { For } from "solid-js";
import { indicator, item, itemControl, itemText, root } from "./SegmentedControl.css";

export interface SegmentedControlOption<T extends string> {
  value: T;
  label: string;
}

interface SegmentedControlProps<T extends string> {
  value: T;
  options: readonly SegmentedControlOption<T>[];
  onChange: (value: T) => void;
  ariaLabel?: string;
}

/** Pill-shaped single-choice toggle between a few options. */
export default function SegmentedControl<T extends string>(props: SegmentedControlProps<T>) {
  return (
    <SegmentGroup.Root
      class={root}
      value={props.value}
      aria-label={props.ariaLabel}
      onValueChange={(details) => {
        const option = props.options.find((candidate) => candidate.value === details.value);
        if (option) {
          props.onChange(option.value);
        }
      }}
    >
      <SegmentGroup.Indicator class={indicator} />
      <For each={props.options}>
        {(option) => (
          <SegmentGroup.Item class={item} value={option.value}>
            <SegmentGroup.ItemText class={itemText}>{option.label}</SegmentGroup.ItemText>
            <SegmentGroup.ItemControl class={itemControl} />
            <SegmentGroup.ItemHiddenInput />
          </SegmentGroup.Item>
        )}
      </For>
    </SegmentGroup.Root>
  );
}
