import { getApiBaseUrl } from "@revscope/web-shared/apiBaseUrl";
import { projectApiPath } from "@revscope/web-shared/apiPaths";
import { authRequestInit, fetchJson } from "@revscope/web-shared/apiRequest";
import type { AuthSession } from "@revscope/web-shared/authState";

export type ImportedRange = {
  start_date: string;
  end_date: string;
  source_name: string;
};

export type ImportedCoverage = {
  include_in_reports: boolean;
  /** Imported rows exist for days in the requested range, whatever the setting. */
  has_data: boolean;
  /** Custom events are among those rows; a property may not send any. */
  has_events: boolean;
  ranges: ImportedRange[];
};

export function fetchImportedCoverage(
  session: AuthSession,
  projectId: string,
  start: number,
  end: number,
): Promise<ImportedCoverage> {
  const url = new URL(projectApiPath("read", projectId, "imported-coverage"), getApiBaseUrl());
  url.searchParams.set("start", String(start));
  url.searchParams.set("end", String(end));
  return fetchJson<ImportedCoverage>(
    url,
    authRequestInit(session),
    "Unable to check for imported data.",
  );
}
