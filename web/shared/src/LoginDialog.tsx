/// <reference types="solid-js" />

import { Dialog } from "@ark-ui/solid/dialog";
import {
  createEffect,
  createMemo,
  createSignal,
  createUniqueId,
  on,
  onCleanup,
  onMount,
  Show,
} from "solid-js";
import { Portal } from "solid-js/web";
import { error, form, input, label, primaryButton, success } from "./AuthForm.css";
import { getApiBaseUrl } from "./apiBaseUrl";
import { publicApiPath } from "./apiPaths";
import { parseAuthSession, saveAuthSession } from "./authState";
import {
  EMAIL_PATTERN,
  MIN_PASSWORD_LENGTH,
  validateEmail,
  validateLoginPassword,
} from "./authValidation";
import { type IdTokenSignIn, signInWithIdToken } from "./idTokenSignIn";
import LastUsedFrame from "./LastUsedFrame";
import {
  actionPrimaryButton,
  actionRow,
  backdrop,
  content,
  description,
  iconCloseButton,
  positioner,
  rememberMeInput,
  rememberMeRow,
  secondaryButton,
  textButton,
  title,
} from "./LoginDialog.css";
import { readLastLoginMethod } from "./lastLoginMethod";
import { appPath, SELF_HOSTED, signupPath } from "./links";
import SocialSignIn from "./SocialSignIn";

type LoginDialogProps = {
  triggerLabel?: string;
  triggerClass?: string;
  /**
   * Where to go once logged in. Defaults to the dashboard; the MCP consent page
   * points it back at itself so the flow it was in the middle of continues.
   */
  redirectTo?: string;
  /**
   * Controls the dialog from outside. A controlled dialog renders no trigger
   * and ignores openLoginDialog, which is for the header's own dialog.
   */
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
};

type DialogMode = "login" | "forgot-password";

const REMEMBERED_EMAIL_STORAGE_KEY = "revscope.auth.rememberedEmail";
const OPEN_LOGIN_EVENT = "revscope:open-login";

/** Opens any mounted login dialog (e.g. the one in the site header). */
export function openLoginDialog(): void {
  if (typeof window === "undefined") {
    return;
  }
  window.dispatchEvent(new CustomEvent(OPEN_LOGIN_EVENT));
}

function readRememberedEmail(): string {
  if (typeof window === "undefined") {
    return "";
  }
  return window.localStorage.getItem(REMEMBERED_EMAIL_STORAGE_KEY) ?? "";
}

function writeRememberedEmail(email: string | null): void {
  if (typeof window === "undefined") {
    return;
  }
  if (email) {
    window.localStorage.setItem(REMEMBERED_EMAIL_STORAGE_KEY, email);
    return;
  }
  window.localStorage.removeItem(REMEMBERED_EMAIL_STORAGE_KEY);
}

export default function LoginDialog(props: LoginDialogProps) {
  const idPrefix = createUniqueId();
  const emailInputId = `${idPrefix}-login-email`;
  const passwordInputId = `${idPrefix}-login-password`;
  const rememberMeInputId = `${idPrefix}-login-remember-me`;
  const rememberedEmail = readRememberedEmail();
  const [uncontrolledOpen, setUncontrolledOpen] = createSignal(false);
  const controlled = () => props.open !== undefined;
  const open = () => props.open ?? uncontrolledOpen();
  const setOpen = (next: boolean) => {
    if (!controlled()) {
      setUncontrolledOpen(next);
    }
    props.onOpenChange?.(next);
  };
  const [mode, setMode] = createSignal<DialogMode>("login");
  const [email, setEmail] = createSignal(rememberedEmail);
  const [password, setPassword] = createSignal("");
  const [rememberMe, setRememberMe] = createSignal(rememberedEmail.length > 0);
  const [errorMessage, setErrorMessage] = createSignal("");
  const [successMessage, setSuccessMessage] = createSignal("");
  const [isSubmitting, setIsSubmitting] = createSignal(false);
  // Hidden until a self-hosted instance confirms signup is still open. On
  // revscope.co it always is.
  const [signupOpen, setSignupOpen] = createSignal(!SELF_HOSTED);
  const isForgotPassword = () => mode() === "forgot-password";
  // Read on each open: another tab may have logged in with a different method.
  const lastUsed = createMemo(() => (open() ? readLastLoginMethod() : undefined));
  const dialogTitle = () => (isForgotPassword() ? "Reset password" : "Log in");
  const dialogDescription = () =>
    isForgotPassword()
      ? "Enter your email and we'll send a password reset link."
      : "Use your email and password to access your account.";
  const submitLabel = () =>
    isForgotPassword()
      ? isSubmitting()
        ? "Sending..."
        : "Send reset link"
      : isSubmitting()
        ? "Logging in..."
        : "Log in";

  const switchMode = (nextMode: DialogMode) => {
    setMode(nextMode);
    setPassword("");
    setErrorMessage("");
    setSuccessMessage("");
  };

  // The openLoginDialog listener resets the form on each open; a controlled
  // dialog skips it, so reset here instead. Otherwise it reopens where it was
  // left, such as on the reset-password form or a stale error.
  createEffect(
    on(
      () => props.open,
      (isOpen, wasOpen) => {
        if (isOpen && !wasOpen) {
          switchMode("login");
        }
      },
      { defer: true },
    ),
  );

  onMount(() => {
    if (!controlled()) {
      const onOpenLogin = () => {
        switchMode("login");
        setOpen(true);
      };
      window.addEventListener(OPEN_LOGIN_EVENT, onOpenLogin);
      onCleanup(() => window.removeEventListener(OPEN_LOGIN_EVENT, onOpenLogin));
    }

    if (!SELF_HOSTED) {
      return;
    }
    void fetch(new URL(publicApiPath("auth/signup"), getApiBaseUrl()))
      .then((response) => (response.ok ? response.json() : null))
      .then((payload: { open?: boolean } | null) => setSignupOpen(payload?.open === true))
      .catch(() => setSignupOpen(false));
  });

  const signupHref = () => {
    const trimmedEmail = email().trim();
    const signup = signupPath();
    return trimmedEmail ? `${signup}?email=${encodeURIComponent(trimmedEmail)}` : signup;
  };

  const onIdTokenSignIn = async (signIn: IdTokenSignIn) => {
    setErrorMessage("");
    setSuccessMessage("");
    setIsSubmitting(true);
    try {
      await signInWithIdToken(signIn);
      window.location.assign(props.redirectTo ?? appPath("/dashboard"));
    } catch (error) {
      setErrorMessage(error instanceof Error ? error.message : "Unable to log in.");
    } finally {
      setIsSubmitting(false);
    }
  };

  const onSubmit = async (event: SubmitEvent & { currentTarget: HTMLFormElement }) => {
    event.preventDefault();
    setErrorMessage("");
    setSuccessMessage("");

    const trimmedEmail = email().trim();
    const passwordValue = password();

    const validationError = isForgotPassword()
      ? validateEmail(trimmedEmail)
      : (validateEmail(trimmedEmail) ?? validateLoginPassword(passwordValue));
    if (validationError) {
      setErrorMessage(validationError);
      return;
    }

    setIsSubmitting(true);

    try {
      if (isForgotPassword()) {
        const response = await fetch(
          new URL(publicApiPath("auth/forgot-password"), getApiBaseUrl()),
          {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ email: trimmedEmail }),
          },
        );
        const payload = (await response.json()) as { error?: string };
        if (!response.ok) {
          setErrorMessage(payload.error ?? "Unable to send reset link.");
          return;
        }
        setSuccessMessage("If an account exists for that email, we'll send a password reset link.");
        return;
      }

      // `credentials` so the browser keeps the session cookie the API sets.
      const response = await fetch(new URL(publicApiPath("auth/login"), getApiBaseUrl()), {
        method: "POST",
        credentials: "include",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          username: trimmedEmail,
          password: passwordValue,
        }),
      });
      const payload = (await response.json()) as { error?: string };
      if (!response.ok) {
        setErrorMessage(payload.error ?? "Unable to log in.");
        return;
      }
      // Login answers with the same body as `/v2/session/me`.
      const session = parseAuthSession(payload);
      if (!session) {
        setErrorMessage("Unable to log in.");
        return;
      }
      saveAuthSession(session);
      writeRememberedEmail(rememberMe() ? trimmedEmail : null);
      window.location.assign(props.redirectTo ?? appPath("/dashboard"));
    } catch {
      setErrorMessage("Unable to reach the server.");
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <Dialog.Root
      preventScroll={false}
      open={open()}
      onOpenChange={(details) => setOpen(details.open)}
    >
      <Show when={!controlled()}>
        <Dialog.Trigger class={props.triggerClass}>{props.triggerLabel ?? "Log in"}</Dialog.Trigger>
      </Show>
      <Portal>
        <Dialog.Backdrop class={backdrop} />
        <Dialog.Positioner class={positioner}>
          <Dialog.Content class={content}>
            <Dialog.CloseTrigger class={iconCloseButton} aria-label="Close">
              X
            </Dialog.CloseTrigger>
            <Dialog.Title class={title}>{dialogTitle()}</Dialog.Title>
            <Dialog.Description class={description}>{dialogDescription()}</Dialog.Description>
            {/* Only while open: a closed dialog's Google button would take the
                sign-ins of the page's own, such as the signup form's. */}
            <Show when={open() && !isForgotPassword()}>
              <SocialSignIn
                lastUsed={lastUsed()}
                onSignIn={(signIn) => void onIdTokenSignIn(signIn)}
              />
            </Show>
            <LastUsedFrame active={!isForgotPassword() && lastUsed() === "email"}>
              <form class={form} onSubmit={onSubmit}>
                <label class={label} for={emailInputId}>
                  Email
                </label>
                <input
                  id={emailInputId}
                  name="email"
                  type="email"
                  autocomplete="username"
                  required
                  pattern={EMAIL_PATTERN.source}
                  class={input}
                  value={email()}
                  onInput={(event) => setEmail(event.currentTarget.value)}
                />
                {!isForgotPassword() && (
                  <>
                    <label class={label} for={passwordInputId}>
                      Password
                    </label>
                    <input
                      id={passwordInputId}
                      name="password"
                      type="password"
                      autocomplete="current-password"
                      required
                      minLength={MIN_PASSWORD_LENGTH}
                      class={input}
                      value={password()}
                      onInput={(event) => setPassword(event.currentTarget.value)}
                    />
                    <button
                      type="button"
                      class={textButton}
                      onClick={() => switchMode("forgot-password")}
                    >
                      Forgot password?
                    </button>
                    <label class={rememberMeRow} for={rememberMeInputId}>
                      <input
                        id={rememberMeInputId}
                        name="rememberMe"
                        type="checkbox"
                        class={rememberMeInput}
                        checked={rememberMe()}
                        onChange={(event) => setRememberMe(event.currentTarget.checked)}
                      />
                      Remember me
                    </label>
                  </>
                )}
                {isForgotPassword() && (
                  <button type="button" class={textButton} onClick={() => switchMode("login")}>
                    Back to log in
                  </button>
                )}
                <div class={actionRow}>
                  <button
                    type="submit"
                    class={`${primaryButton} ${actionPrimaryButton}`}
                    disabled={isSubmitting()}
                  >
                    {submitLabel()}
                  </button>
                  <Show when={signupOpen()}>
                    <a href={signupHref()} class={secondaryButton}>
                      Sign up
                    </a>
                  </Show>
                </div>
              </form>
            </LastUsedFrame>
            {errorMessage() && <p class={error}>{errorMessage()}</p>}
            {successMessage() && <p class={success}>{successMessage()}</p>}
          </Dialog.Content>
        </Dialog.Positioner>
      </Portal>
    </Dialog.Root>
  );
}
