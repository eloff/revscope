/// <reference types="solid-js" />

import { getApiBaseUrl } from "@revscope/web-shared/apiBaseUrl";
import { publicApiPath } from "@revscope/web-shared/apiPaths";
import {
  error,
  form,
  formCard,
  formTitle,
  input,
  label,
  primaryButton,
  success,
} from "@revscope/web-shared/authFormStyles";
import { MIN_PASSWORD_LENGTH, validatePassword } from "@revscope/web-shared/authValidation";
import { openLoginDialog } from "@revscope/web-shared/LoginDialog";
import { createSignal, Show } from "solid-js";

type ResetPasswordResponse = {
  error?: string;
};

function readTokenFromUrl(): string {
  if (typeof window === "undefined") {
    return "";
  }
  return new URLSearchParams(window.location.search).get("token")?.trim() ?? "";
}

/** Invites are reset links for a user who has no password yet. */
function isInvite(): boolean {
  return typeof window !== "undefined" && new URLSearchParams(window.location.search).has("invite");
}

export default function ResetPasswordForm() {
  const token = readTokenFromUrl();
  const invite = isInvite();
  const [password, setPassword] = createSignal("");
  const [errorMessage, setErrorMessage] = createSignal(
    token ? "" : "Password reset link is missing a token.",
  );
  const [successMessage, setSuccessMessage] = createSignal("");
  const [isSubmitting, setIsSubmitting] = createSignal(false);

  const onSubmit = async (event: SubmitEvent & { currentTarget: HTMLFormElement }) => {
    event.preventDefault();
    setErrorMessage("");
    setSuccessMessage("");

    if (!token) {
      setErrorMessage("Password reset link is missing a token.");
      return;
    }

    const passwordValue = password();
    const validationError = validatePassword(passwordValue);
    if (validationError) {
      setErrorMessage(validationError);
      return;
    }

    setIsSubmitting(true);
    try {
      const response = await fetch(new URL(publicApiPath("auth/reset-password"), getApiBaseUrl()), {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ token, password: passwordValue }),
      });
      const payload = (await response.json()) as ResetPasswordResponse;
      if (!response.ok) {
        setErrorMessage(payload.error ?? "Unable to reset password.");
        return;
      }
      setPassword("");
      setSuccessMessage(
        invite
          ? "Password set. Log in to get started."
          : "Password updated. You can log in with your new password.",
      );
    } catch {
      setErrorMessage("Unable to reach the server.");
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <div class={formCard}>
      <h2 class={formTitle}>{invite ? "Choose your password" : "Choose a new password"}</h2>
      <form class={form} onSubmit={onSubmit}>
        <label class={label} for="reset-password">
          {invite ? "Password" : "New password"}
        </label>
        <input
          id="reset-password"
          name="password"
          type="password"
          autocomplete="new-password"
          required
          minLength={MIN_PASSWORD_LENGTH}
          class={input}
          value={password()}
          onInput={(event) => setPassword(event.currentTarget.value)}
        />
        <button type="submit" class={primaryButton} disabled={isSubmitting()}>
          {isSubmitting() ? "Saving..." : invite ? "Set password" : "Update password"}
        </button>
      </form>
      {errorMessage() && <p class={error}>{errorMessage()}</p>}
      {successMessage() && <p class={success}>{successMessage()}</p>}
      <Show when={successMessage()}>
        <button type="button" class={primaryButton} onClick={openLoginDialog}>
          Log in
        </button>
      </Show>
    </div>
  );
}
