/// <reference types="solid-js" />
import { getApiBaseUrl } from "@revscope/web-shared/apiBaseUrl";
import { sessionApiPath } from "@revscope/web-shared/apiPaths";
import { authRequestInit, fetchJson } from "@revscope/web-shared/apiRequest";
import type { AuthSession } from "@revscope/web-shared/authState";
import { createSignal, Show } from "solid-js";
import { resendButton } from "./EmailVerificationBanner.css";
import { banner, bannerText } from "./UnpaidSubscriptionBanner.css";

type EmailVerificationBannerProps = {
  session: AuthSession | null | undefined;
};

/** Asks for a confirmed email on an account nobody has confirmed yet, and emails a link on request. */
export default function EmailVerificationBanner(props: EmailVerificationBannerProps) {
  const [sending, setSending] = createSignal(false);
  const [outcome, setOutcome] = createSignal("");

  const resend = async (session: AuthSession) => {
    setSending(true);
    setOutcome("");
    try {
      await fetchJson(
        new URL(sessionApiPath("me/verify-email"), getApiBaseUrl()),
        authRequestInit(session, { method: "POST" }),
        "Couldn't send a new link. Try again shortly.",
      );
      setOutcome(
        `Sending a link to ${session.email}. If it doesn't arrive in a few minutes, try again.`,
      );
    } catch (error) {
      setOutcome(error instanceof Error ? error.message : "Couldn't send a new link.");
    } finally {
      setSending(false);
    }
  };

  return (
    <Show when={props.session && !props.session.account_confirmed ? props.session : undefined}>
      {(session) => (
        <div class={banner} role="status">
          <p class={bannerText}>
            Confirm your email address to turn on Ask AI, Google Analytics imports and integrations
            for this account. <Show when={outcome()}>{(text) => text()}</Show>
          </p>
          <button
            type="button"
            class={resendButton}
            disabled={sending()}
            onClick={() => void resend(session())}
          >
            {sending() ? "Sending…" : "Email me a link"}
          </button>
        </div>
      )}
    </Show>
  );
}
