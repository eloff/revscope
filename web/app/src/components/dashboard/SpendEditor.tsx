/// <reference types="solid-js" />
import { createSignal, Index, Show } from "solid-js";
import type { CampaignSpendInput, CampaignSpendKind } from "../../lib/campaignsApi";
import { primaryButton, secondaryButton } from "../buttons.css";
import {
  addConditionButton,
  editorActions,
  equalsText,
  mutedText,
  removeButton,
  spendAmountField,
  spendAmountInput,
  spendDateInput,
  spendForm,
  spendKindInput,
  spendRow,
} from "./Campaigns.css";
import {
  emptySpendRow,
  MAX_SPEND_ENTRIES,
  type SpendDraftRow,
  validateSpendRows,
} from "./campaignSpendLogic";
import { errorMessage } from "./messages.css";

type SpendEditorProps = {
  initialRows: SpendDraftRow[];
  readOnly: boolean;
  onCancel: () => void;
  onSave: (entries: CampaignSpendInput[]) => Promise<void>;
};

export default function SpendEditor(props: SpendEditorProps) {
  const [rows, setRows] = createSignal<SpendDraftRow[]>(
    props.initialRows.length > 0 ? props.initialRows : [emptySpendRow()],
  );
  const [error, setError] = createSignal("");
  const [invalidRow, setInvalidRow] = createSignal<number | null>(null);
  const [saving, setSaving] = createSignal(false);

  const updateRow = (index: number, patch: Partial<SpendDraftRow>) => {
    setRows((current) => current.map((row, i) => (i === index ? { ...row, ...patch } : row)));
    setInvalidRow(null);
    setError("");
  };

  const removeRow = (index: number) => {
    setRows((current) => current.filter((_, i) => i !== index));
    setInvalidRow(null);
    setError("");
  };

  const save = async () => {
    if (props.readOnly) {
      return;
    }
    const result = validateSpendRows(rows());
    if (!result.ok) {
      setError(result.error);
      setInvalidRow(result.row ?? null);
      return;
    }
    setSaving(true);
    setError("");
    try {
      await props.onSave(result.entries);
    } catch (saveError) {
      setError(saveError instanceof Error ? saveError.message : "Unable to save spend.");
    } finally {
      setSaving(false);
    }
  };

  return (
    <form
      class={spendForm}
      onSubmit={(event) => {
        event.preventDefault();
        void save();
      }}
    >
      <Index each={rows()}>
        {(row, index) => (
          <div class={spendRow} data-invalid={invalidRow() === index}>
            <select
              class={spendKindInput}
              aria-label="Spend type"
              value={row().kind}
              onChange={(event) =>
                updateRow(index, { kind: event.currentTarget.value as CampaignSpendKind })
              }
            >
              <option value="once">Single amount</option>
              <option value="daily">Daily rate</option>
            </select>
            <span class={equalsText}>{row().kind === "daily" ? "from" : "on"}</span>
            <input
              type="date"
              class={spendDateInput}
              aria-label={row().kind === "daily" ? "Start date" : "Date"}
              value={row().startDate}
              onInput={(event) => updateRow(index, { startDate: event.currentTarget.value })}
            />
            <Show when={row().kind === "daily"}>
              <span class={equalsText}>through</span>
              <input
                type="date"
                class={spendDateInput}
                aria-label="End date"
                value={row().endDate}
                min={row().startDate || undefined}
                onInput={(event) => updateRow(index, { endDate: event.currentTarget.value })}
              />
            </Show>
            <span class={spendAmountField}>
              <span class={equalsText}>$</span>
              <input
                class={spendAmountInput}
                inputmode="decimal"
                aria-label={row().kind === "daily" ? "Amount per day" : "Amount"}
                placeholder="0.00"
                value={row().amount}
                onInput={(event) => updateRow(index, { amount: event.currentTarget.value })}
              />
              <Show when={row().kind === "daily"}>
                <span class={equalsText}>/ day</span>
              </Show>
            </span>
            <button
              type="button"
              class={removeButton}
              aria-label="Remove spend"
              title="Remove spend"
              onClick={() => removeRow(index)}
            >
              ×
            </button>
          </div>
        )}
      </Index>
      <button
        type="button"
        class={addConditionButton}
        disabled={rows().length >= MAX_SPEND_ENTRIES}
        onClick={() => setRows((current) => [...current, emptySpendRow()])}
      >
        + Add spend
      </button>
      <p class={mutedText}>
        Dates use the project timezone. Spend only counts once a day has finished, so today is left
        out. Daily ranges can't overlap.
      </p>
      <Show when={error()}>
        <p class={errorMessage}>{error()}</p>
      </Show>
      <div class={editorActions}>
        <button type="submit" class={primaryButton} disabled={saving() || props.readOnly}>
          {saving() ? "Saving…" : "Save spend"}
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
