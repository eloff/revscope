import { SERVER_HOST } from "astro:env/client";

const configuredServerHost = SERVER_HOST?.trim();

/** API origin. An empty `SERVER_HOST` means the API is served alongside the page. */
export function getApiBaseUrl(): string {
  if (!configuredServerHost) {
    return typeof window === "undefined" ? "http://localhost" : window.location.origin;
  }
  return configuredServerHost.includes("://")
    ? configuredServerHost
    : `http://${configuredServerHost}`;
}
