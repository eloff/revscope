import { getApiBaseUrl } from "./apiBaseUrl";
import { publicApiPath } from "./apiPaths";
import { readJson } from "./apiRequest";
import { type AuthSession, parseAuthSession, saveAuthSession } from "./authState";
import { rememberLoginMethod } from "./lastLoginMethod";

export type SignInProvider = "google" | "apple";

/** The OAuth client id the server takes each provider's tokens for, or null when it's off. */
export type SignInProviders = Record<SignInProvider, string | null>;

/** What a Google or Apple button hands its form. */
export type IdTokenSignIn = {
  provider: SignInProvider;
  idToken: string;
  /** Apple's name for the visitor, which only the browser ever sees. */
  name?: string;
};

const NO_PROVIDERS: SignInProviders = { google: null, apple: null };

/** The server's answer, once one arrives: it only changes when the server restarts. */
let knownProviders: SignInProviders | undefined;
let providersRequest: Promise<SignInProviders> | undefined;

/** Which sign-in buttons to show, if this page has already asked the server. */
export function knownSignInProviders(): SignInProviders | undefined {
  return knownProviders;
}

/**
 * Which sign-in buttons to show, asked of the server once per page load. A
 * failed request shows none and isn't kept, so the next form asks again.
 */
export function fetchSignInProviders(): Promise<SignInProviders> {
  if (!providersRequest) {
    const request = requestSignInProviders().then((providers) => {
      // A request `forgetSignInProviders` dropped mustn't fill in for a newer one.
      if (providersRequest === request) {
        if (providers) {
          knownProviders = providers;
        } else {
          providersRequest = undefined;
        }
      }
      return providers ?? NO_PROVIDERS;
    });
    providersRequest = request;
  }
  return providersRequest;
}

/** The server's answer, or undefined when it couldn't be had. */
async function requestSignInProviders(): Promise<SignInProviders | undefined> {
  try {
    const response = await fetch(new URL(publicApiPath("auth/providers"), getApiBaseUrl()));
    const payload = (await response.json()) as Partial<SignInProviders>;
    if (!response.ok) {
      return undefined;
    }
    return {
      google: typeof payload.google === "string" ? payload.google : null,
      apple: typeof payload.apple === "string" ? payload.apple : null,
    };
  } catch {
    return undefined;
  }
}

/** Tests answer the providers route differently each time, so each starts unasked. */
export function forgetSignInProviders(): void {
  knownProviders = undefined;
  providersRequest = undefined;
}

/**
 * Logs in with a Google or Apple ID token, or, given the new account's fields,
 * signs up when the sign-in has no account yet. Saves and answers the session;
 * throws the server's message on failure.
 */
export async function signInWithIdToken(
  signIn: IdTokenSignIn,
  signup?: Record<string, string>,
): Promise<AuthSession> {
  let response: Response;
  try {
    // `credentials` so the browser keeps the session cookie the API sets.
    response = await fetch(new URL(publicApiPath("auth/id-token"), getApiBaseUrl()), {
      method: "POST",
      credentials: "include",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        provider: signIn.provider,
        id_token: signIn.idToken,
        name: signIn.name,
        signup,
      }),
    });
  } catch {
    throw new Error("Unable to reach the server.");
  }
  const fallback = signup ? "Unable to sign up." : "Unable to log in.";
  const session = parseAuthSession(await readJson<unknown>(response, fallback));
  if (!session) {
    throw new Error(fallback);
  }
  saveAuthSession(session);
  rememberLoginMethod(signIn.provider);
  return session;
}
