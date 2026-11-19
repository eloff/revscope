import { SegmentGroup } from "@ark-ui/solid/segment-group";
import { For } from "solid-js";
import {
  indicator,
  item,
  itemControl,
  itemText,
  itemTextSmall,
  root,
} from "./SegmentedControl.css";

export interface SegmentedControlOption<T extends string> {
  value: T;
  label: string;
  disabled?: boolean;
}

interface SegmentedControlProps<T extends string> {
  value: T;
  options: readonly SegmentedControlOption<T>[];
  onChange: (value: T) => void;
  /**
   * Called when the option already selected is picked again, for a control
   * showing a stand-in for the value it sets.
   */
  onReselect?: (value: T) => void;
  ariaLabel?: string;
  size?: "small";
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
          <SegmentGroup.Item
            class={item}
            value={option.value}
            disabled={option.disabled}
            onClick={() => {
              if (option.value === props.value && !option.disabled) {
                props.onReselect?.(option.value);
              }
            }}
          >
            <SegmentGroup.ItemText
              class={props.size === "small" ? `${itemText} ${itemTextSmall}` : itemText}
            >
              {option.label}
            </SegmentGroup.ItemText>
            <SegmentGroup.ItemControl class={itemControl} />
            <SegmentGroup.ItemHiddenInput />
          </SegmentGroup.Item>
        )}
      </For>
    </SegmentGroup.Root>
  );
}
