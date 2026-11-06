/// <reference types="solid-js" />

import { Dialog } from "@ark-ui/solid/dialog";
import { type AuthSession, reloadAuthSession } from "@revscope/web-shared/authState";
import { browserTimezone } from "@revscope/web-shared/browserTimezone";
import { docsUrl } from "@revscope/web-shared/links";
import {
  backdrop,
  content,
  description,
  title as dialogTitle,
  positioner,
} from "@revscope/web-shared/loginDialogStyles";
import { projectNameFromWebsite, validateProjectName } from "@revscope/web-shared/projectName";
import { createSignal, Show } from "solid-js";
import { type CreatedProject, createProject } from "../../lib/projectsApi";
import {
  dialogActions,
  dialogPrimary,
  dialogSecondary,
  fieldLabel,
  input,
  statusError,
  statusMessage,
} from "../Settings.css";
import { dialogField, dialogPreview } from "./ProjectsOverview.css";

type AddProjectDialogProps = {
  session: AuthSession;
  open: boolean;
  onClose: () => void;
  onOpenProject: (projectId: string) => void;
};

export default function AddProjectDialog(props: AddProjectDialogProps) {
  const [website, setWebsite] = createSignal("");
  const [saving, setSaving] = createSignal(false);
  const [errorMessage, setErrorMessage] = createSignal("");
  const [created, setCreated] = createSignal<CreatedProject | null>(null);
  const name = () => projectNameFromWebsite(website());
  const nameError = () => validateProjectName(name());

  const close = () => {
    if (saving()) {
      return;
    }
    setWebsite("");
    setErrorMessage("");
    setCreated(null);
    props.onClose();
  };

  const submit = async (event: SubmitEvent) => {
    event.preventDefault();
    if (nameError() || saving()) {
      return;
    }
    setSaving(true);
    setErrorMessage("");
    try {
      const project = await createProject(props.session, website(), browserTimezone());
      // The project list everywhere comes from `/v2/session/me`.
      await reloadAuthSession();
      setCreated(project);
    } catch (caught) {
      setErrorMessage(caught instanceof Error ? caught.message : "Unable to add the project.");
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog.Root
      open={props.open}
      onOpenChange={(details) => {
        if (!details.open) {
          close();
        }
      }}
    >
      <Dialog.Backdrop class={backdrop} />
      <Dialog.Positioner class={positioner}>
        <Dialog.Content class={content}>
          <Show
            when={created()}
            fallback={
              <form onSubmit={(event) => void submit(event)}>
                <Dialog.Title class={dialogTitle}>Add a project</Dialog.Title>
                <Dialog.Description class={description}>
                  Each website gets its own project, tracking script and settings.
                </Dialog.Description>
                <label class={dialogField}>
                  <span class={fieldLabel}>Website</span>
                  <input
                    class={input}
                    type="text"
                    placeholder="shop.example.com"
                    value={website()}
                    onInput={(event) => setWebsite(event.currentTarget.value)}
                    disabled={saving()}
                    autofocus
                  />
                </label>
                <Show when={website().trim()}>
                  <p class={dialogPreview}>{nameError() ?? `Project name: ${name()}`}</p>
                </Show>
                <Show when={errorMessage()}>
                  <p class={`${statusMessage} ${statusError}`}>{errorMessage()}</p>
                </Show>
                <div class={dialogActions}>
                  <button
                    type="submit"
                    class={dialogPrimary}
                    disabled={saving() || nameError() !== null}
                  >
                    {saving() ? "Adding…" : "Add project"}
                  </button>
                  <button type="button" class={dialogSecondary} disabled={saving()} onClick={close}>
                    Cancel
                  </button>
                </div>
              </form>
            }
          >
            {(project) => (
              <>
                <Dialog.Title class={dialogTitle}>Added {project().name}</Dialog.Title>
                <Dialog.Description class={description}>
                  To start collecting data, add the tracking script to your site with the project
                  name <code>{project().name}</code>. See{" "}
                  <a href={docsUrl("/docs/getting-started")}>Getting started</a>.
                </Dialog.Description>
                <div class={dialogActions}>
                  <button
                    type="button"
                    class={dialogPrimary}
                    onClick={() => {
                      const projectId = project().id;
                      close();
                      props.onOpenProject(projectId);
                    }}
                  >
                    Open project
                  </button>
                  <button type="button" class={dialogSecondary} onClick={close}>
                    Close
                  </button>
                </div>
              </>
            )}
          </Show>
        </Dialog.Content>
      </Dialog.Positioner>
    </Dialog.Root>
  );
}
