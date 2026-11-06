// @vitest-environment node
// The Cloudflare adapter loads esbuild, which refuses to run under jsdom.
import { afterEach, describe, expect, it, vi } from "vitest";
import { APP_ASSETS_DIR, isAppPath } from "../../shared/src/devServers";

async function loadConfig(bundle: boolean) {
  vi.stubEnv("REVSCOPE_BUNDLE", bundle ? "1" : "");
  vi.resetModules();
  // Computed so Vite's dependency scan, which crawls src/ including tests, can't follow it: the
  // config reaches wrangler's blake3-wasm, whose ESM entry can't be bundled, and `astro dev` dies.
  const configUrl = new URL("../astro.config.mjs", import.meta.url).href;
  return (await import(/* @vite-ignore */ configUrl)).default;
}

afterEach(() => {
  vi.unstubAllEnvs();
});

describe("astro.config build.assets", () => {
  it("puts the Worker build's assets under a path the app worker routes", async () => {
    const assets = (await loadConfig(false)).build?.assets;
    expect(assets).toBe(APP_ASSETS_DIR);
    expect(isAppPath(`/${assets}/client.abc123.js`)).toBe(true);
  });

  it("keeps the self-hosted bundle on _astro, which revscope/src/web_ui.rs caches as immutable", async () => {
    expect((await loadConfig(true)).build?.assets ?? "_astro").toBe("_astro");
  });
});
