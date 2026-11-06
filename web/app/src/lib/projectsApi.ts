import { getApiBaseUrl } from "@revscope/web-shared/apiBaseUrl";
import { projectApiPath, sessionApiPath } from "@revscope/web-shared/apiPaths";
import { authRequestInit, fetchJson } from "@revscope/web-shared/apiRequest";
import type { AuthSession } from "@revscope/web-shared/authState";

export type CreatedProject = { id: string; name: string };

/** Adds a project named after `website`; the timezone is the new project's default. */
export async function createProject(
  session: AuthSession,
  website: string,
  timezone: string,
): Promise<CreatedProject> {
  const url = new URL(sessionApiPath("projects"), getApiBaseUrl());
  return fetchJson<CreatedProject>(
    url,
    authRequestInit(session, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ website, timezone }),
    }),
    "Unable to add the project.",
  );
}

/** Permanently deletes a project. The server refuses unless `confirmName` is its name. */
export async function deleteProject(
  session: AuthSession,
  projectId: string,
  confirmName: string,
): Promise<void> {
  const url = new URL(projectApiPath("session", projectId, ""), getApiBaseUrl());
  await fetchJson<unknown>(
    url,
    authRequestInit(session, {
      method: "DELETE",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ confirm_name: confirmName }),
    }),
    "Unable to delete the project.",
  );
}
