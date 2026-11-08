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
} from "@revscope/web-shared/authFormStyles";
import { parseAuthSession, saveAuthSession } from "@revscope/web-shared/authState";
import {
  EMAIL_PATTERN,
  MIN_PASSWORD_LENGTH,
  MIN_USER_NAME_LENGTH,
  validateEmail,
  validatePassword,
  validateUserName,
} from "@revscope/web-shared/authValidation";
import { browserTimezone } from "@revscope/web-shared/browserTimezone";
import { type IdTokenSignIn, signInWithIdToken } from "@revscope/web-shared/idTokenSignIn";
import { openLoginDialog } from "@revscope/web-shared/LoginDialog";
import { appPath, returnPathFrom } from "@revscope/web-shared/links";
import ProjectNameFields, { createProjectNameState } from "@revscope/web-shared/ProjectNameFields";
import { normalizeWebsite, validateProjectName } from "@revscope/web-shared/projectName";
import SocialSignIn from "@revscope/web-shared/SocialSignIn";
import { createSignal } from "solid-js";
import { loginPrompt, loginPromptButton } from "./SignupForm.css";

function emailFromUrl(): string {
  if (typeof window === "undefined") {
    return "";
  }
  return new URLSearchParams(window.location.search).get("email")?.trim() ?? "";
}

/**
 * Self-hosted signup: no plan; the user joins the instance's one shared
 * account, and names its first project.
 */
export default function SignupForm() {
  const project = createProjectNameState();
  const [name, setName] = createSignal("");
  const [email, setEmail] = createSignal(emailFromUrl());
  const [password, setPassword] = createSignal("");
  const [errorMessage, setErrorMessage] = createSignal("");
  const [isSubmitting, setIsSubmitting] = createSignal(false);

  const accountFields = () => ({
    account_name: normalizeWebsite(project.website()),
    project_name: project.projectName(),
    timezone: browserTimezone(),
  });

  /** Google and Apple sign up the project above their buttons, so it's checked first. */
  const onIdTokenSignIn = async (signIn: IdTokenSignIn) => {
    setErrorMessage("");
    const validationError = validateProjectName(project.projectName());
    if (validationError) {
      setErrorMessage(validationError);
      return;
    }
    setIsSubmitting(true);
    try {
      await signInWithIdToken(signIn, accountFields());
      window.location.assign(returnPathFrom(window.location.search) ?? appPath("/dashboard"));
    } catch (error) {
      setErrorMessage(error instanceof Error ? error.message : "Unable to sign up.");
    } finally {
      setIsSubmitting(false);
    }
  };

  const onSubmit = async (event: SubmitEvent & { currentTarget: HTMLFormElement }) => {
    event.preventDefault();
    setErrorMessage("");

    const trimmedName = name().trim();
    const trimmedEmail = email().trim();
    const validationError =
      validateProjectName(project.projectName()) ??
      validateUserName(trimmedName) ??
      validateEmail(trimmedEmail) ??
      validatePassword(password());
    if (validationError) {
      setErrorMessage(validationError);
      return;
    }

    setIsSubmitting(true);
    try {
      const response = await fetch(new URL(publicApiPath("auth/signup"), getApiBaseUrl()), {
        method: "POST",
        credentials: "include",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          name: trimmedName,
          email: trimmedEmail,
          password: password(),
          ...accountFields(),
        }),
      });
      const payload = (await response.json()) as { error?: string };
      if (!response.ok) {
        setErrorMessage(payload.error ?? "Unable to sign up.");
        return;
      }
      // Signup answers with the same body as `/v2/session/me`.
      const session = parseAuthSession(payload);
      if (!session) {
        setErrorMessage("Unable to sign up.");
        return;
      }
      saveAuthSession(session);
      window.location.assign(returnPathFrom(window.location.search) ?? appPath("/dashboard"));
    } catch {
      setErrorMessage("Unable to reach the server.");
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <div class={formCard}>
      <h2 class={formTitle}>Create your account</h2>
      <form class={form} onSubmit={onSubmit}>
        <ProjectNameFields state={project} websiteRequired={false} />
        <SocialSignIn
          dividerLabel="or sign up with email"
          onSignIn={(signIn) => void onIdTokenSignIn(signIn)}
        />
        <label class={label} for="signup-name">
          Name
        </label>
        <input
          id="signup-name"
          name="name"
          autocomplete="name"
          required
          minLength={MIN_USER_NAME_LENGTH}
          class={input}
          value={name()}
          onInput={(event) => setName(event.currentTarget.value)}
        />
        <label class={label} for="signup-email">
          Email
        </label>
        <input
          id="signup-email"
          name="email"
          type="email"
          autocomplete="email"
          required
          pattern={EMAIL_PATTERN.source}
          class={input}
          value={email()}
          onInput={(event) => setEmail(event.currentTarget.value)}
        />
        <label class={label} for="signup-password">
          Password
        </label>
        <input
          id="signup-password"
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
          {isSubmitting() ? "Creating account..." : "Sign up"}
        </button>
      </form>
      {errorMessage() && <p class={error}>{errorMessage()}</p>}
      <p class={loginPrompt}>
        Already have an account?{" "}
        <button type="button" class={loginPromptButton} onClick={openLoginDialog}>
          Log in
        </button>
      </p>
    </div>
  );
}
