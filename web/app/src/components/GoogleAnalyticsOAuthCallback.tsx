/// <reference types="solid-js" />
import { useAuthState } from "@revscope/web-shared/authState";
import { appPath } from "@revscope/web-shared/links";
import { setSelectedProjectId } from "@revscope/web-shared/projectSelection";
import { createEffect, createSignal, Show } from "solid-js";
import { connectOauth, projectIdFromOauthState, storePendingConnection } from "../lib/gaImportApi";
import { statusError, statusMessage } from "./Settings.css";

/** Where Google sends the browser after its consent screen. */
export default function GoogleAnalyticsOAuthCallback() {
  const { authUser, authResolved } = useAuthState();
  const [error, setError] = createSignal("");
  let finishing = false;

  const settingsUrl = appPath("/settings#google-analytics-import");

  createEffect(() => {
    if (!authResolved() || finishing) {
      return;
    }
    finishing = true;
    const query = new URLSearchParams(window.location.search);
    const code = query.get("code");
    const state = query.get("state");
    const projectId = state ? projectIdFromOauthState(state) : null;
    const user = authUser();
    if (query.get("error") || !code || !state || !projectId) {
      setError("Google didn't grant access to Google Analytics.");
      return;
    }
    if (!user) {
      setError("Log in, then connect to Google again from Settings.");
      return;
    }
    connectOauth(user, projectId, code, state)
      .then((connection) => {
        storePendingConnection(projectId, connection);
        setSelectedProjectId(projectId);
        window.location.replace(settingsUrl);
      })
      .catch((caught: unknown) => {
        setError(caught instanceof Error ? caught.message : "Unable to connect to Google.");
      });
  });

  return (
    <Show when={error()} fallback={<p>Connecting to Google Analytics…</p>}>
      <p class={`${statusMessage} ${statusError}`}>{error()}</p>
      <a href={settingsUrl}>Back to Settings</a>
    </Show>
  );
}
