// @ts-check

import cloudflare from "@astrojs/cloudflare";
import solid from "@astrojs/solid-js";
import { vanillaExtractPlugin } from "@vanilla-extract/vite-plugin";
import { defineConfig, envField } from "astro/config";
import { APP_ASSETS_DIR, APP_DEV_PORT, appDevRedirectPlugin } from "../shared/src/devServers.ts";

// `pnpm build:bundle` sets this: a static build under /ui that a self-hosted
// backend embeds and serves itself (cargo feature `self-hosted`), instead of
// the Worker. It builds in mode `bundle`, so `.env.production` is not read; put
// overrides in `.env.bundle` instead.
const bundle = process.env.REVSCOPE_BUNDLE === "1";

/** Pages only a self-hosted instance has, since revscope.co's live on the marketing site. */
const selfHostedPages = {
  name: "revscope-self-hosted-pages",
  hooks: {
    /** @param {{ injectRoute: (route: { pattern: string; entrypoint: string }) => void }} options */
    "astro:config:setup": ({ injectRoute }) => {
      injectRoute({ pattern: "/signup", entrypoint: "./src/self-hosted/signup.astro" });
    },
  },
};

// https://astro.build/config
export default defineConfig({
  compressHTML: true,

  // Astro overrides vite.server.port with this, so the port must be set here.
  server: { port: APP_DEV_PORT },

  base: bundle ? "/ui" : undefined,
  outDir: bundle ? "./dist-ui" : undefined,
  // On revscope.co the site worker's catch-all route would answer the default
  // /_astro/, and it doesn't have the app's files. APP_PATH_PREFIXES, which a
  // test keeps equal to wrangler.jsonc's routes, includes this directory. The self-hosted backend
  // expects _astro/ (revscope/src/web_ui.rs), and has no site to collide with.
  build: bundle ? undefined : { assets: APP_ASSETS_DIR },

  env: {
    schema: {
      // Where the API lives, read at build time. Empty means the page's own
      // origin, which is how the bundled build is served.
      // Use `localhost` rather than `127.0.0.1` so the dev server and the API
      // share a site and the browser sends the SameSite=Strict session cookie.
      SERVER_HOST: envField.string({
        context: "client",
        access: "public",
        default: bundle ? "" : "localhost:4320",
      }),
    },
  },

  integrations: bundle ? [solid(), selfHostedPages] : [solid()],

  vite: {
    plugins: bundle ? [vanillaExtractPlugin()] : [vanillaExtractPlugin(), appDevRedirectPlugin()],
    define: {
      // Read by `SELF_HOSTED` in @revscope/web-shared/links.
      "import.meta.env.PUBLIC_SELF_HOSTED": JSON.stringify(bundle ? "true" : "false"),
    },
    server: {
      // Fail rather than drift onto another port: the site's dev server
      // iframes /embed from this exact origin.
      strictPort: true,
    },
    ssr: {
      noExternal: ["@revscope/web-shared"],
    },
    optimizeDeps: {
      exclude: ["jsvectormap", "@revscope/web-shared"],
    },
    resolve: {
      dedupe: ["solid-js", "@ark-ui/solid", "@vanilla-extract/css"],
    },
  },

  adapter: bundle ? undefined : cloudflare(),
});
