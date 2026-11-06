import { getApiBaseUrl } from "@revscope/web-shared/apiBaseUrl";
import { projectApiPath } from "@revscope/web-shared/apiPaths";
import { authRequestInit, readJson } from "@revscope/web-shared/apiRequest";
import type { AuthSession } from "@revscope/web-shared/authState";

/** Matches the `Content-Disposition` name the API sends. */
export function projectDatabaseFileName(projectId: string): string {
  return `revscope-project-${projectId}.db`;
}

export type DownloadDbToken = {
  token: string;
  /** Roughly what the download will transfer. */
  size_bytes: number;
  /** Seconds since the unix epoch. */
  expires_at: number;
};

/**
 * Trades the session for a short-lived token that grants nothing but
 * downloading this project's database, and reports how big that download is.
 */
export async function createDownloadToken(
  session: AuthSession,
  projectId: string,
): Promise<DownloadDbToken> {
  const url = new URL(projectApiPath("session", projectId, "download-token"), getApiBaseUrl());
  const response = await fetch(url, authRequestInit(session, { method: "POST" }));
  return readJson<DownloadDbToken>(response, "Unable to prepare the download.");
}

/**
 * Link the browser downloads the database from. Navigating to it keeps the
 * transfer in the browser's hands, so it lands in the downloads folder, shows
 * progress, and can be resumed. That rules out an `Authorization` header, so
 * the token travels in the query string; the API redacts it from its logs.
 * Every project needs one, the public demo included.
 */
export function projectDatabaseDownloadUrl(projectId: string, token: string): string {
  const url = new URL(projectApiPath("download", projectId, ""), getApiBaseUrl());
  url.searchParams.set("token", token);
  return url.toString();
}

const SIZE_UNITS = ["bytes", "kb", "mb", "gb", "tb"];

/** Human size to one decimal place, e.g. `340.2 mb`. */
export function formatFileSize(bytes: number): string {
  let size = Math.max(bytes, 0);
  let unit = 0;
  while (size >= 1024 && unit < SIZE_UNITS.length - 1) {
    size /= 1024;
    unit += 1;
  }
  return unit === 0 ? `${Math.round(size)} bytes` : `${size.toFixed(1)} ${SIZE_UNITS[unit]}`;
}
