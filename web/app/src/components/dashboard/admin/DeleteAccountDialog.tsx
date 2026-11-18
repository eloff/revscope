/// <reference types="solid-js" />
import { Dialog } from "@ark-ui/solid/dialog";
import type { AuthSession } from "@revscope/web-shared/authState";
import {
  backdrop,
  content,
  description,
  title as dialogTitle,
  positioner,
} from "@revscope/web-shared/loginDialogStyles";
import { createSignal, Show } from "solid-js";
import { deleteSiteAdminAccount, type SiteAdminAccount } from "../../../lib/siteAdminApi";
import { destructiveButton, secondaryButton } from "../../buttons.css";
import { dialogActions, fieldLabel, input, statusError, statusMessage } from "../../Settings.css";
import { confirmField } from "../../SettingsDangerZone.css";

type DeleteAccountDialogProps = {
  session: AuthSession;
  account: SiteAdminAccount;
  open: boolean;
  onClose: () => void;
  onDeleted: () => void;
};

function count(n: number, noun: string): string {
  return `${n} ${noun}${n === 1 ? "" : "s"}`;
}

/** Deleting an account, once its name is typed out, since there's no undo. */
export default function DeleteAccountDialog(props: DeleteAccountDialogProps) {
  const [typedName, setTypedName] = createSignal("");
  const [deleting, setDeleting] = createSignal(false);
  const [errorMessage, setErrorMessage] = createSignal("");
  const nameMatches = () => typedName().trim() === props.account.name;

  const close = () => {
    if (deleting()) {
      return;
    }
    setTypedName("");
    setErrorMessage("");
    props.onClose();
  };

  const handleDelete = async (event: SubmitEvent) => {
    event.preventDefault();
    if (!nameMatches() || deleting()) {
      return;
    }
    setDeleting(true);
    setErrorMessage("");
    try {
      await deleteSiteAdminAccount(props.session, props.account.account_id, typedName().trim());
      setDeleting(false);
      close();
      props.onDeleted();
    } catch (caught) {
      setErrorMessage(caught instanceof Error ? caught.message : "Unable to delete the account.");
      setDeleting(false);
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
          <form onSubmit={(event) => void handleDelete(event)}>
            <Dialog.Title class={dialogTitle}>Delete {props.account.name}?</Dialog.Title>
            <Dialog.Description class={description}>
              Its {count(props.account.users.length, "user")} and{" "}
              {count(props.account.projects.length, "project")}, with every project's analytics
              database, are deleted for good. Their project names stay reserved, so sites still
              running the tracker can't send data to anyone else. There is no undo.
            </Dialog.Description>
            <label class={confirmField}>
              <span class={fieldLabel}>Type {props.account.name} to confirm</span>
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
                {deleting() ? "Deleting…" : "Delete account"}
              </button>
              <button type="button" class={secondaryButton} disabled={deleting()} onClick={close}>
                Cancel
              </button>
            </div>
          </form>
        </Dialog.Content>
      </Dialog.Positioner>
    </Dialog.Root>
  );
}
