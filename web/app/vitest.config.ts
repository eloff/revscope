import { fileURLToPath } from "node:url";
import { vanillaExtractPlugin } from "@vanilla-extract/vite-plugin";
import solid from "vite-plugin-solid";
import { configDefaults, defineConfig } from "vitest/config";

// Astro's own Vite config cannot be reused here: the Cloudflare adapter rejects
// the options Vitest sets. So this rebuilds the parts tests need: Solid's JSX
// transform, vanilla-extract styles, and Astro's virtual env module.
//
// Tests default to jsdom, logic tests included: the Solid plugin resolves
// solid-js to its browser build, so modules that touch `localStorage` at import
// (projectSelection.ts) can't load under node. A file that needs node, as
// astroConfig.test.ts does for esbuild, opts in and setup.ts skips its DOM shims.
export default defineConfig({
  plugins: [solid(), vanillaExtractPlugin()],
  resolve: {
    alias: {
      "astro:env/client": fileURLToPath(new URL("./test/astroEnvClient.ts", import.meta.url)),
    },
    dedupe: ["solid-js", "@ark-ui/solid", "@vanilla-extract/css"],
  },
  define: {
    "import.meta.env.PUBLIC_SELF_HOSTED": JSON.stringify("false"),
  },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.{ts,tsx}", "../shared/src/**/*.test.{ts,tsx}"],
    setupFiles: ["./test/setup.ts"],
    // astroConfig.test.ts hides its import of this from Vite's dependency scan, which hides it
    // from Vitest's module graph too, so a config change wouldn't otherwise rerun that test.
    forceRerunTriggers: [...configDefaults.forceRerunTriggers, "**/astro.config.mjs"],
  },
});
