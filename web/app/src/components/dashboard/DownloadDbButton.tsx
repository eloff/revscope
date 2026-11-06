/// <reference types="solid-js" />
import { Dialog } from "@ark-ui/solid/dialog";
import { error as errorMessage, primaryButton } from "@revscope/web-shared/authFormStyles";
import { useAuthState } from "@revscope/web-shared/authState";
import LoginDialog from "@revscope/web-shared/LoginDialog";
import {
  actionPrimaryButton,
  actionRow,
  backdrop,
  content,
  description,
  iconCloseButton,
  positioner,
  secondaryButton,
  title,
} from "@revscope/web-shared/loginDialogStyles";
import { createResource, createSignal, Show } from "solid-js";
import { Portal } from "solid-js/web";
import { DEMO_PROJECT_ID } from "../../lib/dashboardApi";
import {
  createDownloadToken,
  formatFileSize,
  projectDatabaseDownloadUrl,
  projectDatabaseFileName,
} from "../../lib/downloadDbApi";
import { confirmLink, downloadButton, downloadIcon, downloadSize } from "./DownloadDbButton.css";

function DownloadIcon() {
  return (
    <svg
      class={downloadIcon}
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      stroke-width="1.6"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <path d="M8 2v8" />
      <path d="M4.5 7L8 10.5 11.5 7" />
      <path d="M2.5 13h11" />
    </svg>
  );
}

type DownloadDbButtonProps = {
  projectId?: string;
};

export default function DownloadDbButton(props: DownloadDbButtonProps) {
  const { authUser } = useAuthState();
  const [open, setOpen] = createSignal(false);
  const [loginOpen, setLoginOpen] = createSignal(false);

  // The demo shows the control to everyone, but downloading it still takes a
  // session (a download can copy the whole database), so a visitor who isn't
  // signed in gets a sign-in prompt in place of the download.
  const projectId = () => {
    const id = props.projectId;
    return id && (id === DEMO_PROJECT_ID || authUser()) ? id : undefined;
  };
  const signInRequired = () => !authUser();

  // Minting only once the dialog opens keeps the token's twelve hours running
  // from the moment someone actually intends to download.
  const [token] = createResource(
    () => {
      const id = projectId();
      const session = authUser();
      return open() && id && session ? { session, projectId: id } : undefined;
    },
    ({ session, projectId }) => createDownloadToken(session, projectId),
  );
  // Reading an errored resource rethrows its error, so every read goes through here.
  const readyToken = () => (token.error ? undefined : token());
  const failure = () => {
    const reason = token.error as unknown;
    if (!reason) {
      return undefined;
    }
    return reason instanceof Error ? reason.message : "Unable to prepare the download.";
  };
  const sizeLine = () => {
    const ready = readyToken();
    if (ready) {
      return `Download size: ${formatFileSize(ready.size_bytes)}`;
    }
    return failure() ? undefined : "Checking download size…";
  };
  const downloadHref = () => {
    const id = projectId();
    const ready = readyToken();
    return id && ready ? projectDatabaseDownloadUrl(id, ready.token) : undefined;
  };

  // Swapped for a login dialog of our own rather than openLoginDialog: the demo
  // page has no header dialog to open. Nesting it inside this one doesn't work
  // either, since moving focus into it dismisses this dialog and unmounts it.
  const signIn = () => {
    setOpen(false);
    setLoginOpen(true);
  };

  return (
    <Show when={projectId()}>
      {(id) => (
        <>
          <Show when={signInRequired()}>
            <LoginDialog
              open={loginOpen()}
              onOpenChange={setLoginOpen}
              // Back to this page, so the download is a click away.
              redirectTo={window.location.pathname + window.location.search}
            />
          </Show>
          <Dialog.Root
            preventScroll={false}
            open={open()}
            onOpenChange={(details) => setOpen(details.open)}
          >
            <Dialog.Trigger
              class={downloadButton}
              aria-label="Download project database"
              title="Download project database"
            >
              <DownloadIcon />
            </Dialog.Trigger>
            <Portal>
              <Dialog.Backdrop class={backdrop} />
              <Dialog.Positioner class={positioner}>
                <Dialog.Content class={content}>
                  <Dialog.CloseTrigger class={iconCloseButton} aria-label="Close">
                    X
                  </Dialog.CloseTrigger>
                  <Dialog.Title class={title}>Download database</Dialog.Title>
                  <Dialog.Description class={description}>
                    Download your data as a sqlite database to query. Use Codex, Claude Code, or
                    Cursor to chat with your data on your local machine.
                  </Dialog.Description>
                  <Show
                    when={!signInRequired()}
                    fallback={
                      <>
                        <p class={errorMessage}>Sign in to download the demo database.</p>
                        <div class={actionRow}>
                          <button
                            type="button"
                            class={`${primaryButton} ${actionPrimaryButton}`}
                            onClick={signIn}
                          >
                            Sign in
                          </button>
                          <Dialog.CloseTrigger class={secondaryButton}>Cancel</Dialog.CloseTrigger>
                        </div>
                      </>
                    }
                  >
                    <Show when={failure()}>
                      {(reason) => <p class={errorMessage}>{reason()}</p>}
                    </Show>
                    <Show when={sizeLine()}>{(line) => <p class={downloadSize}>{line()}</p>}</Show>
                    <div class={actionRow}>
                      <Show
                        when={downloadHref()}
                        fallback={
                          <button
                            type="button"
                            class={`${primaryButton} ${actionPrimaryButton}`}
                            disabled
                          >
                            OK
                          </button>
                        }
                      >
                        {(href) => (
                          <a
                            class={`${primaryButton} ${actionPrimaryButton} ${confirmLink}`}
                            href={href()}
                            download={projectDatabaseFileName(id())}
                            // Closing synchronously would unmount the link mid-click and
                            // cancel the download the browser is about to start.
                            onClick={() => setTimeout(() => setOpen(false))}
                          >
                            OK
                          </a>
                        )}
                      </Show>
                      <Dialog.CloseTrigger class={secondaryButton}>Cancel</Dialog.CloseTrigger>
                    </div>
                  </Show>
                </Dialog.Content>
              </Dialog.Positioner>
            </Portal>
          </Dialog.Root>
        </>
      )}
    </Show>
  );
}
