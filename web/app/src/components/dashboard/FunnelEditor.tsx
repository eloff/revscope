import { createEffect, createMemo, createSignal, For, Index, on, onCleanup, Show } from "solid-js";
import { formatPercentRatio } from "../../lib/dashboardApi";
import type {
  Funnel,
  FunnelDataResponse,
  FunnelOptionsResponse,
  FunnelPreviewRequest,
  SaveFunnelStep,
} from "../../lib/funnelsApi";
import { sourceFilterParam } from "../../lib/segmentFilters";
import { primaryButton, secondaryButton } from "../buttons.css";
import { useDashboardState } from "./dashboardPeriodContext";
import {
  funnelAddButton,
  funnelChip,
  funnelChipGroup,
  funnelChipInput,
  funnelChipLabel,
  funnelEditor,
  funnelFormGrid,
  funnelPaymentStepCard,
  funnelPaymentStepFields,
  funnelPaymentStepTitle,
  funnelSectionLabel,
  funnelStepConnector,
  funnelStepConnectorBadge,
  funnelStepConnectorLine,
  funnelStepConnectorValue,
  funnelStepList,
  funnelToggleHint,
  funnelToggleInput,
  funnelToggleRow,
  funnelToggleText,
  funnelToggleThumb,
  funnelToggleTitle,
  funnelToggleTrack,
} from "./FunnelEditor.css";
import FunnelPageStepCard from "./FunnelPageStepCard";
import FunnelStepVisitorCount from "./FunnelStepVisitorCount";
import { funnelActions, funnelDescription, funnelHeader, funnelTitle } from "./Funnels.css";
import {
  type EditorStep,
  editorStepIsComplete,
  editorStepsAreComplete,
  editorStepsFromFunnel,
  editorStepsToSaveSteps,
  emptyPageStep,
  emptyPaymentStep,
  fillMissingPaths,
  movePageStep,
  PAYMENT_TYPES,
  togglePaymentType,
  updatePageStep,
} from "./funnelEditorSteps";
import {
  funnelField,
  funnelFieldLabel,
  funnelInput,
  funnelStepHeader,
  funnelStepHeaderInfo,
  funnelStepIcon,
  funnelTextarea,
} from "./funnelForm.css";
import { hoverHighlightRevenueIcon } from "./icons.css";
import { errorMessage, message } from "./messages.css";
import StrokeIcon from "./StrokeIcon";

const PREVIEW_DEBOUNCE_MS = 300;

export type FunnelDraft = {
  name: string;
  description?: string;
  steps: SaveFunnelStep[];
};

interface FunnelEditorProps {
  /** The funnel to edit; omitted when creating a new one. */
  funnel?: Funnel;
  options?: FunnelOptionsResponse;
  optionsLoading: boolean;
  optionsError: string;
  fetchPreview: (request: FunnelPreviewRequest) => Promise<FunnelDataResponse>;
  onCancel: () => void;
  /** Rejects with the error to show when the save fails. */
  onSave: (draft: FunnelDraft) => Promise<void>;
}

export default function FunnelEditor(props: FunnelEditorProps) {
  const { range, rangeReady, previousRange, segments } = useDashboardState();
  const initialSteps = props.funnel
    ? editorStepsFromFunnel(props.funnel.steps, props.options)
    : { pageSteps: [emptyPageStep(props.options)] };
  const [name, setName] = createSignal(props.funnel?.name ?? "New funnel");
  const [description, setDescription] = createSignal(props.funnel?.description ?? "");
  const [pageSteps, setPageSteps] = createSignal<EditorStep[]>(initialSteps.pageSteps);
  // The optional trailing payment step, kept separate so it always stays last.
  const [paymentStep, setPaymentStep] = createSignal<EditorStep | undefined>(
    initialSteps.paymentStep,
  );
  const [saving, setSaving] = createSignal(false);
  const [saveError, setSaveError] = createSignal("");
  const [previewData, setPreviewData] = createSignal<FunnelDataResponse>();
  const [previewLoading, setPreviewLoading] = createSignal(false);
  let previewToken = 0;

  // The full ordered step list the editor evaluates and saves.
  const allSteps = createMemo<EditorStep[]>(() => {
    const payment = paymentStep();
    return payment ? [...pageSteps(), payment] : pageSteps();
  });

  createEffect(
    on(
      () => props.options,
      (options) => {
        if (options) {
          setPageSteps((steps) => fillMissingPaths(steps, options));
        }
      },
      // Only options that arrive after opening; a saved funnel opened with
      // options in hand keeps its steps exactly as stored.
      { defer: true },
    ),
  );

  const loadPreview = async (request: FunnelPreviewRequest) => {
    const token = ++previewToken;
    setPreviewLoading(true);
    try {
      const response = await props.fetchPreview(request);
      if (token === previewToken) {
        setPreviewData(response);
      }
    } catch {
      // Preview is a non-critical enhancement; on failure we simply hide the
      // per-step counts rather than surfacing an error in the editor.
      if (token === previewToken) {
        setPreviewData(undefined);
      }
    } finally {
      if (token === previewToken) {
        setPreviewLoading(false);
      }
    }
  };

  createEffect(() => {
    const steps = allSteps();
    const current = range();
    const previous = previousRange();
    if (!rangeReady()) {
      return;
    }
    if (!editorStepsAreComplete(steps)) {
      // Discard any in-flight response and clear stale counts.
      previewToken += 1;
      setPreviewLoading(false);
      setPreviewData(undefined);
      return;
    }
    const filters = segments();
    const request: FunnelPreviewRequest = {
      steps: editorStepsToSaveSteps(steps),
      start: current.start,
      end: current.end,
      previous_start: previous.start,
      previous_end: previous.end,
      countries: filters.countries.map((country) => country.code),
      sources: filters.sources.map(sourceFilterParam),
      pages: filters.pages.map((page) => page.id),
      landing_pages: filters.landingPages.map((page) => page.id),
      events: filters.events.map((event) => event.id),
      attributes: filters.attributes.map((filter) => ({
        dimension: filter.attribute,
        value: filter.value,
      })),
    };
    const handle = setTimeout(() => void loadPreview(request), PREVIEW_DEBOUNCE_MS);
    onCleanup(() => clearTimeout(handle));
  });

  const previewVisitorsAt = (index: number): number | undefined =>
    previewData()?.steps.find((step) => step.order === index)?.visitors;

  /** Share of the previous step's visitors that reached this step. */
  const conversionInto = (index: number): string | undefined => {
    const from = previewVisitorsAt(index - 1);
    const to = previewVisitorsAt(index);
    if (from === undefined || to === undefined || from <= 0) {
      return undefined;
    }
    return formatPercentRatio(to / from, 1);
  };

  const save = async () => {
    const trimmedName = name().trim();
    if (!trimmedName) {
      setSaveError("Name the funnel before saving.");
      return;
    }
    const steps = allSteps();
    if (steps.length === 0) {
      setSaveError("Add at least one funnel step.");
      return;
    }
    if (!steps.every(editorStepIsComplete)) {
      setSaveError("Each step needs a page, and purchase needs at least one payment type.");
      return;
    }
    setSaving(true);
    setSaveError("");
    try {
      await props.onSave({
        name: trimmedName,
        description: description().trim() || undefined,
        steps: editorStepsToSaveSteps(steps),
      });
    } catch (error) {
      setSaveError(error instanceof Error ? error.message : "Unable to save funnel.");
    } finally {
      setSaving(false);
    }
  };

  const paymentStepIndex = () => pageSteps().length;

  return (
    <div class={funnelEditor}>
      <div class={funnelHeader}>
        <div>
          <h2 class={funnelTitle}>{props.funnel ? "Edit funnel" : "Create funnel"}</h2>
          <p class={funnelDescription}>
            Every step targets a page, with an optional event on that page. Turn on purchase to
            measure revenue.
          </p>
        </div>
        <div class={funnelActions}>
          <button
            type="button"
            class={secondaryButton}
            onClick={props.onCancel}
            disabled={saving()}
          >
            Cancel
          </button>
          <button
            type="button"
            class={primaryButton}
            onClick={() => void save()}
            disabled={saving() || props.optionsLoading}
          >
            {saving() ? "Saving..." : "Save funnel"}
          </button>
        </div>
      </div>

      <Show when={props.optionsError}>{(error) => <p class={errorMessage}>{error()}</p>}</Show>
      <Show when={saveError()}>{(error) => <p class={errorMessage}>{error()}</p>}</Show>

      <div class={funnelFormGrid}>
        <label class={funnelField}>
          <span class={funnelFieldLabel}>Name</span>
          <input
            class={funnelInput}
            value={name()}
            onInput={(event) => setName(event.currentTarget.value)}
          />
        </label>
        <label class={funnelField}>
          <span class={funnelFieldLabel}>Description</span>
          <textarea
            class={funnelTextarea}
            value={description()}
            onInput={(event) => setDescription(event.currentTarget.value)}
            placeholder="Optional notes for this funnel"
          />
        </label>
      </div>

      <Show
        when={props.options}
        fallback={
          <Show when={props.optionsLoading}>
            <p class={message}>Loading events and pages...</p>
          </Show>
        }
      >
        {(options) => (
          <>
            <h3 class={funnelSectionLabel}>Steps</h3>
            <div class={funnelStepList}>
              <Index each={pageSteps()}>
                {(step, index) => (
                  <>
                    <Show when={index > 0}>
                      <StepConnector rate={conversionInto(index)} />
                    </Show>
                    <FunnelPageStepCard
                      step={step()}
                      index={index}
                      stepCount={pageSteps().length}
                      options={options()}
                      previewVisitors={previewVisitorsAt(index)}
                      previewLoading={previewLoading()}
                      onChange={(update) =>
                        setPageSteps((steps) =>
                          updatePageStep(steps, step().localId, update, options()),
                        )
                      }
                      onMove={(direction) =>
                        setPageSteps((steps) => movePageStep(steps, step().localId, direction))
                      }
                      onRemove={() =>
                        setPageSteps((steps) =>
                          steps.filter((candidate) => candidate.localId !== step().localId),
                        )
                      }
                    />
                  </>
                )}
              </Index>
            </div>

            <button
              type="button"
              class={funnelAddButton}
              onClick={() => setPageSteps((steps) => [...steps, emptyPageStep(options())])}
            >
              <StrokeIcon class={funnelStepIcon}>
                <path d="M12 5v14" />
                <path d="M5 12h14" />
              </StrokeIcon>
              Add step
            </button>

            <label class={funnelToggleRow}>
              <input
                class={funnelToggleInput}
                type="checkbox"
                checked={paymentStep() !== undefined}
                onChange={(event) =>
                  setPaymentStep(event.currentTarget.checked ? emptyPaymentStep() : undefined)
                }
              />
              <span class={funnelToggleTrack} aria-hidden="true">
                <span class={funnelToggleThumb} />
              </span>
              <span class={funnelToggleText}>
                <span class={funnelToggleTitle}>End the funnel with a purchase</span>
                <span class={funnelToggleHint}>
                  Adds a final payment step so you can measure revenue and conversion to purchase.
                </span>
              </span>
            </label>

            <Show when={paymentStep()}>
              {(payment) => (
                <>
                  <StepConnector rate={conversionInto(paymentStepIndex())} />
                  <PaymentStepCard
                    step={payment()}
                    previewVisitors={previewVisitorsAt(paymentStepIndex())}
                    previewLoading={previewLoading()}
                    onChange={setPaymentStep}
                  />
                </>
              )}
            </Show>
          </>
        )}
      </Show>
    </div>
  );
}

/** Line between two step cards, badged with the share of visitors who continued. */
function StepConnector(props: { rate: string | undefined }) {
  return (
    <div class={funnelStepConnector} aria-hidden="true">
      <span class={funnelStepConnectorLine} />
      <Show when={props.rate}>
        {(rate) => (
          <>
            <span class={funnelStepConnectorBadge}>
              <span class={funnelStepConnectorValue}>{rate()}</span>
              continue
            </span>
            <span class={funnelStepConnectorLine} />
          </>
        )}
      </Show>
    </div>
  );
}

function PaymentStepCard(props: {
  step: EditorStep;
  previewVisitors: number | undefined;
  previewLoading: boolean;
  onChange: (step: EditorStep) => void;
}) {
  return (
    <div class={funnelPaymentStepCard}>
      <div class={funnelStepHeader}>
        <div class={funnelStepHeaderInfo}>
          <span class={hoverHighlightRevenueIcon} aria-hidden="true" />
          <span class={funnelPaymentStepTitle}>Purchase</span>
          <FunnelStepVisitorCount visitors={props.previewVisitors} loading={props.previewLoading} />
        </div>
      </div>
      <div class={funnelPaymentStepFields}>
        <div class={funnelField}>
          <span class={funnelFieldLabel}>Payment types</span>
          <div class={funnelChipGroup}>
            <For each={PAYMENT_TYPES}>
              {(paymentType) => (
                <label class={funnelChip}>
                  <input
                    class={funnelChipInput}
                    type="checkbox"
                    checked={props.step.payment_types.includes(paymentType.value)}
                    onChange={(event) =>
                      props.onChange(
                        togglePaymentType(
                          props.step,
                          paymentType.value,
                          event.currentTarget.checked,
                        ),
                      )
                    }
                  />
                  <span class={funnelChipLabel}>{paymentType.label}</span>
                </label>
              )}
            </For>
          </div>
        </div>
        <label class={funnelField}>
          <span class={funnelFieldLabel}>Description</span>
          <input
            class={funnelInput}
            value={props.step.description}
            placeholder="e.g. Completed payment"
            onInput={(event) =>
              props.onChange({ ...props.step, description: event.currentTarget.value })
            }
          />
        </label>
      </div>
    </div>
  );
}
