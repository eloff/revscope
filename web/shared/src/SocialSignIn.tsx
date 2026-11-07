/// <reference types="solid-js" />

import { createSignal, onCleanup, onMount, Show } from "solid-js";
import { appleSignIn, loadAppleJs } from "./appleSignIn";
import { receiveGoogleSignIns, renderGoogleButton } from "./googleSignIn";
import { fetchSignInProviders, type IdTokenSignIn, knownSignInProviders } from "./idTokenSignIn";
import LastUsedFrame from "./LastUsedFrame";
import type { LoginMethod } from "./lastLoginMethod";
import { appleButton, buttons, divider, error, googleButton } from "./SocialSignIn.css";

type SocialSignInProps = {
  /** Gets each ID token a button produces; the form decides what to do with it. */
  onSignIn: (signIn: IdTokenSignIn) => void;
  /** Labels the divider below the buttons, between them and the email form. */
  dividerLabel?: string;
  /** Marks this method's button as the one used last, if it's one of these. */
  lastUsed?: LoginMethod;
};

/**
 * "Continue with Google" and "Continue with Apple", for whichever the server
 * has a client id for. Renders nothing when neither is on.
 */
export default function SocialSignIn(props: SocialSignInProps) {
  // Known already when another form on the page asked, so the buttons show
  // at once instead of popping in.
  const [providers, setProviders] = createSignal(knownSignInProviders());
  if (!providers()) {
    void fetchSignInProviders().then(setProviders);
  }
  // On mount, so Google's sign-ins go to forms in the order they appeared,
  // whatever the providers request is doing.
  onCleanup(receiveGoogleSignIns((idToken) => props.onSignIn({ provider: "google", idToken })));
  return (
    <Show when={providers() && (providers()?.google || providers()?.apple)}>
      <div class={buttons}>
        <Show when={providers()?.google}>
          {(clientId) => (
            <LastUsedFrame active={props.lastUsed === "google"}>
              <GoogleButton clientId={clientId()} />
            </LastUsedFrame>
          )}
        </Show>
        <Show when={providers()?.apple}>
          {(servicesId) => (
            <LastUsedFrame active={props.lastUsed === "apple"}>
              <AppleButton servicesId={servicesId()} onSignIn={props.onSignIn} />
            </LastUsedFrame>
          )}
        </Show>
      </div>
      <p class={divider}>{props.dividerLabel ?? "or use your email"}</p>
    </Show>
  );
}

function GoogleButton(props: { clientId: string }) {
  const [failed, setFailed] = createSignal("");
  let parent!: HTMLDivElement;
  onMount(() => {
    renderGoogleButton(parent, props.clientId).catch(() =>
      setFailed("Couldn't load Google sign-in. Check your connection."),
    );
  });
  return (
    <>
      <div class={googleButton} ref={parent} />
      <Show when={failed()}>
        <p class={error}>{failed()}</p>
      </Show>
    </>
  );
}

function AppleButton(props: { servicesId: string; onSignIn: (signIn: IdTokenSignIn) => void }) {
  const [ready, setReady] = createSignal(false);
  const [failed, setFailed] = createSignal("");
  onMount(() => {
    loadAppleJs(props.servicesId)
      .then(() => setReady(true))
      .catch(() => setFailed("Couldn't load Apple sign-in. Check your connection."));
  });
  const signIn = () => {
    setFailed("");
    appleSignIn()
      .then((result) => result && props.onSignIn({ provider: "apple", ...result }))
      .catch(() => setFailed("Apple sign-in didn't work. Try again."));
  };
  return (
    <>
      <button type="button" class={appleButton} onClick={signIn} disabled={!ready()}>
        <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
          <path d={APPLE_LOGO} />
        </svg>
        Continue with Apple
      </button>
      <Show when={failed()}>
        <p class={error}>{failed()}</p>
      </Show>
    </>
  );
}

const APPLE_LOGO =
  "M12.152 6.896c-.948 0-2.415-1.078-3.96-1.04-2.04.027-3.91 1.183-4.961 3.014-2.117 3.675-.546 9.103 1.519 12.09 1.013 1.454 2.208 3.09 3.792 3.039 1.52-.065 2.09-.987 3.935-.987 1.831 0 2.35.987 3.96.948 1.637-.026 2.676-1.48 3.676-2.948 1.156-1.688 1.636-3.325 1.662-3.415-.039-.013-3.182-1.221-3.22-4.857-.026-3.04 2.48-4.494 2.597-4.559-1.429-2.09-3.623-2.324-4.39-2.376-2-.156-3.675 1.09-4.61 1.09zM15.53 3.83c.843-1.012 1.4-2.427 1.245-3.83-1.207.052-2.662.805-3.532 1.818-.78.896-1.454 2.338-1.273 3.714 1.338.104 2.715-.688 3.559-1.701";
