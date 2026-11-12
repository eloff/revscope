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
import type { ProjectOption } from "@revscope/web-shared/projectSelection";
import { createSignal, Show } from "solid-js";
import { type AccountUser, removeUser, updateUserAccess } from "../../lib/usersApi";
import { dangerButton, destructiveButton, primaryButton, secondaryButton } from "../buttons.css";
import { dialogActions, statusError, statusMessage } from "../Settings.css";
import {
  adminTag,
  editActions,
  editPanel,
  pendingTag,
  userAccessSummary,
  userActions,
  userEmail,
  userIdentity,
  userName,
  userRow,
  userSummary,
  youTag,
} from "./ManageUsers.css";
import UserAccessFields, {
  type AccessDraft,
  accessFromDraft,
  draftFromAccess,
} from "./UserAccessFields";

type UserRowProps = {
  session: AuthSession;
  user: AccountUser;
  projects: ProjectOption[];
  /** Admins can't change or remove themselves, so their own row has no actions. */
  isSelf: boolean;
  onChanged: () => void;
};

/** Which projects a member reads, by name. Ids no longer on the account are dropped. */
export function projectAccessLabel(user: AccountUser, projects: ProjectOption[]): string {
  if (user.is_admin || user.project_ids === null) {
    return "All projects";
  }
  const names = user.project_ids
    .map((id) => projects.find((project) => project.value === String(id))?.label)
    .filter((name): name is string => Boolean(name));
  return names.length === 0 ? "No projects" : names.join(", ");
}

export default function UserRow(props: UserRowProps) {
  const [editing, setEditing] = createSignal(false);
  const [draft, setDraft] = createSignal<AccessDraft>(draftFromAccess(props.user));
  const [confirmingRemove, setConfirmingRemove] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [errorMessage, setErrorMessage] = createSignal("");

  const startEditing = () => {
    setDraft(draftFromAccess(props.user));
    setErrorMessage("");
    setEditing(true);
  };

  const save = async () => {
    const access = accessFromDraft(draft());
    if (typeof access === "string") {
      setErrorMessage(access);
      return;
    }
    setBusy(true);
    setErrorMessage("");
    try {
      await updateUserAccess(props.session, props.user.user_id, access);
      setEditing(false);
      props.onChanged();
    } catch (caught) {
      setErrorMessage(caught instanceof Error ? caught.message : "Unable to save access.");
    } finally {
      setBusy(false);
    }
  };

  const remove = async () => {
    setBusy(true);
    setErrorMessage("");
    try {
      await removeUser(props.session, props.user.user_id);
      setConfirmingRemove(false);
      props.onChanged();
    } catch (caught) {
      setConfirmingRemove(false);
      setErrorMessage(caught instanceof Error ? caught.message : "Unable to remove the user.");
    } finally {
      setBusy(false);
    }
  };

  return (
    <li class={userRow} aria-label={props.user.name}>
      <div class={userSummary}>
        <div class={userIdentity}>
          <span class={userName}>{props.user.name}</span>
          <span class={userEmail}>{props.user.email}</span>
        </div>
        <Show when={props.isSelf}>
          <span class={youTag}>You</span>
        </Show>
        <Show when={props.user.is_admin}>
          <span class={adminTag}>Admin</span>
        </Show>
        <Show when={props.user.pending}>
          <span class={pendingTag}>Invited</span>
        </Show>
        <span class={userAccessSummary}>{projectAccessLabel(props.user, props.projects)}</span>
        <Show when={!props.isSelf && !editing()}>
          <div class={userActions}>
            <button type="button" class={secondaryButton} onClick={startEditing}>
              Edit access
            </button>
            <button
              type="button"
              class={dangerButton}
              onClick={() => {
                setErrorMessage("");
                setConfirmingRemove(true);
              }}
            >
              Remove
            </button>
          </div>
        </Show>
      </div>

      <Show when={editing()}>
        <div class={editPanel}>
          <UserAccessFields
            idPrefix={`edit-${props.user.user_id}`}
            projects={props.projects}
            value={draft()}
            onChange={setDraft}
          />
          <div class={editActions}>
            <button
              type="button"
              class={primaryButton}
              disabled={busy()}
              onClick={() => void save()}
            >
              {busy() ? "Saving…" : "Save access"}
            </button>
            <button
              type="button"
              class={secondaryButton}
              disabled={busy()}
              onClick={() => setEditing(false)}
            >
              Cancel
            </button>
          </div>
        </div>
      </Show>
      <Show when={errorMessage()}>
        <p class={`${statusMessage} ${statusError}`}>{errorMessage()}</p>
      </Show>

      <Dialog.Root
        open={confirmingRemove()}
        onOpenChange={(details) => {
          if (!details.open && !busy()) {
            setConfirmingRemove(false);
          }
        }}
      >
        <Dialog.Backdrop class={backdrop} />
        <Dialog.Positioner class={positioner}>
          <Dialog.Content class={content}>
            <Dialog.Title class={dialogTitle}>Remove {props.user.name}?</Dialog.Title>
            <Dialog.Description class={description}>
              {props.user.pending
                ? "Their invite link stops working."
                : "They're logged out and can no longer see this account's analytics."}
            </Dialog.Description>
            <div class={dialogActions}>
              <button
                type="button"
                class={destructiveButton}
                disabled={busy()}
                onClick={() => void remove()}
              >
                {busy() ? "Removing…" : "Remove"}
              </button>
              <button
                type="button"
                class={secondaryButton}
                disabled={busy()}
                onClick={() => setConfirmingRemove(false)}
              >
                Cancel
              </button>
            </div>
          </Dialog.Content>
        </Dialog.Positioner>
      </Dialog.Root>
    </li>
  );
}
