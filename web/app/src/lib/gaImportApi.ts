import { getApiBaseUrl } from "@revscope/web-shared/apiBaseUrl";
import { projectApiPath } from "@revscope/web-shared/apiPaths";
import { authRequestInit, fetchJson } from "@revscope/web-shared/apiRequest";
import type { AuthSession } from "@revscope/web-shared/authState";

export type ImportStatus = "running" | "paused" | "completed" | "cancelled" | "failed";

export type GaImport = {
  id: number;
  property_name: string;
  start_date: string;
  end_date: string;
  hostnames: string[];
  status: ImportStatus;
  chunks_done: number;
  chunks_total: number;
  resume_at: string;
  error: string | null;
  sessions_imported: number;
};

export type GaImportsResponse = {
  /** Whether "Connect with Google" can work on this server. */
  oauth_available: boolean;
  /** Oldest start the plan allows; null when it keeps everything. */
  oldest_allowed_date: string | null;
  latest_allowed_date: string;
  default_start_date: string;
  default_end_date: string;
  /** An end date on or after this duplicates visits and pageviews. Null without tracking. */
  first_tracked_date: string | null;
  include_in_reports: boolean;
  imports: GaImport[];
};

export type GaProperty = {
  id: string;
  name: string;
  account: string;
};

export type GaConnection = {
  connection_id: string;
  properties: GaProperty[];
  known_hostnames: string[];
};

export type StartImportPayload = {
  connection_id: string;
  property_id: string;
  start_date: string;
  end_date: string;
  hostnames: string[];
  allow_duplicates: boolean;
};

function importsUrl(projectId: string, rest = ""): URL {
  return new URL(projectApiPath("session", projectId, `ga-imports${rest}`), getApiBaseUrl());
}

function jsonInit(session: AuthSession, method: string, body?: unknown): RequestInit {
  return authRequestInit(session, {
    method,
    headers: { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
}

export function fetchGaImports(
  session: AuthSession,
  projectId: string,
): Promise<GaImportsResponse> {
  return fetchJson<GaImportsResponse>(
    importsUrl(projectId),
    authRequestInit(session),
    "Unable to load Google Analytics imports.",
  );
}

export async function fetchGaOauthUrl(session: AuthSession, projectId: string): Promise<string> {
  const response = await fetchJson<{ url: string }>(
    importsUrl(projectId, "/oauth-url"),
    jsonInit(session, "POST", {}),
    "Unable to start connecting to Google.",
  );
  return response.url;
}

export function connectServiceAccount(
  session: AuthSession,
  projectId: string,
  serviceAccountJson: string,
): Promise<GaConnection> {
  return fetchJson<GaConnection>(
    importsUrl(projectId, "/connect"),
    jsonInit(session, "POST", { service_account_json: serviceAccountJson }),
    "Unable to connect that service account.",
  );
}

export function connectOauth(
  session: AuthSession,
  projectId: string,
  code: string,
  state: string,
): Promise<GaConnection> {
  return fetchJson<GaConnection>(
    importsUrl(projectId, "/connect"),
    jsonInit(session, "POST", { oauth_code: code, oauth_state: state }),
    "Unable to finish connecting to Google.",
  );
}

export function startGaImport(
  session: AuthSession,
  projectId: string,
  payload: StartImportPayload,
): Promise<GaImport> {
  return fetchJson<GaImport>(
    importsUrl(projectId),
    jsonInit(session, "POST", payload),
    "Unable to start the import.",
  );
}

export function cancelGaImport(
  session: AuthSession,
  projectId: string,
  importId: number,
): Promise<GaImport> {
  return fetchJson<GaImport>(
    importsUrl(projectId, `/cancel?id=${importId}`),
    jsonInit(session, "POST", {}),
    "Unable to cancel the import.",
  );
}

export function deleteGaImport(
  session: AuthSession,
  projectId: string,
  importId: number,
): Promise<{ id: number }> {
  return fetchJson<{ id: number }>(
    importsUrl(projectId, `?id=${importId}`),
    jsonInit(session, "DELETE"),
    "Unable to delete the import.",
  );
}

export function saveImportedDataSetting(
  session: AuthSession,
  projectId: string,
  includeInReports: boolean,
): Promise<{ include_in_reports: boolean }> {
  return fetchJson<{ include_in_reports: boolean }>(
    importsUrl(projectId, "/settings"),
    jsonInit(session, "POST", { include_in_reports: includeInReports }),
    "Unable to save that setting.",
  );
}

/**
 * Carries a connection across the redirect from Google's consent screen, from
 * the callback page to the Settings section it returns to. Only the id of a
 * server-side connection is in here, never a credential.
 */
const PENDING_CONNECTION_KEY = "revscope.gaImportConnection";

export function storePendingConnection(projectId: string, connection: GaConnection): void {
  sessionStorage.setItem(PENDING_CONNECTION_KEY, JSON.stringify({ projectId, connection }));
}

export function takePendingConnection(projectId: string): GaConnection | null {
  const stored = sessionStorage.getItem(PENDING_CONNECTION_KEY);
  if (!stored) {
    return null;
  }
  sessionStorage.removeItem(PENDING_CONNECTION_KEY);
  try {
    const parsed = JSON.parse(stored) as { projectId: string; connection: GaConnection };
    return parsed.projectId === projectId ? parsed.connection : null;
  } catch {
    return null;
  }
}

/** The project a consent redirect belongs to: the server starts its state with the id. */
export function projectIdFromOauthState(state: string): string | null {
  const [projectId] = state.split(".");
  return projectId && /^\d+$/.test(projectId) ? projectId : null;
}

/** Whether a range would count days both Google Analytics and our tracker saw. */
export function overlapsTracking(endDate: string, firstTrackedDate: string | null): boolean {
  return firstTrackedDate !== null && endDate >= firstTrackedDate;
}
