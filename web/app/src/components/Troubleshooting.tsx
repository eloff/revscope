/// <reference types="solid-js" />

import { useAuthState } from "@revscope/web-shared/authState";
import { appPath, docsUrl } from "@revscope/web-shared/links";
import {
  projectOptions,
  resolveProjectId,
  selectedProjectId,
} from "@revscope/web-shared/projectSelection";
import { createEffect, createMemo, createSignal, For, on, onCleanup, Show } from "solid-js";
import { fetchSettings, finishedSyncError } from "../lib/settingsApi";
import {
  fetchTroubleshootingStatus,
  formatDetectedAt,
  formatEventsCount,
  formatSyncAt,
  type IntegrationStatus,
  integrationStatusLabel,
  integrationTypeLabel,
  pollTroubleshootingIngest,
  runTroubleshootingCheck,
  type TroubleshootingCheckResponse,
  type TroubleshootingIngestResponse,
  type TroubleshootingStatusResponse,
} from "../lib/troubleshootingApi";
import { primaryButton } from "./buttons.css";
import {
  badge,
  badgeError,
  badgeOk,
  docsLink,
  emptyState,
  errorMessage,
  field,
  fieldLabel,
  finding,
  findingError,
  findingOk,
  findings,
  findingWaiting,
  findingWarning,
  followUp,
  header,
  heading,
  input,
  inputRow,
  message,
  metric,
  metricLabel,
  mutedCell,
  page,
  section,
  sectionDescription,
  sectionTitle,
  subtitle,
  table,
  tableScroll,
} from "./Troubleshooting.css";

const INGEST_POLL_MS = 2000;
const INGEST_WAIT_TIMEOUT_MS = 2 * 60 * 1000;

function findingClass(severity: string) {
  if (severity === "ok") {
    return findingOk;
  }
  if (severity === "warning") {
    return findingWarning;
  }
  return findingError;
}

function statusBadgeClass(status: string) {
  if (status === "ok") {
    return badgeOk;
  }
  if (status === "error") {
    return badgeError;
  }
  return undefined;
}

function integrationsFromSettings(
  integrations: Awaited<ReturnType<typeof fetchSettings>>["integrations"],
): IntegrationStatus[] {
  return integrations.map((integration) => ({
    integration_type: integration.integration_type,
    last_synced_at: integration.last_synced_at,
    last_sync_error: finishedSyncError(integration),
    status: finishedSyncError(integration)
      ? "error"
      : integration.last_synced_at
        ? "ok"
        : "never_run",
  }));
}

export default function Troubleshooting() {
  const { authUser, authResolved } = useAuthState();
  const [status, setStatus] = createSignal<TroubleshootingStatusResponse | null>(null);
  const [integrations, setIntegrations] = createSignal<IntegrationStatus[]>([]);
  const [check, setCheck] = createSignal<TroubleshootingCheckResponse | null>(null);
  const [ingestHit, setIngestHit] = createSignal<TroubleshootingIngestResponse | null>(null);
  const [waitingForIngest, setWaitingForIngest] = createSignal(false);
  const [ingestTimedOut, setIngestTimedOut] = createSignal(false);
  const [url, setUrl] = createSignal("");
  const [urlTouched, setUrlTouched] = createSignal(false);
  const [loading, setLoading] = createSignal(true);
  const [checking, setChecking] = createSignal(false);
  const [loadError, setLoadError] = createSignal("");
  const [checkError, setCheckError] = createSignal("");
  const [pollError, setPollError] = createSignal("");

  let pollTimer: ReturnType<typeof setInterval> | undefined;
  let waitDeadline = 0;

  const stopIngestPoll = () => {
    if (pollTimer != null) {
      clearInterval(pollTimer);
      pollTimer = undefined;
    }
    setWaitingForIngest(false);
  };

  onCleanup(stopIngestPoll);

  const projects = createMemo(() => {
    const user = authUser();
    return user ? projectOptions(user.projects) : [];
  });

  const projectId = createMemo(() => resolveProjectId(projects(), selectedProjectId()));

  const projectName = createMemo(
    () =>
      status()?.project_name ??
      projects().find((project) => project.value === projectId())?.label ??
      "",
  );

  const load = async (id: string) => {
    const user = authUser();
    if (!user || !id) {
      setLoading(false);
      return;
    }
    setLoading(true);
    setLoadError("");
    try {
      const next = await fetchTroubleshootingStatus(user, id);
      setStatus(next);
      setIntegrations(next.integrations);
      if (!urlTouched()) {
        setUrl(next.default_url);
      }
    } catch (statusError) {
      setStatus(null);
      setLoadError(
        statusError instanceof Error
          ? statusError.message
          : "Unable to load troubleshooting status.",
      );
      try {
        const settings = await fetchSettings(user, id);
        setIntegrations(integrationsFromSettings(settings.integrations));
      } catch {
        setIntegrations([]);
      }
    } finally {
      setLoading(false);
    }
  };

  const pollOnce = async (id: string, sinceMs: number) => {
    const user = authUser();
    if (!user) {
      return;
    }
    try {
      const result = await pollTroubleshootingIngest(user, id, sinceMs);
      setPollError("");
      if (result.detected) {
        setIngestHit(result);
        stopIngestPoll();
        return;
      }
      if (Date.now() >= waitDeadline) {
        setIngestTimedOut(true);
        stopIngestPoll();
      }
    } catch (caught) {
      setPollError(caught instanceof Error ? caught.message : "Unable to poll ingest.");
    }
  };

  const startIngestPoll = (id: string, sinceMs: number) => {
    stopIngestPoll();
    setIngestHit(null);
    setIngestTimedOut(false);
    setPollError("");
    setWaitingForIngest(true);
    waitDeadline = Date.now() + INGEST_WAIT_TIMEOUT_MS;
    void pollOnce(id, sinceMs);
    pollTimer = setInterval(() => {
      void pollOnce(id, sinceMs);
    }, INGEST_POLL_MS);
  };

  createEffect(
    on([authResolved, projectId], ([resolved, id]) => {
      if (!resolved) {
        return;
      }
      if (!authUser() || !id) {
        setLoading(false);
        return;
      }
      stopIngestPoll();
      setCheck(null);
      setIngestHit(null);
      setIngestTimedOut(false);
      setCheckError("");
      setPollError("");
      setUrlTouched(false);
      void load(id);
    }),
  );

  const onCheck = async () => {
    const user = authUser();
    const id = projectId();
    const checkUrl = url().trim();
    if (!user || !id || !checkUrl) {
      return;
    }
    setChecking(true);
    setCheckError("");
    setIngestHit(null);
    setIngestTimedOut(false);
    setPollError("");
    stopIngestPoll();
    try {
      const result = await runTroubleshootingCheck(user, id, checkUrl);
      setCheck(result);
      startIngestPoll(id, result.wait_started_at_ms);
    } catch (caught) {
      setCheck(null);
      setCheckError(caught instanceof Error ? caught.message : "Unable to run installation check.");
    } finally {
      setChecking(false);
    }
  };

  return (
    <div class={page}>
      <header class={header}>
        <h1 class={heading}>Troubleshooting</h1>
        <p class={subtitle}>
          Check tracker installation, ingest, and integrations for the selected project.
        </p>
      </header>

      <Show when={authResolved()} fallback={<p class={message}>Loading…</p>}>
        <Show when={authUser()} fallback={<p class={message}>Log in to run troubleshooting.</p>}>
          <Show when={loadError()}>
            <p class={errorMessage}>{loadError()}</p>
          </Show>

          <section class={section}>
            <h2 class={sectionTitle}>Recent ingest</h2>
            <p class={sectionDescription}>
              Events flushed
              <Show when={projectName()}>
                {" "}
                for <strong>{projectName()}</strong>
              </Show>{" "}
              in the last 30 days (pageviews and custom events).
            </p>
            <Show
              when={loading()}
              fallback={
                <Show when={status()} fallback={<p class={emptyState}>Event count unavailable.</p>}>
                  {(loaded) => (
                    <>
                      <p class={metric}>{formatEventsCount(loaded().events_last_month)}</p>
                      <p class={metricLabel}>events in the last month</p>
                      <Show when={loaded().events_queued > 0}>
                        <p class={sectionDescription}>
                          Plus {formatEventsCount(loaded().events_queued)} received and waiting to
                          be saved, which happens every five minutes.
                        </p>
                      </Show>
                    </>
                  )}
                </Show>
              }
            >
              <p class={message}>Loading…</p>
            </Show>
          </section>

          <section class={section}>
            <h2 class={sectionTitle}>Tracker installation</h2>
            <p class={sectionDescription}>
              Enter the page URL to inspect (defaults to your account website). We fetch it from our
              servers and look for the RevScope stub, then wait for a real pageview from your site
              to confirm the tracker is sending data.
            </p>
            <div class={field}>
              <label class={fieldLabel} for="troubleshoot-url">
                URL to check
              </label>
              <div class={inputRow}>
                <input
                  id="troubleshoot-url"
                  class={input}
                  type="url"
                  value={url()}
                  placeholder="https://example.com/"
                  onInput={(event) => {
                    setUrlTouched(true);
                    setUrl(event.currentTarget.value);
                  }}
                />
                <button
                  type="button"
                  class={primaryButton}
                  disabled={checking() || !url().trim() || !projectId()}
                  onClick={() => void onCheck()}
                >
                  {checking() ? "Checking…" : "Check installation"}
                </button>
              </div>
            </div>

            <Show when={checkError()}>
              <p class={errorMessage}>{checkError()}</p>
            </Show>

            <Show when={check()}>
              {(result) => (
                <>
                  <ul class={findings}>
                    <li class={`${finding} ${result().fetch.ok ? findingOk : findingError}`}>
                      {result().fetch.ok
                        ? `Fetched ${result().url}${
                            result().fetch.status_code
                              ? ` (HTTP ${result().fetch.status_code})`
                              : ""
                          }.`
                        : `Could not fetch ${result().url}: ${
                            result().fetch.error ?? "unknown error"
                          }`}
                      <Show when={result().fetch.note}>
                        {(note) => (
                          <>
                            <br />
                            {note()}
                          </>
                        )}
                      </Show>
                    </li>
                    <For each={result().install.findings}>
                      {(item) => (
                        <li class={`${finding} ${findingClass(item.severity)}`}>{item.message}</li>
                      )}
                    </For>

                    <Show when={ingestHit()}>
                      {(hit) => {
                        const where =
                          hit().hostname || hit().path
                            ? ` (${hit().hostname ?? ""}${hit().path ?? ""})`
                            : "";
                        return (
                          <li class={`${finding} ${findingOk}`}>
                            Event successfully detected at{" "}
                            {formatDetectedAt(hit().detected_at_ms ?? Date.now())}
                            {where}.
                          </li>
                        );
                      }}
                    </Show>

                    <Show when={!ingestHit() && waitingForIngest()}>
                      <li class={`${finding} ${findingWaiting}`}>
                        Waiting for an event from your site… Open{" "}
                        <a class={docsLink} href={result().url} target="_blank" rel="noreferrer">
                          {result().url}
                        </a>{" "}
                        in another tab (or refresh it). The tracker usually sends within about 30
                        seconds.
                      </li>
                    </Show>

                    <Show when={!ingestHit() && ingestTimedOut()}>
                      <li class={`${finding} ${findingError}`}>
                        No ingest event arrived within 2 minutes. Confirm the tracker is installed,
                        you are not blocking the request, and try again after visiting the page.
                      </li>
                    </Show>

                    <Show when={pollError()}>
                      <li class={`${finding} ${findingError}`}>{pollError()}</li>
                    </Show>
                  </ul>
                  <p class={followUp}>
                    Need the snippet again? See{" "}
                    <a class={docsLink} href={docsUrl("/docs/getting-started")}>
                      Getting started
                    </a>
                    .
                  </p>
                </>
              )}
            </Show>
          </section>

          <section class={section}>
            <h2 class={sectionTitle}>Integrations</h2>
            <p class={sectionDescription}>
              Configured payment and search integrations for this project, with the latest sync
              result.
            </p>
            <Show
              when={loading()}
              fallback={
                <Show
                  when={integrations().length > 0}
                  fallback={
                    <p class={emptyState}>
                      No integrations configured yet. Connect one in{" "}
                      <a class={docsLink} href={appPath("/settings")}>
                        Settings
                      </a>
                      .
                    </p>
                  }
                >
                  <div class={tableScroll}>
                    <table class={table}>
                      <thead>
                        <tr>
                          <th>Integration</th>
                          <th>Status</th>
                          <th>Last success</th>
                          <th>Latest error</th>
                        </tr>
                      </thead>
                      <tbody>
                        <For each={integrations()}>
                          {(integration) => (
                            <tr>
                              <td>{integrationTypeLabel(integration.integration_type)}</td>
                              <td>
                                <span
                                  class={`${badge} ${statusBadgeClass(integration.status) ?? ""}`}
                                >
                                  {integrationStatusLabel(integration.status)}
                                </span>
                              </td>
                              <td class={mutedCell}>{formatSyncAt(integration.last_synced_at)}</td>
                              <td class={mutedCell}>{integration.last_sync_error ?? "—"}</td>
                            </tr>
                          )}
                        </For>
                      </tbody>
                    </table>
                  </div>
                </Show>
              }
            >
              <p class={message}>Loading…</p>
            </Show>
          </section>
        </Show>
      </Show>
    </div>
  );
}
