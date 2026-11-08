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

/**
 * The signup page, carrying what the login dialog knew. `next` is the page
 * signup returns to, so an OAuth consent page that sent someone to sign up
 * gets them back with its request intact.
 */
export function signupHref(email: string, next?: string): string {
  const query = new URLSearchParams();
  if (email) {
    query.set("email", email);
  }
  if (next) {
    query.set("next", next);
  }
  const search = query.toString();
  return search ? `${signupPath()}?${search}` : signupPath();
}

/**
 * The `next` page from a query string, if it's a path on this site. Anything
 * else would let a link send a new account's first page anywhere.
 */
export function returnPathFrom(search: string): string | null {
  const next = new URLSearchParams(search).get("next");
  if (!next?.startsWith("/") || typeof window === "undefined") {
    return null;
  }
  // Callers run this after signing someone in, so a `next` that can't be parsed,
  // such as `//%`, has to fall back to the default page rather than throw.
  let url: URL;
  try {
    url = new URL(next, window.location.origin);
  } catch {
    return null;
  }
  // Resolving dot segments can leave a path such as `//evil.example` (from
  // `/.//evil.example`), which the browser reads as another host.
  if (url.origin !== window.location.origin || url.pathname.startsWith("//")) {
    return null;
  }
  return `${url.pathname}${url.search}${url.hash}`;
}
