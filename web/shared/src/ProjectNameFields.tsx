/// <reference types="solid-js" />

import { createSignal } from "solid-js";
import { input, label } from "./AuthForm.css";
import { MIN_ACCOUNT_NAME_LENGTH, WEBSITE_PATTERN } from "./authValidation";
import { hint } from "./ProjectNameFields.css";
import {
  MIN_PROJECT_NAME_LENGTH,
  projectNameFromWebsite,
  sanitizeProjectName,
} from "./projectName";

export type ProjectNameState = ReturnType<typeof createProjectNameState>;

/**
 * A signup's website and its first project's name. The name follows the
 * website until the user types a different one, then stays as they typed it.
 * Typing the derived name again, or a key the name drops, keeps it following.
 */
export function createProjectNameState() {
  const [website, setWebsite] = createSignal("");
  const [editedName, setEditedName] = createSignal<string | null>(null);
  const derivedName = () => projectNameFromWebsite(website());
  return {
    website,
    setWebsite,
    projectName: () => editedName() ?? derivedName(),
    setProjectName: (name: string) => setEditedName(name === derivedName() ? null : name),
  };
}

/** Cleans up what was typed into the name field and stores it. */
function applyProjectName(field: HTMLInputElement, state: ProjectNameState) {
  const name = sanitizeProjectName(field.value);
  if (name !== field.value) {
    // Set directly: a dropped character leaves the signal unchanged, so Solid
    // wouldn't rewrite the input. Rewriting it moves the caret to the end, so
    // put it back after what was kept before it.
    const caret = sanitizeProjectName(field.value.slice(0, field.selectionStart ?? 0)).length;
    field.value = name;
    field.setSelectionRange(caret, caret);
  }
  state.setProjectName(name);
}

type ProjectNameFieldsProps = {
  state: ProjectNameState;
  /** Self-hosted signup only uses the website to suggest the name. */
  websiteRequired: boolean;
};

export default function ProjectNameFields(props: ProjectNameFieldsProps) {
  return (
    <>
      <label class={label} for="signup-website">
        Website
      </label>
      <input
        id="signup-website"
        name="account_name"
        type="text"
        autocomplete="url"
        required={props.websiteRequired}
        minLength={props.websiteRequired ? MIN_ACCOUNT_NAME_LENGTH : undefined}
        pattern={props.websiteRequired ? WEBSITE_PATTERN.source : undefined}
        placeholder="mywebsite.com"
        class={input}
        value={props.state.website()}
        onInput={(event) => props.state.setWebsite(event.currentTarget.value)}
      />
      <label class={label} for="signup-project-name">
        Project name
      </label>
      <input
        id="signup-project-name"
        name="project_name"
        type="text"
        autocomplete="off"
        autocapitalize="off"
        spellcheck={false}
        required
        minLength={MIN_PROJECT_NAME_LENGTH}
        aria-describedby="signup-project-name-hint"
        class={input}
        value={props.state.projectName()}
        onInput={(event) => {
          // Rewriting the field mid-composition (IMEs, and Android keyboards
          // that compose whole words) can duplicate text, so wait for it to end.
          if (!event.isComposing) {
            applyProjectName(event.currentTarget, props.state);
          }
        }}
        onCompositionEnd={(event) => applyProjectName(event.currentTarget, props.state)}
      />
      <p id="signup-project-name-hint" class={hint}>
        Lowercase letters, numbers and underscores. It's part of your tracking script's ingest URL
        and shows in your dashboard, so it can't be changed later.
      </p>
    </>
  );
}
