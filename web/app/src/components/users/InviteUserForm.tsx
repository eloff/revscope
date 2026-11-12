/// <reference types="solid-js" />
import type { AuthSession } from "@revscope/web-shared/authState";
import { validateEmail, validateUserName } from "@revscope/web-shared/authValidation";
import type { ProjectOption } from "@revscope/web-shared/projectSelection";
import { createSignal, Show } from "solid-js";
import { inviteUser } from "../../lib/usersApi";
import { primaryButton } from "../buttons.css";
import {
  field,
  fieldGrid,
  fieldLabel,
  input,
  section,
  sectionDescription,
  sectionTitle,
  statusError,
  statusMessage,
  statusSuccess,
} from "../Settings.css";
import { inviteLink, inviteLinkRow } from "./ManageUsers.css";
import UserAccessFields, {
  type AccessDraft,
  accessFromDraft,
  NEW_USER_ACCESS,
} from "./UserAccessFields";

type InviteUserFormProps = {
  session: AuthSession;
  projects: ProjectOption[];
  onInvited: () => void;
};

export default function InviteUserForm(props: InviteUserFormProps) {
  const [name, setName] = createSignal("");
  const [email, setEmail] = createSignal("");
  const [access, setAccess] = createSignal<AccessDraft>(NEW_USER_ACCESS);
  const [sending, setSending] = createSignal(false);
  const [errorMessage, setErrorMessage] = createSignal("");
  const [sent, setSent] = createSignal<{ email: string; url: string; emailed: boolean } | null>(
    null,
  );
  const [copied, setCopied] = createSignal(false);

  const onSubmit = async (event: SubmitEvent) => {
    event.preventDefault();
    setErrorMessage("");
    setSent(null);
    setCopied(false);
    const trimmedName = name().trim();
    const trimmedEmail = email().trim();
    const validationError = validateUserName(trimmedName) ?? validateEmail(trimmedEmail);
    if (validationError) {
      setErrorMessage(validationError);
      return;
    }
    const chosen = accessFromDraft(access());
    if (typeof chosen === "string") {
      setErrorMessage(chosen);
      return;
    }
    setSending(true);
    try {
      const result = await inviteUser(props.session, {
        name: trimmedName,
        email: trimmedEmail,
        ...chosen,
      });
      setSent({ email: trimmedEmail, url: result.invite_url, emailed: result.emailed });
      setName("");
      setEmail("");
      setAccess(NEW_USER_ACCESS);
      props.onInvited();
    } catch (caught) {
      setErrorMessage(caught instanceof Error ? caught.message : "Unable to send the invite.");
    } finally {
      setSending(false);
    }
  };

  const copyLink = async (url: string) => {
    try {
      await navigator.clipboard.writeText(url);
      setCopied(true);
    } catch {
      setErrorMessage("Unable to copy. Select the link and copy it instead.");
    }
  };

  return (
    <section class={section}>
      <h2 class={sectionTitle}>Invite someone</h2>
      <p class={sectionDescription}>
        They get a link to choose a password. Inviting someone who hasn't accepted yet sends a new
        link with the access chosen here.
      </p>
      <form class={fieldGrid} onSubmit={(event) => void onSubmit(event)}>
        <div class={field}>
          <label class={fieldLabel} for="invite-name">
            Name
          </label>
          <input
            id="invite-name"
            class={input}
            autocomplete="off"
            value={name()}
            onInput={(event) => setName(event.currentTarget.value)}
          />
        </div>
        <div class={field}>
          <label class={fieldLabel} for="invite-email">
            Email
          </label>
          <input
            id="invite-email"
            class={input}
            type="email"
            autocomplete="off"
            value={email()}
            onInput={(event) => setEmail(event.currentTarget.value)}
          />
        </div>
        <UserAccessFields
          idPrefix="invite"
          projects={props.projects}
          value={access()}
          onChange={setAccess}
        />
        <div>
          <button type="submit" class={primaryButton} disabled={sending()}>
            {sending() ? "Inviting…" : "Invite"}
          </button>
        </div>
      </form>

      <Show when={errorMessage()}>
        <p class={`${statusMessage} ${statusError}`}>{errorMessage()}</p>
      </Show>
      <Show when={sent()}>
        {(invite) => (
          <>
            <p class={`${statusMessage} ${statusSuccess}`}>
              {invite().emailed
                ? `Emailed an invite to ${invite().email}. You can also send them this link:`
                : `Send this link to ${invite().email}. It expires in 7 days:`}
            </p>
            <div class={inviteLinkRow}>
              <input
                class={`${input} ${inviteLink}`}
                readOnly
                aria-label="Invite link"
                value={invite().url}
              />
              <button
                type="button"
                class={primaryButton}
                onClick={() => void copyLink(invite().url)}
              >
                {copied() ? "Copied" : "Copy"}
              </button>
            </div>
          </>
        )}
      </Show>
    </section>
  );
}
