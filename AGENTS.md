### Workflow

When a task is done, make tests and lints pass and commit with `git commit -s`. CI rejects pull requests with any commit that lacks its author's sign-off, which is how contributors agree to the terms in CONTRIBUTING.md. Then run the harden skill (`.agents/skills/harden/SKILL.md`), committing (also with `-s`) after each round with tests and lints passing. Open a pull request as described in CONTRIBUTING.md.

### Comments

Nothing checks a comment against the code, so a stale one misleads every later reader. Comment only what the code can't say: why it is written this way (a constraint, trade-off, or rejected alternative), context that lives elsewhere (an external system's behavior, an ordering requirement, a gotcha in another file), or why something that looks wrong is deliberate. Never restate the code, rename rather than explain a poor name, and refactor rather than explain misleading or overly complex code. Prefer one or two sentences.

## Code structure

Each backend route implementation should be its own file. If a component or class anywhere is used from more than 1 file or larger than 100 lines of code, it should be in it's own file. Look for existing code that does the same or similar thing, we don't want multiple copies of the same logic. Refactor to use a shared function where possible and compose more specific behavior from more general re-usable functions. Especially utility functions that format or convert things are better to share than duplicate.

## tracker/

tracker.js is the script user's embed to use our analytics service on their pages. Keep tracker.js as a single file.

It targets ES5 JavaScript.

To verify changes, run the linter `pnpm check` and the tests `pnpm test`.

## revscope/

A rust project that implements the analytics backend.

We don't use an http framework, we have a lightweight tokio server with helpers in http.rs.

API routes are served at `/v2/{prefix}/…` (`public`, `session`, `read`, `query`, `download`, `chat`, e.g. `/v2/read/projects/{id}/…`). In production this is served on the api subdomain. Each route should have it's own file.

Auth is centralized in the router. `src/routes/table.rs` declares an `AuthPolicy` for every route and method, and each policy variant only accepts a handler whose signature takes the grant that policy produces. `src/routes/guard.rs` runs the check and passes the grant to the handler. Grants (`Account`, `AuthorizedProject`, `OwnedProject`, `WritableProject`, `DownloadableProject`, `ChatGrant`) can only be constructed in `src/authorize.rs`. Handlers pass their grant to the domain function, not a raw session or project id. Functions that change a project's settings or saved definitions take `WritableProject`. Functions only the owner may call take `OwnedProject`. Handlers must not authorize on their own.

Database connections are established via db/project.rs (sqlite) or db/main.rs (Postgresql) for project or main databases respectively, with migrations in .sql files in the migrations dir and are applied automatically when a connection is made. Don't modify migrations in-place if
they may have been applied to prod.

Global state like caches goes on the ServerContext so we can isolate and mock for tests.

Use FxHashMap over HashMap and FxHashSet over HashSet.

revscope.co's own billing (plan catalog, checkout, plan changes, Polar and Stripe webhooks) is a private module at `revscope/src/billing/`, gitignored here like web/site, and compiled in with `--features billing`. Public code reaches it only behind `#[cfg(feature = "billing")]`, and must build, lint and test with and without it, run cargo both with no features and with `--features self-hosted`. The e2e tests in `revscope/tests/` that only `include!` a file from `src/billing/tests/` compile to nothing without the feature.

## crates/revscope-api/

The HTTP contract shared by the server and the CLI: request and response types, route paths, the integration catalog, the tracker snippet, and limits. When a route's body or path changes, change it here, not in a copy. It depends only on serde and its JSON and URL formats. Released CLIs talk to newer servers, so never remove or rename a response field, and give any enum in a response a `#[serde(other)] Unknown` variant.

It's a path dependency, not a Cargo workspace, because the server and CLI binaries are both named `revscope` and would overwrite each other in a shared target directory.

## cli/

The `revscope` command line tool, for agents and people. It calls only the HTTP API, through the types in `crates/revscope-api`. Each command returns a type that implements `Render`, so `--json` and text can't disagree, and fails with a `CliError`, whose `exit()` is the documented exit status. Those statuses and the `--json` shapes are a public interface.

To verify changes, from `cli/` run `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and `cargo test`, from `cli/npm/` run `npm test`, and from `revscope/` run `cargo test --test cli_e2e`, which runs the real binary against a real server.

To release, bump the version in `cli/Cargo.toml` and `cli/npm/package.json`, and push a tag `cli-vX.Y.Z`. `.github/workflows/release-cli.yml` does the rest. Server changes it depends on must be deployed first.

## web/

pnpm workspace with two packages:

- `web/app` is the product UI (dashboard, demo, settings, troubleshooting, password reset, OAuth). It deploys as the `revscope-app` worker and owns `/dashboard`, `/demo`, `/embed`, `/settings`, `/troubleshooting`, `/reset-password`, and `/oauth/*`, plus its built assets under `/_app/*`.
- `web/shared` (`@revscope/web-shared`) is the auth and session client. The app depends on it with `workspace:*`.
- `web/site` is a separate repo, .gitignored in the main repo

The app uses Astro, vite, and Solid. We use ark-ui for interactive components, and ApexCharts.js for charts and grids.

Use vanilla-extract with the vite-plugin for managing component scoped css styles - no separate stylesheets. Each major component has it's own file. Vanilla-extract doesn't allow comma-separated selectors like & th, & td, nor does it allow nesting child rules within a parent selector.

To verify, from `web/` run tsc with `pnpm tsc`, the linter with `pnpm check`, and the tests with `pnpm test`. Dev server: `pnpm dev` (port 4322).

Tests sit beside their source as `*.test.ts(x)` and run under jsdom with `@solidjs/testing-library`. Mock the API with `mockApi` from `web/shared/test/api.ts`; a request no handler answers fails the test. jsdom can't lay out charts, so swap ApexCharts for `web/app/test/apexchartsMock.ts` with `vi.mock("./apexcharts", …)` and assert on the options it records.

## e2e/

Playwright tests of the public demo dashboard, against any deployment: `E2E_BASE_URL` picks it (see CONTRIBUTING.md). A standalone package like `tracker/`, so its dependencies stay out of the web app's lockfile and Docker build.

The tests read only, and `tests/support/demo.ts` blocks any write the browser sends and fails the test, so they can run against production. It can't see `page.request` or `context.request`, so don't use those. Never submit Ask AI, which spends money. The demo's data changes, so assert relations, not specific values.

To verify changes, run `pnpm check`, `pnpm tsc`, and `pnpm test` against a stack with generated demo data.
