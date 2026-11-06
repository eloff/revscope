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
 * Re-checks the session after a 401. The API also answers 401 to a live
 * session asking for a project it can't access, so this signs the tab out
 * only once `/v2/session/me` refuses the session too; a missing permission
 * leaves it signed in.
 */
export function noticeUnauthorized(response: Response): void {
  if (response.status === 401) {
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
  if (!response.ok || payload === undefined) {
    throw new Error(typeof payload?.error === "string" ? payload.error : fallbackError);
  }
  return payload;
}

/** Fetches `url` and answers its JSON, failing as {@link readJson} does. */
export async function fetchJson<T>(url: URL, init: RequestInit, fallbackError: string): Promise<T> {
  return readJson<T>(await fetch(url, init), fallbackError);
}
