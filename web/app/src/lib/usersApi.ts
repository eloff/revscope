import { getApiBaseUrl } from "@revscope/web-shared/apiBaseUrl";
import { sessionApiPath } from "@revscope/web-shared/apiPaths";
import { authRequestInit, fetchJson } from "@revscope/web-shared/apiRequest";
import type { AuthSession } from "@revscope/web-shared/authState";

/** What a user may do. Admins always reach every project, so `project_ids` is null for them. */
export type UserAccess = {
  is_admin: boolean;
  /** Null for every project on the account, including ones added later. */
  project_ids: number[] | null;
};

export type AccountUser = UserAccess & {
  user_id: string;
  name: string;
  email: string;
  /** Invited but hasn't chosen a password yet. */
  pending: boolean;
  /** Unix seconds; null if they've never logged in. */
  last_login_at: number | null;
};

export type InviteUserResponse = {
  invite_url: string;
  /** False when the server can't send email; share the link yourself. */
  emailed: boolean;
};

function postJson<T>(session: AuthSession, path: string, body: unknown, failure: string) {
  return fetchJson<T>(
    new URL(sessionApiPath(path), getApiBaseUrl()),
    authRequestInit(session, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    }),
    failure,
  );
}

/** Admins only: everyone on the account, oldest first. */
export async function fetchAccountUsers(session: AuthSession): Promise<AccountUser[]> {
  const url = new URL(sessionApiPath("users"), getApiBaseUrl());
  const result = await fetchJson<{ users?: AccountUser[] }>(
    url,
    authRequestInit(session),
    "Unable to load users.",
  );
  if (!result.users) {
    throw new Error("Unable to load users.");
  }
  return result.users;
}

/** Inviting a pending user again replaces their link and their access. */
export function inviteUser(
  session: AuthSession,
  invite: { name: string; email: string } & UserAccess,
): Promise<InviteUserResponse> {
  return postJson(session, "users/invite", invite, "Unable to send the invite.");
}

export async function updateUserAccess(
  session: AuthSession,
  userId: string,
  access: UserAccess,
): Promise<void> {
  await postJson(session, "users/update", { user_id: userId, ...access }, "Unable to save access.");
}

export async function removeUser(session: AuthSession, userId: string): Promise<void> {
  await postJson(session, "users/remove", { user_id: userId }, "Unable to remove the user.");
}
