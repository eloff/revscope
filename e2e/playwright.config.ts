import { defineConfig, devices } from "@playwright/test";

const baseURL = (process.env.E2E_BASE_URL ?? "http://localhost:4322").replace(/\/?$/, "/");
const isLocal = /^https?:\/\/(localhost|127\.0\.0\.1)[:/]/.test(baseURL);
const isCI = Boolean(process.env.CI);

export default defineConfig({
  testDir: "./tests",
  fullyParallel: true,
  forbidOnly: isCI,
  // Production's nightly demo refresh can swap the database under a run, and CI runners are
  // slow and uneven. On a developer's machine a flake should show.
  retries: !isLocal ? 2 : isCI ? 1 : 0,
  // Dashboard queries slow sharply under concurrency, past test timeouts at one worker per
  // core, and production has real visitors to serve.
  workers: isLocal && !isCI ? 4 : 2,
  timeout: 60_000,
  expect: { timeout: 20_000 },
  reporter: isCI ? [["github"], ["html", { open: "never" }]] : [["list"]],
  use: {
    baseURL,
    // Parsers in tests/support read en-US numbers and dates.
    locale: "en-US",
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"], viewport: { width: 1440, height: 900 } },
    },
  ],
});
