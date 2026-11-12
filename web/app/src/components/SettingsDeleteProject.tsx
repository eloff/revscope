/// <reference types="solid-js" />
import { Dialog } from "@ark-ui/solid/dialog";
import { type AuthSession, reloadAuthSession } from "@revscope/web-shared/authState";
import {
  backdrop,
  content,
  description,
  title as dialogTitle,
  positioner,
} from "@revscope/web-shared/loginDialogStyles";
import { createSignal, Show } from "solid-js";
import { deleteProject } from "../lib/projectsApi";
import { destructiveButton, secondaryButton } from "./buttons.css";
import { dialogActions, fieldLabel, input, statusError, statusMessage } from "./Settings.css";
import {
  confirmField,
  dangerActions,
  dangerDescription,
  dangerItem,
  dangerItemTitle,
} from "./SettingsDangerZone.css";

type SettingsDeleteProjectProps = {
  session: AuthSession;
  projectId: string;
  projectName: string;
  /** An account keeps at least one project, which the server also enforces. */
  onlyProject: boolean;
  onDeleted: (projectId: string) => void;
};

export default function SettingsDeleteProject(props: SettingsDeleteProjectProps) {
  const [confirming, setConfirming] = createSignal(false);
  const [typedName, setTypedName] = createSignal("");
  const [deleting, setDeleting] = createSignal(false);
  const [errorMessage, setErrorMessage] = createSignal("");
  const nameMatches = () => typedName().trim() === props.projectName;

  const close = () => {
    if (deleting()) {
      return;
    }
    setConfirming(false);
    setTypedName("");
    setErrorMessage("");
  };

  const handleDelete = async (event: SubmitEvent) => {
    event.preventDefault();
    if (!nameMatches() || deleting()) {
      return;
    }
    const projectId = props.projectId;
    setDeleting(true);
    setErrorMessage("");
    try {
      await deleteProject(props.session, projectId, typedName().trim());
      await reloadAuthSession();
      setDeleting(false);
      close();
      props.onDeleted(projectId);
    } catch (caught) {
      setErrorMessage(caught instanceof Error ? caught.message : "Unable to delete the project.");
      setDeleting(false);
    }
  };

  return (
    <div class={dangerItem}>
      <h3 class={dangerItemTitle}>Delete {props.projectName}</h3>
      <p class={dangerDescription}>
        Permanently delete this project with all of its analytics, settings, integrations and API
        tokens. Its tracking script stops recording. This cannot be undone.
      </p>
      <div class={dangerActions}>
        <button
          type="button"
          class={destructiveButton}
          disabled={props.onlyProject}
          onClick={() => setConfirming(true)}
        >
          Delete project
        </button>
        <Show when={props.onlyProject}>
          <p class={statusMessage}>This is your only project, so it can't be deleted.</p>
        </Show>
      </div>

      <Dialog.Root
        open={confirming()}
        onOpenChange={(details) => {
          if (!details.open) {
            close();
          }
        }}
      >
        <Dialog.Backdrop class={backdrop} />
        <Dialog.Positioner class={positioner}>
          <Dialog.Content class={content}>
            <form onSubmit={(event) => void handleDelete(event)}>
              <Dialog.Title class={dialogTitle}>Delete {props.projectName}?</Dialog.Title>
              <Dialog.Description class={description}>
                Every pageview, payment, funnel, campaign and integration in this project is deleted
                for good. There is no undo.
              </Dialog.Description>
              <label class={confirmField}>
                <span class={fieldLabel}>Type {props.projectName} to confirm</span>
                <input
                  class={input}
                  type="text"
                  autocomplete="off"
                  value={typedName()}
                  onInput={(event) => setTypedName(event.currentTarget.value)}
                  disabled={deleting()}
                />
              </label>
              <Show when={errorMessage()}>
                <p class={`${statusMessage} ${statusError}`}>{errorMessage()}</p>
              </Show>
              <div class={dialogActions}>
                <button
                  type="submit"
                  class={destructiveButton}
                  disabled={deleting() || !nameMatches()}
                >
                  {deleting() ? "Deleting…" : "Delete project"}
                </button>
                <button type="button" class={secondaryButton} disabled={deleting()} onClick={close}>
                  Cancel
                </button>
              </div>
            </form>
          </Dialog.Content>
        </Dialog.Positioner>
      </Dialog.Root>
    </div>
  );
}
