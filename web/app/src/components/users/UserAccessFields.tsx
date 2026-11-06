/// <reference types="solid-js" />
import type { ProjectOption } from "@revscope/web-shared/projectSelection";
import { For, Show } from "solid-js";
import type { UserAccess } from "../../lib/usersApi";
import {
  accessFieldset,
  accessGroups,
  accessLegend,
  choice,
  choiceHint,
  projectChoices,
} from "./ManageUsers.css";

/**
 * Access while it's being edited. Unlike {@link UserAccess} it keeps the picked
 * projects while "all projects" is on, so toggling back doesn't lose them.
 */
export type AccessDraft = {
  isAdmin: boolean;
  allProjects: boolean;
  projectIds: number[];
};

export const NEW_USER_ACCESS: AccessDraft = { isAdmin: false, allProjects: true, projectIds: [] };

export function draftFromAccess(access: UserAccess): AccessDraft {
  return {
    isAdmin: access.is_admin,
    allProjects: access.is_admin || access.project_ids === null,
    projectIds: access.project_ids ?? [],
  };
}

/** The access to send, or an error to show when a member was given no projects. */
export function accessFromDraft(draft: AccessDraft): UserAccess | string {
  if (draft.isAdmin) {
    return { is_admin: true, project_ids: null };
  }
  if (draft.allProjects) {
    return { is_admin: false, project_ids: null };
  }
  if (draft.projectIds.length === 0) {
    return "Choose at least one project, or give access to all of them.";
  }
  return { is_admin: false, project_ids: [...draft.projectIds].sort((a, b) => a - b) };
}

type UserAccessFieldsProps = {
  /** Keeps radio groups and labels distinct when several editors are on the page. */
  idPrefix: string;
  projects: ProjectOption[];
  value: AccessDraft;
  onChange: (next: AccessDraft) => void;
};

/** Role, and for members, which projects they can read. */
export default function UserAccessFields(props: UserAccessFieldsProps) {
  const update = (patch: Partial<AccessDraft>) => props.onChange({ ...props.value, ...patch });
  const toggleProject = (projectId: number, checked: boolean) => {
    const others = props.value.projectIds.filter((id) => id !== projectId);
    update({ projectIds: checked ? [...others, projectId] : others });
  };

  return (
    <div class={accessGroups}>
      <fieldset class={accessFieldset}>
        <legend class={accessLegend}>Role</legend>
        <label class={choice}>
          <input
            type="radio"
            name={`${props.idPrefix}-role`}
            checked={!props.value.isAdmin}
            onChange={() => update({ isAdmin: false })}
          />
          <span>
            Member <span class={choiceHint}>reads analytics</span>
          </span>
        </label>
        <label class={choice}>
          <input
            type="radio"
            name={`${props.idPrefix}-role`}
            checked={props.value.isAdmin}
            onChange={() => update({ isAdmin: true })}
          />
          <span>
            Admin <span class={choiceHint}>also manages billing, users and every project</span>
          </span>
        </label>
      </fieldset>

      <Show when={!props.value.isAdmin}>
        <fieldset class={accessFieldset}>
          <legend class={accessLegend}>Projects</legend>
          <label class={choice}>
            <input
              type="radio"
              name={`${props.idPrefix}-projects`}
              checked={props.value.allProjects}
              onChange={() => update({ allProjects: true })}
            />
            <span>
              All projects <span class={choiceHint}>including ones added later</span>
            </span>
          </label>
          <label class={choice}>
            <input
              type="radio"
              name={`${props.idPrefix}-projects`}
              checked={!props.value.allProjects}
              onChange={() => update({ allProjects: false })}
            />
            <span>Only these projects</span>
          </label>
          <Show when={!props.value.allProjects}>
            <div class={projectChoices}>
              <For each={props.projects}>
                {(project) => (
                  <label class={choice}>
                    <input
                      type="checkbox"
                      checked={props.value.projectIds.includes(Number(project.value))}
                      onChange={(event) =>
                        toggleProject(Number(project.value), event.currentTarget.checked)
                      }
                    />
                    <span>{project.label}</span>
                  </label>
                )}
              </For>
            </div>
          </Show>
        </fieldset>
      </Show>
    </div>
  );
}
