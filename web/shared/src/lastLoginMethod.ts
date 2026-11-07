import type { SignInProvider } from "./idTokenSignIn";

export type LoginMethod = SignInProvider | "email";

/** The API sets this on each successful login or signup (`routes/session.rs` in the backend). */
const COOKIE_NAME = "revscope_last_login_method";
// A record so a new method can't compile without being added here.
const LOGIN_METHODS: Record<LoginMethod, true> = { google: true, apple: true, email: true };

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
