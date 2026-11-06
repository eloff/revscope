/** Auth-policy prefixes for the v2 API (`/v2/{policy}/…`). */
export type ApiAuthPolicy = "public" | "session" | "read" | "query" | "download" | "chat";

/** Account-scoped session routes (no project id in the path). */
export function sessionApiPath(resource: string): string {
  return `/v2/session/${resource}`;
}

/** Public routes under `/v2/public/…`. */
export function publicApiPath(resource: string): string {
  return `/v2/public/${resource}`;
}

/** Project-scoped route: `/v2/{policy}/projects/{id}/{resource}`. */
export function projectApiPath(
  policy: Exclude<ApiAuthPolicy, "public">,
  projectId: string,
  resource: string,
): string {
  const path = resource ? `/${resource}` : "";
  return `/v2/${policy}/projects/${encodeURIComponent(projectId)}${path}`;
}
