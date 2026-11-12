/// <reference types="solid-js" />

import { getApiBaseUrl } from "@revscope/web-shared/apiBaseUrl";
import { publicApiPath } from "@revscope/web-shared/apiPaths";
import { readJson } from "@revscope/web-shared/apiRequest";
import { error, formCard, formTitle, success } from "@revscope/web-shared/authFormStyles";
import { reloadAuthSession } from "@revscope/web-shared/authState";
import { createSignal, onMount, Show } from "solid-js";

function readTokenFromUrl(): string {
  if (typeof window === "undefined") {
    return "";
  }
  return new URLSearchParams(window.location.search).get("token")?.trim() ?? "";
}

/** Spends the token from a verification email as soon as the page opens. */
export default function VerifyEmail() {
  const token = readTokenFromUrl();
  const [state, setState] = createSignal<"checking" | "confirmed" | "failed">(
    token ? "checking" : "failed",
  );
  const [errorMessage, setErrorMessage] = createSignal(
    token ? "" : "This confirmation link is missing its token.",
  );

  onMount(async () => {
    if (!token) {
      return;
    }
    try {
      const response = await fetch(new URL(publicApiPath("auth/verify-email"), getApiBaseUrl()), {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ token }),
      });
      await readJson(response, "Couldn't confirm your email. Try the link again.");
      setState("confirmed");
      // A tab already signed in would otherwise keep showing the banner until its next refresh.
      void reloadAuthSession();
    } catch (failure) {
      setErrorMessage(failure instanceof Error ? failure.message : "Unable to reach the server.");
      setState("failed");
    }
  });

  return (
    <div class={formCard}>
      <h2 class={formTitle}>Confirm your email</h2>
      <Show when={state() === "checking"}>
        <p>Confirming…</p>
      </Show>
      <Show when={state() === "confirmed"}>
        <p class={success}>
          Your email is confirmed. Ask AI, Google Analytics imports and integrations are on.
        </p>
        <p>
          <a href="/dashboard">Go to your dashboard</a>
        </p>
      </Show>
      <Show when={state() === "failed"}>
        <p class={error}>{errorMessage()}</p>
      </Show>
    </div>
  );
}
