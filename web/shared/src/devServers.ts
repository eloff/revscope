/**
 * In production one origin serves both workers, so the site and the app link to
 * each other with bare paths. In dev they are separate servers, and these
 * redirect a navigation that lands on the wrong one.
 *
 * Loaded by astro.config.mjs, where `import.meta.env` isn't populated.
 */

export const SITE_DEV_PORT = 4321;
export const APP_DEV_PORT = 4322;
export const SITE_DEV_ORIGIN = `http://localhost:${SITE_DEV_PORT}`;
export const APP_DEV_ORIGIN = `http://localhost:${APP_DEV_PORT}`;

/** Where the app's Worker build puts its assets, so its route reaches the app rather than the site. */
export const APP_ASSETS_DIR = "_app";

/** The app worker's paths, as `web/app/wrangler.jsonc` routes them (a test keeps them equal). */
export const APP_PATH_PREFIXES = [
  "/dashboard",
  "/demo",
  "/embed",
  "/settings",
  "/troubleshooting",
  "/reset-password",
  "/oauth/",
  `/${APP_ASSETS_DIR}/`,
];

/** Matches the way Cloudflare matches the app's `revscope.co/<prefix>*` route patterns. */
export function isAppPath(pathname: string): boolean {
  return APP_PATH_PREFIXES.some((prefix) => pathname.startsWith(prefix));
}

interface DevRequest {
  method?: string;
  url?: string;
  headers: Record<string, string | string[] | undefined>;
}

interface DevResponse {
  statusCode: number;
  setHeader(name: string, value: string): void;
  end(): void;
}

/** The slice of a Connect middleware this plugin uses, so shared needn't depend on Vite. */
export type DevMiddleware = (req: DevRequest, res: DevResponse, next: () => void) => void;

/**
 * A Vite plugin redirecting browser page loads whose path `belongsElsewhere` to
 * the same path on `origin`. Only `sec-fetch-mode: navigate` requests qualify,
 * so the dev server's module, asset and HMR fetches pass through; opening one
 * of those URLs directly in a tab is redirected like any other page.
 * `configureServer` only runs under `astro dev`, so builds are unaffected.
 */
export function devRedirectPlugin(belongsElsewhere: (pathname: string) => boolean, origin: string) {
  return {
    name: "revscope-dev-redirect",
    configureServer(server: { middlewares: { use(handler: DevMiddleware): void } }) {
      server.middlewares.use((req, res, next) => {
        const url = req.url ?? "/";
        const pathname = url.split("?")[0];
        if (
          req.method === "GET" &&
          req.headers["sec-fetch-mode"] === "navigate" &&
          belongsElsewhere(pathname)
        ) {
          res.statusCode = 307;
          res.setHeader("Location", `${origin}${url}`);
          res.end();
          return;
        }
        next();
      });
    },
  };
}

/** Sends page loads for site paths from the app's dev server to the site's. */
export function appDevRedirectPlugin() {
  return devRedirectPlugin((pathname) => !isAppPath(pathname), SITE_DEV_ORIGIN);
}

/** Sends page loads for app paths from the site's dev server to the app's. */
export function siteDevRedirectPlugin() {
  return devRedirectPlugin(isAppPath, APP_DEV_ORIGIN);
}
