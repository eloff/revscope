import type {
  FunnelEventOption,
  FunnelOptionsResponse,
  FunnelPaymentType,
  FunnelStep,
  FunnelStepType,
  SaveFunnelStep,
} from "../../lib/funnelsApi";

export type EditorStep = {
  localId: string;
  step_type: FunnelStepType;
  event_name_id?: number;
  path_id?: number;
  is_landing: boolean;
  payment_types: FunnelPaymentType[];
  description: string;
};

/** A funnel's steps as the editor holds them: the payment step is kept apart so it stays last. */
export type EditorSteps = {
  pageSteps: EditorStep[];
  paymentStep?: EditorStep;
};

export const PAYMENT_TYPES: { value: FunnelPaymentType; label: string }[] = [
  { value: "single", label: "Single" },
  { value: "subscription", label: "Subscription" },
  { value: "refund", label: "Refund" },
];

let nextLocalStepId = 1;

function newLocalStepId(): string {
  nextLocalStepId += 1;
  return `step-${Date.now()}-${nextLocalStepId}`;
}

export function selectOptionValue(id: number | undefined): string {
  return id == null ? "" : String(id);
}

function eventOccursOnPath(eventName: FunnelEventOption, pathId: number | undefined): boolean {
  return pathId !== undefined && eventName.path_ids.includes(pathId);
}

export function eventOptionsForPath(
  options: FunnelOptionsResponse | undefined,
  pathId: number | undefined,
): FunnelEventOption[] {
  return (options?.events ?? []).filter((eventName) => eventOccursOnPath(eventName, pathId));
}

function eventIsAvailableForPath(
  options: FunnelOptionsResponse | undefined,
  eventNameId: number | undefined,
  pathId: number | undefined,
): boolean {
  if (eventNameId === undefined || !options) {
    return true;
  }
  return options.events.some(
    (eventName) => eventName.id === eventNameId && eventOccursOnPath(eventName, pathId),
  );
}

function editorStepFromFunnelStep(step: FunnelStep): EditorStep {
  return {
    localId: newLocalStepId(),
    step_type: step.step_type,
    event_name_id: step.event_name_id,
    path_id: step.path_id,
    is_landing: step.is_landing,
    payment_types: [...step.payment_types],
    description: step.description ?? "",
  };
}

export function emptyPageStep(options?: FunnelOptionsResponse): EditorStep {
  return {
    localId: newLocalStepId(),
    step_type: "pageview",
    path_id: options?.paths[0]?.id,
    is_landing: false,
    payment_types: [],
    description: "",
  };
}

export function emptyPaymentStep(): EditorStep {
  return {
    localId: newLocalStepId(),
    step_type: "payment",
    is_landing: false,
    payment_types: ["single", "subscription"],
    description: "Purchase",
  };
}

/** Splits a saved funnel's steps into the editor's reorderable page steps and pinned payment step. */
export function editorStepsFromFunnel(
  steps: FunnelStep[],
  options?: FunnelOptionsResponse,
): EditorSteps {
  const editorSteps = steps.map(editorStepFromFunnelStep);
  const last = editorSteps[editorSteps.length - 1];
  const paymentStep = last?.step_type === "payment" ? last : undefined;
  const pageSteps = paymentStep ? editorSteps.slice(0, -1) : editorSteps;
  return {
    pageSteps: pageSteps.length > 0 ? pageSteps : [emptyPageStep(options)],
    paymentStep,
  };
}

/** Maps editor steps onto the API step shape shared by save and preview. */
export function editorStepsToSaveSteps(steps: EditorStep[]): SaveFunnelStep[] {
  return steps.map((step, index) => ({
    step_type: step.step_type,
    event_name_id: step.step_type === "pageview" ? step.event_name_id : undefined,
    path_id: step.step_type === "pageview" ? step.path_id : undefined,
    is_landing: step.step_type === "pageview" && index === 0 && step.is_landing,
    payment_types: step.step_type === "payment" ? step.payment_types : [],
    description: step.description.trim() || undefined,
  }));
}

/** A pageview step only needs a page (the event is optional); a payment step needs a type. */
export function editorStepIsComplete(step: EditorStep): boolean {
  if (step.step_type === "payment") {
    return step.payment_types.length > 0;
  }
  return Boolean(step.path_id);
}

/** True when the funnel can be evaluated by the preview/save endpoints without a 400. */
export function editorStepsAreComplete(steps: EditorStep[]): boolean {
  return steps.length > 0 && steps.every(editorStepIsComplete);
}

/** Applies an edit to one page step, dropping state the edit made invalid. */
export function updatePageStep(
  steps: EditorStep[],
  localId: string,
  update: Partial<EditorStep>,
  options: FunnelOptionsResponse | undefined,
): EditorStep[] {
  return steps.map((step, index) => {
    if (step.localId !== localId) {
      return step;
    }
    const next = { ...step, ...update };
    if (index !== 0) {
      next.is_landing = false;
    }
    if (
      update.path_id !== undefined &&
      !eventIsAvailableForPath(options, next.event_name_id, next.path_id)
    ) {
      next.event_name_id = undefined;
    }
    return next;
  });
}

/** Swaps a step with its neighbour; only the first step may stay landing-only. */
export function movePageStep(
  steps: EditorStep[],
  localId: string,
  direction: -1 | 1,
): EditorStep[] {
  const index = steps.findIndex((step) => step.localId === localId);
  const nextIndex = index + direction;
  if (index === -1 || nextIndex < 0 || nextIndex >= steps.length) {
    return steps;
  }
  const next = [...steps];
  [next[index], next[nextIndex]] = [next[nextIndex], next[index]];
  return next.map((step, stepIndex) => ({
    ...step,
    is_landing: stepIndex === 0 ? step.is_landing : false,
  }));
}

/**
 * Steps added before the options arrived have no page yet, while their select
 * already shows the first one; give them that page so the form saves what it
 * displays.
 */
export function fillMissingPaths(
  steps: EditorStep[],
  options: FunnelOptionsResponse,
): EditorStep[] {
  const firstPathId = options.paths[0]?.id;
  if (firstPathId === undefined) {
    return steps;
  }
  return steps.map((step) =>
    step.step_type === "pageview" && step.path_id === undefined
      ? { ...step, path_id: firstPathId }
      : step,
  );
}

export function togglePaymentType(
  step: EditorStep,
  paymentType: FunnelPaymentType,
  checked: boolean,
): EditorStep {
  const paymentTypes = checked
    ? [...step.payment_types, paymentType]
    : step.payment_types.filter((value) => value !== paymentType);
  return { ...step, payment_types: [...new Set(paymentTypes)] };
}
