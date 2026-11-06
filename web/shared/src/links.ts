/**
 * True in the build bundled into a self-hosted backend (`pnpm build:bundle`),
 * which has no marketing site, billing, or docs of its own.
 */
export const SELF_HOSTED = import.meta.env.PUBLIC_SELF_HOSTED === "true";

/**
 * A product app page (`/dashboard`, `/settings`, ...) under the base the app was
 * built with: `/` on revscope.co, `/ui` when bundled into the backend.
 */
export function appPath(path: string): string {
  return `${import.meta.env.BASE_URL.replace(/\/$/, "")}${path}`;
}

/** A docs page (`/docs/...`). Self-hosted instances link to revscope.co's. */
export function docsUrl(path: string): string {
  return SELF_HOSTED ? `https://revscope.co${path}` : path;
}

/** The signup page: the marketing site's on revscope.co, the app's own when self-hosted. */
export function signupPath(): string {
  return SELF_HOSTED ? appPath("/signup") : "/signup";
}
