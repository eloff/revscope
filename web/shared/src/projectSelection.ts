import { makePersisted } from "@solid-primitives/storage";
import { createSignal } from "solid-js";
import { isServer } from "solid-js/web";
import type { AuthProject } from "./authState";

/** Public demo project. The dashboard re-exports this for its data clients. */
export const DEMO_PROJECT_ID = "1";

export type ProjectOption = { label: string; value: string; timezone: string };

export function isDemoProjectId(projectId: string): boolean {
  return projectId === DEMO_PROJECT_ID;
}

/** JSON object key order isn't dependable, so sort by name on the client. */
export function projectOptions(projects: Record<string, AuthProject>): ProjectOption[] {
  return Object.entries(projects)
    .map(([name, project]) => ({
      label: name,
      value: project.id,
      timezone: project.settings.timezone || "UTC",
    }))
    .sort((left, right) => left.label.localeCompare(right.label));
}

/**
 * The selected project's IANA zone, or UTC while the account is still loading.
 *
 * Every calendar boundary the dashboard computes goes through this, so a project
 * whose options have not arrived yet must not silently pick the browser's zone.
 */
export function projectTimeZone(
  options: ProjectOption[] | undefined,
  projectId: string | undefined,
): string {
  return options?.find((option) => option.value === projectId)?.timezone || "UTC";
}

/** Falls back to the first project when the preferred one isn't on this account. */
export function resolveProjectId(options: ProjectOption[], preferred: string): string {
  return options.some((option) => option.value === preferred)
    ? preferred
    : (options[0]?.value ?? "");
}

/** Shared by the dashboard and settings so both open on the same project. */
export const [selectedProjectId, setSelectedProjectId] = makePersisted(createSignal(""), {
  name: "revscope.selectedProjectId",
  storage: isServer ? undefined : localStorage,
});

/**
 * Plans sold with more than one website. Mirrors the plans in
 * `plan_allows_multiple_projects` in revscope/src/projects.rs, which also needs a
 * subscription in good standing and is what actually refuses the extra project.
 */
export function planAllowsMultipleProjects(plan: string): boolean {
  return plan === "growth" || plan === "founder" || plan === "enterprise";
}
