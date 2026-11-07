import { getApiBaseUrl } from "@revscope/web-shared/apiBaseUrl";
import { projectApiPath } from "@revscope/web-shared/apiPaths";
import { authRequestInit, readJson } from "@revscope/web-shared/apiRequest";
import type { AuthSession } from "@revscope/web-shared/authState";
import { INTEGRATION_META, type IntegrationType } from "./settingsApi";

export type IntegrationStatus = {
  integration_type: string;
  last_synced_at: string | null;
  last_sync_error: string | null;
  status: "never_run" | "ok" | "error" | string;
};

export type TroubleshootingStatusResponse = {
  project_name: string;
  default_url: string;
  events_last_month: number;
  /** Received but not yet written; the server saves each project's batch every 5 minutes. */
  events_queued: number;
  integrations: IntegrationStatus[];
};

export type InstallFinding = {
  severity: "ok" | "warning" | "error" | string;
  message: string;
};

export type TroubleshootingCheckResponse = {
  url: string;
  fetch: {
    ok: boolean;
    status_code: number | null;
    error: string | null;
    note: string | null;
  };
  install: {
    ok: boolean;
    findings: InstallFinding[];
  };
  /** Server clock; poll ingest for hits at or after this. */
  wait_started_at_ms: number;
};

export type TroubleshootingIngestResponse = {
  detected: boolean;
  detected_at_ms: number | null;
  path: string | null;
  hostname: string | null;
};

function networkErrorMessage(caught: unknown, fallback: string): Error {
  if (caught instanceof TypeError) {
    return new Error(`Unable to reach the API at ${getApiBaseUrl()}. Is the backend running?`);
  }
  if (caught instanceof Error) {
    return caught;
  }
  return new Error(fallback);
}

export async function fetchTroubleshootingStatus(
  session: AuthSession,
  projectId: string,
): Promise<TroubleshootingStatusResponse> {
  const url = new URL(projectApiPath("session", projectId, "troubleshooting"), getApiBaseUrl());
  try {
    const response = await fetch(url, authRequestInit(session));
    return await readJson(response, "Unable to load troubleshooting status.");
  } catch (caught) {
    throw networkErrorMessage(caught, "Unable to load troubleshooting status.");
  }
}

export async function runTroubleshootingCheck(
  session: AuthSession,
  projectId: string,
  checkUrl: string,
): Promise<TroubleshootingCheckResponse> {
  const url = new URL(
    projectApiPath("session", projectId, "troubleshooting/check"),
    getApiBaseUrl(),
  );
  try {
    const response = await fetch(
      url,
      authRequestInit(session, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ url: checkUrl }),
      }),
    );
    return await readJson(response, "Unable to run installation check.");
  } catch (caught) {
    throw networkErrorMessage(caught, "Unable to run installation check.");
  }
}

export async function pollTroubleshootingIngest(
  session: AuthSession,
  projectId: string,
  sinceMs: number,
): Promise<TroubleshootingIngestResponse> {
  const url = new URL(
    projectApiPath("session", projectId, "troubleshooting/ingest"),
    getApiBaseUrl(),
  );
  url.searchParams.set("since_ms", String(sinceMs));
  try {
    const response = await fetch(url, authRequestInit(session));
    return await readJson(response, "Unable to poll ingest.");
  } catch (caught) {
    throw networkErrorMessage(caught, "Unable to poll ingest.");
  }
}

export function integrationTypeLabel(type: string): string {
  const meta = INTEGRATION_META[type as IntegrationType];
  return meta?.label ?? type.replace(/_/g, " ");
}

export function integrationStatusLabel(status: string): string {
  if (status === "ok") {
    return "Synced";
  }
  if (status === "error") {
    return "Error";
  }
  if (status === "never_run") {
    return "Waiting for first sync";
  }
  return status;
}

export function formatEventsCount(count: number): string {
  return new Intl.NumberFormat().format(count);
}

/** Formats a Postgres timestamptz text value for the integrations table. */
export function formatSyncAt(value: string | null): string {
  if (!value) {
    return "—";
  }
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }
  return date.toLocaleString(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  });
}

export function formatDetectedAt(epochMs: number): string {
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "medium",
  }).format(new Date(epochMs));
}
