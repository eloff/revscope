import { For, Show } from "solid-js";
import type { FunnelOptionsResponse } from "../../lib/funnelsApi";
import {
  funnelCheckboxRow,
  funnelStepActions,
  funnelStepBadge,
  funnelStepCard,
  funnelStepFields,
  funnelStepFullWidth,
  funnelStepIconButton,
  funnelStepIconButtonDanger,
} from "./FunnelPageStepCard.css";
import FunnelStepVisitorCount from "./FunnelStepVisitorCount";
import { type EditorStep, eventOptionsForPath, selectOptionValue } from "./funnelEditorSteps";
import {
  funnelField,
  funnelFieldLabel,
  funnelInput,
  funnelSelect,
  funnelStepHeader,
  funnelStepHeaderInfo,
  funnelStepIcon,
} from "./funnelForm.css";
import StrokeIcon from "./StrokeIcon";

interface FunnelPageStepCardProps {
  step: EditorStep;
  index: number;
  stepCount: number;
  options: FunnelOptionsResponse;
  previewVisitors: number | undefined;
  previewLoading: boolean;
  onChange: (update: Partial<EditorStep>) => void;
  onMove: (direction: -1 | 1) => void;
  onRemove: () => void;
}

/** Editor card for one pageview step: page, optional event, and description. */
export default function FunnelPageStepCard(props: FunnelPageStepCardProps) {
  return (
    <div class={funnelStepCard}>
      <div class={funnelStepHeader}>
        <div class={funnelStepHeaderInfo}>
          <span class={funnelStepBadge}>{props.index + 1}</span>
          <FunnelStepVisitorCount visitors={props.previewVisitors} loading={props.previewLoading} />
        </div>
        <div class={funnelStepActions}>
          <button
            type="button"
            class={funnelStepIconButton}
            aria-label="Move step up"
            title="Move step up"
            onClick={() => props.onMove(-1)}
            disabled={props.index === 0}
          >
            <StrokeIcon class={funnelStepIcon}>
              <path d="m18 15-6-6-6 6" />
            </StrokeIcon>
          </button>
          <button
            type="button"
            class={funnelStepIconButton}
            aria-label="Move step down"
            title="Move step down"
            onClick={() => props.onMove(1)}
            disabled={props.index === props.stepCount - 1}
          >
            <StrokeIcon class={funnelStepIcon}>
              <path d="m6 9 6 6 6-6" />
            </StrokeIcon>
          </button>
          <button
            type="button"
            class={`${funnelStepIconButton} ${funnelStepIconButtonDanger}`}
            aria-label="Remove step"
            title="Remove step"
            onClick={() => props.onRemove()}
            disabled={props.stepCount <= 1}
          >
            <StrokeIcon class={funnelStepIcon}>
              <path d="M18 6L6 18" />
              <path d="M6 6l12 12" />
            </StrokeIcon>
          </button>
        </div>
      </div>
      <div class={funnelStepFields}>
        <label class={funnelField}>
          <span class={funnelFieldLabel}>Page</span>
          <select
            class={funnelSelect}
            value={selectOptionValue(props.step.path_id)}
            onChange={(event) => props.onChange({ path_id: Number(event.currentTarget.value) })}
          >
            <For each={props.options.paths}>
              {(path) => <option value={selectOptionValue(path.id)}>{path.name}</option>}
            </For>
          </select>
        </label>
        <label class={funnelField}>
          <span class={funnelFieldLabel}>Event (optional)</span>
          <select
            class={funnelSelect}
            value={selectOptionValue(props.step.event_name_id)}
            onChange={(event) =>
              props.onChange({
                event_name_id: event.currentTarget.value
                  ? Number(event.currentTarget.value)
                  : undefined,
              })
            }
          >
            <option value="">No event</option>
            <For each={eventOptionsForPath(props.options, props.step.path_id)}>
              {(eventName) => (
                <option value={selectOptionValue(eventName.id)}>{eventName.name}</option>
              )}
            </For>
          </select>
        </label>
        <label class={funnelField}>
          <span class={funnelFieldLabel}>Description</span>
          <input
            class={funnelInput}
            value={props.step.description}
            placeholder="e.g. Visited pricing page"
            onInput={(event) => props.onChange({ description: event.currentTarget.value })}
          />
        </label>

        <Show when={props.index === 0}>
          <label class={`${funnelCheckboxRow} ${funnelStepFullWidth}`}>
            <input
              type="checkbox"
              checked={props.step.is_landing}
              onChange={(event) => props.onChange({ is_landing: event.currentTarget.checked })}
            />
            Only count sessions that start on this page
          </label>
        </Show>
      </div>
    </div>
  );
}
