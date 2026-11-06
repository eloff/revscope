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
import { revokeMcpTokens } from "../lib/settingsApi";
import {
  dialogActions,
  dialogSecondary,
  statusError,
  statusMessage,
  statusSuccess,
} from "./Settings.css";
import {
  dangerActions,
  dangerButton,
  dangerDescription,
  dangerItem,
  dangerItemTitle,
  dangerTitle,
  dangerZone,
} from "./SettingsDangerZone.css";
import SettingsDeleteProject from "./SettingsDeleteProject";

type SettingsDangerZoneProps = {
  session: AuthSession;
  /** The project the settings form is showing; absent while none is loaded. */
  project?: { id: string; name: string };
  projectCount: number;
  onProjectDeleted: (projectId: string) => void;
};

export default function SettingsDangerZone(props: SettingsDangerZoneProps) {
  const [confirming, setConfirming] = createSignal(false);
  const [revoking, setRevoking] = createSignal(false);
  const [errorMessage, setErrorMessage] = createSignal("");
  const [successMessage, setSuccessMessage] = createSignal("");

  const handleRevoke = async () => {
    setRevoking(true);
    setErrorMessage("");
    setSuccessMessage("");
    try {
      const result = await revokeMcpTokens(props.session);
      setConfirming(false);
      setSuccessMessage(
        result.revoked === 0
          ? "No MCP tokens to revoke."
          : result.revoked === 1
            ? "Revoked 1 MCP token."
            : `Revoked ${result.revoked} MCP tokens.`,
      );
    } catch (caught) {
      setErrorMessage(caught instanceof Error ? caught.message : "Unable to revoke MCP tokens.");
    } finally {
      setRevoking(false);
    }
  };

  return (
    <section class={dangerZone}>
      <h2 class={dangerTitle}>Danger zone</h2>
      <Show when={props.project}>
        {(project) => (
          <SettingsDeleteProject
            session={props.session}
            projectId={project().id}
            projectName={project().name}
            onlyProject={props.projectCount <= 1}
            onDeleted={props.onProjectDeleted}
          />
        )}
      </Show>
      <div class={dangerItem}>
        <h3 class={dangerItemTitle}>Revoke MCP tokens</h3>
        <p class={dangerDescription}>
          Revoke every MCP token on this account. Approved clients can read every project for a
          year; this deletes those tokens. Agents may keep working for up to five minutes.
        </p>
        <div class={dangerActions}>
          <button type="button" class={dangerButton} onClick={() => setConfirming(true)}>
            Revoke all MCP tokens
          </button>
          <Show when={errorMessage()}>
            <p class={`${statusMessage} ${statusError}`}>{errorMessage()}</p>
          </Show>
          <Show when={successMessage()}>
            <p class={`${statusMessage} ${statusSuccess}`}>{successMessage()}</p>
          </Show>
        </div>
      </div>

      <Dialog.Root
        open={confirming()}
        onOpenChange={(details) => {
          if (!details.open && !revoking()) {
            setConfirming(false);
          }
        }}
      >
        <Dialog.Backdrop class={backdrop} />
        <Dialog.Positioner class={positioner}>
          <Dialog.Content class={content}>
            <Dialog.Title class={dialogTitle}>Revoke MCP tokens?</Dialog.Title>
            <Dialog.Description class={description}>
              This disconnects every MCP client from this account. Agents may keep working for up to
              five minutes. You will need to approve each client again.
            </Dialog.Description>
            <div class={dialogActions}>
              <button
                type="button"
                class={dangerButton}
                disabled={revoking()}
                onClick={() => void handleRevoke()}
              >
                {revoking() ? "Revoking…" : "Revoke all MCP tokens"}
              </button>
              <button
                type="button"
                class={dialogSecondary}
                disabled={revoking()}
                onClick={() => setConfirming(false)}
              >
                Cancel
              </button>
            </div>
          </Dialog.Content>
        </Dialog.Positioner>
      </Dialog.Root>
    </section>
  );
}
