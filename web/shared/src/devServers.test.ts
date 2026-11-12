import { describe, expect, it, vi } from "vitest";
import wrangler from "../../app/wrangler.jsonc?raw";
import {
  APP_DEV_ORIGIN,
  APP_PATH_PREFIXES,
  appDevRedirectPlugin,
  type DevMiddleware,
  isAppPath,
  SITE_DEV_ORIGIN,
  siteDevRedirectPlugin,
} from "./devServers";

describe("APP_PATH_PREFIXES", () => {
  it("matches the app worker's production routes", () => {
    // Wrangler's JSONC allows full-line comments and trailing commas; JSON doesn't.
    const json = wrangler.replace(/^\s*\/\/.*$/gm, "").replace(/,(\s*[\]}])/g, "$1");
    const config = JSON.parse(json);
    expect(Object.keys(config)).not.toContain("env");
    const prefixes = config.routes.map(({ pattern }: { pattern: string }) => {
      const prefix = /^revscope\.co(\/.*)\*$/.exec(pattern)?.[1];
      if (!prefix) throw new Error(`No dev equivalent for route ${pattern}`);
      return prefix;
    });
    expect(prefixes).toEqual(APP_PATH_PREFIXES);
  });

  // A page left out of the routes is served the site's 404 in production.
  it("covers every page the app builds", () => {
    const pages = Object.keys(import.meta.glob("../../app/src/pages/**/*.astro"));
    expect(pages.length).toBeGreaterThan(0);
    for (const page of pages) {
      const path = page.replace("../../app/src/pages", "").replace(/(\/index)?\.astro$/, "");
      expect(isAppPath(path), path).toBe(true);
    }
  });
});

describe("isAppPath", () => {
  it("matches the app's pages and their subpaths", () => {
    expect(isAppPath("/demo")).toBe(true);
    expect(isAppPath("/embed/runway")).toBe(true);
    expect(isAppPath("/oauth/callback")).toBe(true);
    expect(isAppPath("/_app/client.abc123.js")).toBe(true);
  });

  it("leaves the site's pages alone", () => {
    expect(isAppPath("/")).toBe(false);
    expect(isAppPath("/docs/getting-started")).toBe(false);
    expect(isAppPath("/oauth")).toBe(false);
  });
});

type Plugin = ReturnType<typeof appDevRedirectPlugin>;

function request(plugin: Plugin, url: string, init: { method?: string; mode?: string } = {}) {
  let handler: DevMiddleware | undefined;
  plugin.configureServer({ middlewares: { use: (h) => (handler = h) } });
  const res = { statusCode: 200, setHeader: vi.fn(), end: vi.fn() };
  const next = vi.fn();
  handler?.(
    { method: init.method ?? "GET", url, headers: { "sec-fetch-mode": init.mode ?? "navigate" } },
    res,
    next,
  );
  const location = res.setHeader.mock.calls.find(([name]) => name === "Location")?.[1];
  return { status: res.statusCode, location, passed: next.mock.calls.length > 0 };
}

describe("the app's dev server", () => {
  it("sends site pages to the site, keeping the query", () => {
    expect(request(appDevRedirectPlugin(), "/docs/ai?x=1")).toEqual({
      status: 307,
      location: `${SITE_DEV_ORIGIN}/docs/ai?x=1`,
      passed: false,
    });
  });

  it("serves its own pages", () => {
    expect(request(appDevRedirectPlugin(), "/demo").passed).toBe(true);
  });
});

describe("the site's dev server", () => {
  it("sends app pages to the app, keeping the query", () => {
    expect(request(siteDevRedirectPlugin(), "/demo?tab=ask")).toEqual({
      status: 307,
      location: `${APP_DEV_ORIGIN}/demo?tab=ask`,
      passed: false,
    });
  });

  it("serves its own pages", () => {
    expect(request(siteDevRedirectPlugin(), "/pricing").passed).toBe(true);
  });

  it("leaves module fetches and non-GET requests alone", () => {
    expect(request(siteDevRedirectPlugin(), "/demo", { mode: "cors" }).passed).toBe(true);
    expect(request(siteDevRedirectPlugin(), "/demo", { method: "POST" }).passed).toBe(true);
  });
});

it("never bounces a page between the two servers", () => {
  for (const url of [
    "/",
    "/demo",
    "/embed/runway",
    "/oauth",
    "/oauth/x",
    "/docs",
    "/settings?a=1",
  ]) {
    const app = request(appDevRedirectPlugin(), url);
    const site = request(siteDevRedirectPlugin(), url);
    expect([app.passed, site.passed].filter(Boolean)).toHaveLength(1);
  }
});
