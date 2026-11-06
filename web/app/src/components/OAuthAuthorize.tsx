/// <reference types="solid-js" />

import { getApiBaseUrl } from "@revscope/web-shared/apiBaseUrl";
import { sessionApiPath } from "@revscope/web-shared/apiPaths";
import { noticeUnauthorized } from "@revscope/web-shared/apiRequest";
import { useAuthState } from "@revscope/web-shared/authState";
import LoginDialog from "@revscope/web-shared/LoginDialog";
import { appPath } from "@revscope/web-shared/links";
import { projectOptions } from "@revscope/web-shared/projectSelection";
import { createMemo, createSignal, onMount, Show } from "solid-js";
import {
  actionRow,
  approveButton,
  card,
  clientName,
  denyButton,
  errorMessage,
  fieldHelp,
  projectNames,
  redirectHost,
  requestSummary,
  scopeList,
  settingsLink,
  statusMessage,
} from "./OAuthAuthorize.css";

/** The authorization request, as the MCP client put it in the URL. */
type AuthorizationRequest = {
  clientId: string;
  redirectUri: string;
  state: string | null;
  codeChallenge: string;
  codeChallengeMethod: string | null;
  /** "cli" is the RevScope CLI asking for full access rather than an MCP client reading. */
  scope: string | null;
};

type ClientDescription = {
  client_id: string;
  client_name: string;
  redirect_uri: string;
};

type RedirectResponse = {
  redirect_url?: string;
  error_description?: string;
  error?: string;
};

function readAuthorizationRequest(): AuthorizationRequest | null {
  const query = new URLSearchParams(window.location.search);
  const clientId = query.get("client_id");
  const redirectUri = query.get("redirect_uri");
  const codeChallenge = query.get("code_challenge");
  if (!clientId || !redirectUri || !codeChallenge) {
    return null;
  }
  return {
    clientId,
    redirectUri,
    state: query.get("state"),
    codeChallenge,
    codeChallengeMethod: query.get("code_challenge_method"),
    scope: query.get("scope"),
  };
}

/** The host is what a user can actually judge; the full URI is often opaque. */
function hostOf(redirectUri: string): string {
  try {
    const url = new URL(redirectUri);
    return url.host || url.protocol.replace(":", "");
  } catch {
    return redirectUri;
  }
}

/**
 * The consent step of the MCP OAuth flow. An MCP client sends the user's browser
 * here; the page confirms who is asking and hands the browser on to the client's
 * redirect URI with an authorization code.
 *
 * There is no project to pick: the token covers every project on the account, so
 * the agent can move between them, and reach ones added later, without coming
 * back through here.
 *
 * The redirect URL is built by the API rather than here: it comes from a client
 * that registered itself unauthenticated, so it has to be checked against that
 * registration before a browser follows it.
 */
export default function OAuthAuthorize() {
  const { authUser, authResolved } = useAuthState();
  const [request, setRequest] = createSignal<AuthorizationRequest | null>(null);
  const [requestRead, setRequestRead] = createSignal(false);
  const [client, setClient] = createSignal<ClientDescription | null>(null);
  const [failure, setFailure] = createSignal("");
  const [isSubmitting, setIsSubmitting] = createSignal(false);

  const forCli = () => request()?.scope === "cli";
  const names = createMemo(() =>
    projectOptions(authUser()?.projects ?? {}).map((option) => option.label),
  );

  onMount(() => {
    setRequest(readAuthorizationRequest());
    setRequestRead(true);
  });

  /**
   * Names the client, and in doing so proves the redirect URI belongs to it.
   * Runs once the account is known, because the route is session-authed.
   */
  const loadClient = async () => {
    const current = request();
    if (!current || !authUser() || client()) {
      return;
    }
    const query = new URLSearchParams({
      client_id: current.clientId,
      redirect_uri: current.redirectUri,
    });
    try {
      const response = await fetch(
        new URL(`${sessionApiPath("mcp/authorize")}?${query.toString()}`, getApiBaseUrl()),
        { credentials: "include" },
      );
      noticeUnauthorized(response);
      const payload = (await response.json()) as ClientDescription & RedirectResponse;
      if (!response.ok) {
        setFailure(payload.error_description ?? "That authorization request is not valid.");
        return;
      }
      setClient(payload);
    } catch {
      setFailure("Unable to reach the server.");
    }
  };

  // Re-runs as the account resolves, and does nothing until it has.
  createMemo(() => {
    if (authResolved() && authUser() && request()) {
      void loadClient();
    }
  });

  const decide = async (approve: boolean) => {
    const current = request();
    if (!current) {
      return;
    }
    setFailure("");
    setIsSubmitting(true);
    try {
      const response = await fetch(new URL(sessionApiPath("mcp/authorize"), getApiBaseUrl()), {
        method: "POST",
        credentials: "include",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          client_id: current.clientId,
          redirect_uri: current.redirectUri,
          state: current.state,
          code_challenge: current.codeChallenge,
          code_challenge_method: current.codeChallengeMethod,
          // An MCP client's own scope values mean nothing to us, so only this one is sent.
          scope: forCli() ? "cli" : undefined,
          approve,
        }),
      });
      noticeUnauthorized(response);
      const payload = (await response.json()) as RedirectResponse;
      if (!response.ok || !payload.redirect_url) {
        setFailure(payload.error_description ?? "Unable to complete the request.");
        return;
      }
      // Leaves our origin for the client's callback, which is the last step of
      // the flow as far as the browser is concerned.
      window.location.assign(payload.redirect_url);
    } catch {
      setFailure("Unable to reach the server.");
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <div class={card}>
      <Show when={requestRead() && !request()}>
        <p class={statusMessage}>
          This page completes a connection started by an MCP client. Open it from your client rather
          than directly.
        </p>
      </Show>

      <Show when={request()}>
        <Show when={authResolved()} fallback={<p class={statusMessage}>Loading…</p>}>
          <Show
            when={authUser()}
            fallback={
              <>
                <p class={statusMessage}>
                  Log in to connect your RevScope account to this MCP client.
                </p>
                <div class={actionRow}>
                  <LoginDialog
                    triggerLabel="Log in"
                    triggerClass={approveButton}
                    redirectTo={`${window.location.pathname}${window.location.search}`}
                  />
                </div>
              </>
            }
          >
            <Show when={client()} fallback={<p class={statusMessage}>Checking the request…</p>}>
              {(described) => (
                <>
                  <h2 class={clientName}>{described().client_name}</h2>
                  <Show
                    when={forCli()}
                    fallback={
                      <>
                        <p class={requestSummary}>
                          wants read-only access to your RevScope account. After you approve, your
                          browser returns to{" "}
                          <span class={redirectHost}>{hostOf(described().redirect_uri)}</span>.
                        </p>
                        <ul class={scopeList}>
                          <li>
                            Read any project on this account with SQL, including ones you add later
                          </li>
                          <li>Read the schema those projects share</li>
                          <li>Nothing else: no writes, no billing, no other account</li>
                        </ul>
                        <p class={fieldHelp}>
                          <Show when={names().length > 0}>
                            Today that is <span class={projectNames}>{names().join(", ")}</span>.{" "}
                          </Show>
                          The access lasts a year. Revoke all MCP tokens from the danger zone in{" "}
                          <a class={settingsLink} href={appPath("/settings")}>
                            Settings
                          </a>
                          . Agents may keep working for up to five minutes.
                        </p>
                      </>
                    }
                  >
                    <p class={requestSummary}>
                      wants to act as you on your RevScope account. Only approve this if you just
                      ran <code>revscope login</code> on this computer. After you approve, your
                      browser returns to{" "}
                      <span class={redirectHost}>{hostOf(described().redirect_uri)}</span>.
                    </p>
                    <ul class={scopeList}>
                      <li>Everything you can do yourself, including changing settings</li>
                      <li>Its access follows your role, and ends if you are removed</li>
                    </ul>
                    <p class={fieldHelp}>
                      The access lasts a year. <code>revscope logout</code> ends it, and so does
                      changing your password.
                    </p>
                  </Show>

                  <Show when={!forCli() && !authUser()?.is_admin}>
                    <p class={errorMessage}>
                      Only account admins can connect an agent. Ask an admin to approve it.
                    </p>
                  </Show>
                  <div class={actionRow}>
                    <Show when={forCli() || authUser()?.is_admin}>
                      <button
                        type="button"
                        class={approveButton}
                        disabled={isSubmitting()}
                        onClick={() => void decide(true)}
                      >
                        {isSubmitting() ? "Connecting…" : "Approve"}
                      </button>
                    </Show>
                    <button
                      type="button"
                      class={denyButton}
                      disabled={isSubmitting()}
                      onClick={() => void decide(false)}
                    >
                      Deny
                    </button>
                  </div>
                </>
              )}
            </Show>
          </Show>
        </Show>
      </Show>

      {/* Every failure comes from a signed-in request. A 401 whose session /me
          also refuses signs the tab out, and the login prompt hides it. */}
      <Show when={authUser() && failure()}>
        <p class={errorMessage}>{failure()}</p>
      </Show>
    </div>
  );
}
