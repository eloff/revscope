## Workflow

Do each task in its own worktree. `scripts/worktree-new.sh <branch>` creates `../revscope-<branch>` on a new branch from origin/main, with the web dependencies installed and your local `.env` files linked in.

When a task is done, make `scripts/check.sh` pass and commit with `git commit -s` (CI rejects commits without sign-off). It runs the checks in CONTRIBUTING.md for each part you changed, except the `e2e/` tests, which need a running target. Then run the harden skill (`.agents/skills/harden/SKILL.md`), committing with `-s` after each round once the checks pass. Open a pull request as described in CONTRIBUTING.md.

## Code

Follow the code conventions in CONTRIBUTING.md. Also:

- Reuse and extend existing helpers, especially formatting and conversion utilities, so each piece of logic has one copy.
- Comment only what the code can't say: the reason for a choice (a constraint, trade-off, or rejected alternative), context that lives elsewhere, or why something odd is deliberate. Keep it to one or two sentences. Fix an unclear name or tangled code by renaming or refactoring it.

## revscope/

Auth lives in the router. `src/routes/table.rs` gives every route and method an `AuthPolicy`, and `src/routes/guard.rs` checks it and passes the resulting grant to the handler. Grants are constructed only in `src/authorize.rs`. Handlers pass their grant, not a raw session or project id, to the domain function. Changes to a project's settings or saved definitions take `WritableProject`, and owner-only operations take `OwnedProject`.

revscope.co's billing is a private, gitignored module at `src/billing/`, compiled in with `--features billing`. Public code reaches it only behind `#[cfg(feature = "billing")]` and must build, lint and test without it. Tests in `tests/` that only `include!` a file from `src/billing/tests/` compile to nothing without the feature.

## crates/revscope-api/

The HTTP contract shared by the server and the CLI: request and response types, route paths, the integration catalog, the tracker snippet, and limits. Change a route's body or path here, never in a copy. Keep its dependencies to serde and its JSON and URL formats. Released CLIs talk to newer servers, so response fields are only ever added, and every enum in a response has a `#[serde(other)] Unknown` variant.

It's a path dependency rather than a Cargo workspace because the server and CLI binaries are both named `revscope` and would overwrite each other in a shared target directory.

## cli/

Calls only the HTTP API, through the `revscope-api` types. Each command returns a type that implements `Render`, so `--json` and text agree, and fails with a `CliError`, whose `exit()` is the documented exit status. Exit statuses and `--json` shapes are a public interface.

## web/

Tests sit beside their source as `*.test.ts(x)` and run under jsdom with `@solidjs/testing-library`. Mock the API with `mockApi` from `web/shared/test/api.ts`; a request no handler answers fails the test. jsdom can't lay out charts, so swap ApexCharts for `web/app/test/apexchartsMock.ts` with `vi.mock("./apexcharts", …)` and assert on the options it records.

## e2e/

The tests run against production, so they only read. `tests/support/demo.ts` fails the test on any write the browser sends, but it can't see `page.request` or `context.request`, so drive everything through the page. Leave Ask AI unsubmitted, since each question spends money.
