/// <reference types="solid-js" />
import { createEffect, createSignal, For, Index, onCleanup, Show } from "solid-js";
import type { CampaignPreviewResponse, CampaignRule } from "../../lib/campaignsApi";
import {
  addConditionButton,
  conditionsBlock,
  editor,
  editorActions,
  editorTitle,
  equalsText,
  field,
  fieldLabel,
  mutedText,
  orDivider,
  previewBox,
  primaryButton,
  removeButton,
  ruleRow,
  secondaryButton,
  textInput,
} from "./Campaigns.css";
import { paramLabel, RULE_PARAM_KEYS } from "./campaignLabels";
import { errorMessage } from "./messages.css";

const MAX_CONDITIONS = 8;

export type DraftRule = {
  paramKey: string;
  paramValue: string;
};

export type CampaignDraft = {
  id?: number;
  name: string;
  rules: DraftRule[];
  /** Auto-detected UTM campaigns can be renamed but have no rule to edit. */
  renameOnly?: boolean;
};

type CampaignEditorProps = {
  draft: CampaignDraft;
  saving: boolean;
  readOnly: boolean;
  error: string;
  onChange: (draft: CampaignDraft) => void;
  onCancel: () => void;
  onSave: () => void;
  previewCampaign: (rules: CampaignRule[]) => Promise<CampaignPreviewResponse>;
};

export function emptyRule(): DraftRule {
  return { paramKey: "source", paramValue: "" };
}

export function rulesFromCampaign(rules: CampaignRule[]): DraftRule[] {
  if (rules.length === 0) {
    return [emptyRule()];
  }
  return rules.map((rule) => ({ paramKey: rule.param_key, paramValue: rule.param_value }));
}

export function draftRules(rules: DraftRule[]): CampaignRule[] {
  return rules.flatMap((rule) => {
    const paramValue = rule.paramValue.trim();
    return paramValue ? [{ param_key: rule.paramKey, param_value: paramValue }] : [];
  });
}

export default function CampaignEditor(props: CampaignEditorProps) {
  const [preview, setPreview] = createSignal<CampaignPreviewResponse>();
  const [previewError, setPreviewError] = createSignal("");

  createEffect(() => {
    const rules = props.draft.renameOnly ? [] : draftRules(props.draft.rules);
    if (rules.length === 0) {
      setPreview(undefined);
      setPreviewError("");
      return;
    }
    let cancelled = false;
    const timer = window.setTimeout(() => {
      void props
        .previewCampaign(rules)
        .then((response) => {
          if (!cancelled) {
            setPreview(response);
            setPreviewError("");
          }
        })
        .catch((error: unknown) => {
          if (!cancelled) {
            setPreview(undefined);
            setPreviewError(error instanceof Error ? error.message : "Unable to preview.");
          }
        });
    }, 350);
    onCleanup(() => {
      cancelled = true;
      window.clearTimeout(timer);
    });
  });

  const updateRule = (index: number, patch: Partial<DraftRule>) => {
    props.onChange({
      ...props.draft,
      rules: props.draft.rules.map((rule, ruleIndex) =>
        ruleIndex === index ? { ...rule, ...patch } : rule,
      ),
    });
  };

  const title = () => {
    if (props.draft.renameOnly) {
      return "Rename campaign";
    }
    return props.draft.id ? "Edit campaign" : "New campaign";
  };

  return (
    <form
      class={editor}
      onSubmit={(event) => {
        event.preventDefault();
        props.onSave();
      }}
    >
      <h3 class={editorTitle}>{title()}</h3>
      <div class={field}>
        <label class={fieldLabel} for="campaign-name">
          Name
        </label>
        <input
          id="campaign-name"
          class={textInput}
          value={props.draft.name}
          maxlength={80}
          placeholder="e.g. BetaList launch"
          onInput={(event) => props.onChange({ ...props.draft, name: event.currentTarget.value })}
        />
      </div>
      <Show when={!props.draft.renameOnly}>
        <div class={conditionsBlock}>
          <span class={fieldLabel}>
            {props.draft.rules.length > 1
              ? "Match visits that landed with any of these"
              : "Match visits that landed with"}
          </span>
          <Index each={props.draft.rules}>
            {(rule, index) => (
              <>
                <Show when={index > 0}>
                  <div class={orDivider}>OR</div>
                </Show>
                <div class={ruleRow}>
                  <select
                    class={textInput}
                    aria-label="Query parameter"
                    value={rule().paramKey}
                    onChange={(event) => updateRule(index, { paramKey: event.currentTarget.value })}
                  >
                    <For each={RULE_PARAM_KEYS}>
                      {(key) => <option value={key}>{paramLabel(key)}</option>}
                    </For>
                  </select>
                  <span class={equalsText}>equals</span>
                  <input
                    class={textInput}
                    aria-label="Parameter value"
                    placeholder="e.g. betalist"
                    value={rule().paramValue}
                    onInput={(event) =>
                      updateRule(index, { paramValue: event.currentTarget.value })
                    }
                  />
                  <button
                    type="button"
                    class={removeButton}
                    aria-label="Remove condition"
                    title="Remove condition"
                    disabled={props.draft.rules.length === 1}
                    onClick={() =>
                      props.onChange({
                        ...props.draft,
                        rules: props.draft.rules.filter((_, ruleIndex) => ruleIndex !== index),
                      })
                    }
                  >
                    ×
                  </button>
                </div>
              </>
            )}
          </Index>
          <button
            type="button"
            class={addConditionButton}
            disabled={props.draft.rules.length >= MAX_CONDITIONS}
            onClick={() =>
              props.onChange({ ...props.draft, rules: [...props.draft.rules, emptyRule()] })
            }
          >
            + Add OR condition
          </button>
          <p class={mutedText}>
            Values are case-insensitive. Only matches visits not already part of a campaign.
          </p>
        </div>
        <Show when={preview()}>
          {(current) => (
            <div class={previewBox}>
              <strong>{current().unclassified.toLocaleString()}</strong> visits in this period would
              join this campaign ({current().direct.toLocaleString()} direct,{" "}
              {current().referrer.toLocaleString()} with a referrer).
              <Show when={current().already_classified > 0}>
                {" "}
                {current().already_classified.toLocaleString()} more match but already belong to
                another campaign.
              </Show>
            </div>
          )}
        </Show>
        <Show when={previewError()}>
          <p class={mutedText}>{previewError()}</p>
        </Show>
      </Show>
      <Show when={props.error}>
        <p class={errorMessage}>{props.error}</p>
      </Show>
      <div class={editorActions}>
        <button type="submit" class={primaryButton} disabled={props.saving || props.readOnly}>
          {props.saving ? "Saving…" : "Save campaign"}
        </button>
        <button type="button" class={secondaryButton} onClick={() => props.onCancel()}>
          Cancel
        </button>
        <Show when={props.readOnly}>
          <span class={mutedText}>The demo project is read-only.</span>
        </Show>
      </div>
    </form>
  );
}
