import { type AuthSession, revalidateAuthSession } from "./authState";

/**
 * Session auth rides on the `HttpOnly` cookie, so an authenticated request
 * differs only by opting into credentials. Requests made without a signed-in
 * user omit the cookie so the API answers from the public project instead of
 * refusing the signed-in account access to someone else's data.
 */
export function authRequestInit(session?: AuthSession, init: RequestInit = {}): RequestInit {
  return { ...init, credentials: session ? "include" : "omit" };
}

/**
 * Re-checks the session after a 401, and signs the tab out only once
 * `/v2/session/me` refuses it too. The API keeps 401 for dead credentials
 * (a project the caller can't access is a 404), but a 401 about some other
 * credential, such as a sign-in attempt, mustn't end a live session.
 */
export function noticeUnauthorized(response: Response): void {
  if (response.status === 401) {
    void revalidateAuthSession();
  }
}

/**
 * Re-reads the account after a project turned out to be deleted or taken from
 * this user since the tab loaded its list, so the dashboard moves to one they can use.
 */
export function noticeProjectGone(status: number, error: unknown): void {
  if (status === 404 && error === "project not found") {
    void revalidateAuthSession();
  }
}

/**
 * Reads an API answer's JSON, throwing the server's `error` message, or
 * `fallbackError` when it gave none. A body that is not JSON, like a proxy's
 * HTML 502 page, also throws `fallbackError` rather than a `SyntaxError`.
 */
export async function readJson<T>(response: Response, fallbackError: string): Promise<T> {
  noticeUnauthorized(response);
  const payload = (await response.json().catch(() => undefined)) as
    | (T & { error?: unknown })
    | undefined;
  noticeProjectGone(response.status, payload?.error);
  if (!response.ok || payload === undefined) {
    throw new Error(typeof payload?.error === "string" ? payload.error : fallbackError);
  }
  return payload;
}

/** Fetches `url` and answers its JSON, failing as {@link readJson} does. */
export async function fetchJson<T>(url: URL, init: RequestInit, fallbackError: string): Promise<T> {
  return readJson<T>(await fetch(url, init), fallbackError);
}
