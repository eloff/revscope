import type { SignInProvider } from "./idTokenSignIn";

export type LoginMethod = SignInProvider | "email";

const COOKIE_NAME = "revscope_last_login";
const ONE_YEAR_SECONDS = 365 * 24 * 60 * 60;
// A record so a new method can't compile without being added here.
const LOGIN_METHODS: Record<LoginMethod, true> = { google: true, apple: true, email: true };

/**
 * Starts the year over: only a successful login or signup should call this.
 * Safari's ITP caps cookies written from script at 7 days, whatever Max-Age says,
 * so there the method is forgotten a week after the last login.
 */
export function rememberLoginMethod(method: LoginMethod): void {
  // Self-hosted instances may run on plain http, where a Secure cookie would be dropped.
  const secure = window.location.protocol === "https:" ? "; Secure" : "";
  // biome-ignore lint/suspicious/noDocumentCookie: the Cookie Store API is missing from Safari before 18.4.
  document.cookie = `${COOKIE_NAME}=${method}; Max-Age=${ONE_YEAR_SECONDS}; Path=/; SameSite=Lax${secure}`;
}

export function readLastLoginMethod(): LoginMethod | undefined {
  if (typeof document === "undefined") {
    return undefined;
  }
  const prefix = `${COOKIE_NAME}=`;
  const value = document.cookie
    .split("; ")
    .find((cookie) => cookie.startsWith(prefix))
    ?.slice(prefix.length);
  return value !== undefined && Object.hasOwn(LOGIN_METHODS, value)
    ? (value as LoginMethod)
    : undefined;
}
