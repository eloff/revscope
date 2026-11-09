/// <reference types="solid-js" />
import { type AuthSession, useAuthState } from "@revscope/web-shared/authState";
import { projectOptions } from "@revscope/web-shared/projectSelection";
import { createMemo, createResource, For, Show } from "solid-js";
import { fetchAccountUsers } from "../../lib/usersApi";
import {
  header,
  heading,
  message,
  page,
  section,
  sectionDescription,
  sectionTitle,
  statusError,
  statusMessage,
  subtitle,
} from "../Settings.css";
import InviteUserForm from "./InviteUserForm";
import { userList } from "./ManageUsers.css";
import UserRow from "./UserRow";

/** The `/settings/users` page: everyone on the account, and inviting more. Admins only. */
export default function ManageUsers() {
  const { authUser, authResolved } = useAuthState();

  return (
    <div class={page}>
      <header class={header}>
        <h1 class={heading}>Users</h1>
        <p class={subtitle}>Invite people to this account and choose what they can see.</p>
      </header>
      <Show when={authResolved()} fallback={<p class={message}>Loading users…</p>}>
        <Show when={authUser()} fallback={<p class={message}>Log in to manage users.</p>}>
          {(user) => (
            <Show
              when={user().is_admin}
              fallback={<p class={message}>Only account admins can manage users.</p>}
            >
              <UserDirectory session={user()} />
            </Show>
          )}
        </Show>
      </Show>
    </div>
  );
}

function UserDirectory(props: { session: AuthSession }) {
  const [directory, { refetch }] = createResource(() => fetchAccountUsers(props.session));
  // Reading an errored resource rethrows, so check the error first.
  const loaded = () => (directory.error ? undefined : directory());
  // Kept through a failed reload, which would otherwise unmount the form and lose
  // the invite link it's showing. A failed first load shows the form, since a
  // successful invite is what reloads the list and the server still decides.
  const invitesIncluded = createMemo<boolean | undefined>(
    (previous) => loaded()?.invites_included ?? previous ?? (directory.error ? true : undefined),
  );
  // An admin's session lists every project on the account.
  const projects = createMemo(() => projectOptions(props.session.projects));

  return (
    <>
      <section class={section}>
        <h2 class={sectionTitle}>Everyone on this account</h2>
        <p class={sectionDescription}>
          Admins manage billing, users and projects. Members read analytics for every project or
          only the ones you choose.
        </p>
        <Show when={directory.error}>
          <p class={`${statusMessage} ${statusError}`}>{(directory.error as Error).message}</p>
        </Show>
        <ul class={userList}>
          <For each={loaded()?.users ?? []}>
            {(user) => (
              <UserRow
                session={props.session}
                user={user}
                projects={projects()}
                isSelf={user.user_id === props.session.user_id}
                onChanged={() => void refetch()}
              />
            )}
          </For>
        </ul>
      </section>
      <Show when={invitesIncluded()}>
        <InviteUserForm
          session={props.session}
          projects={projects()}
          onInvited={() => void refetch()}
        />
      </Show>
      <Show when={invitesIncluded() === false}>
        <section class={section}>
          <h2 class={sectionTitle}>Invite someone</h2>
          <p class={sectionDescription}>
            Team members need an active Starter or higher plan.{" "}
            <a href="/billing">Upgrade your plan</a> to invite people.
          </p>
        </section>
      </Show>
    </>
  );
}
