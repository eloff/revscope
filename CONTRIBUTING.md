# Contributing to RevScope

Thanks for your interest in improving RevScope! Bug reports, fixes, docs
improvements, and features are all welcome.

## Before you start

- **Bugs:** open an issue with steps to reproduce, what you expected, and what
  happened. Include versions and logs where relevant (redact any secrets).
- **Features and larger changes:** open an issue to discuss the idea before
  writing a lot of code, so we can agree on the approach first.
- **Small fixes** (typos, docs, obvious bugs) can go straight to a pull request.
- **Security vulnerabilities:** please don't open a public issue. Report them
  privately through GitHub's "Report a vulnerability" button on the Security
  tab.
- **AI tools:** AI-assisted contributions are welcome, but make sure you can
  describe the problem and the solution as if you'd written the code yourself.
  You don't need to know the function names, but you do need enough
  understanding to explain the problem and solution on a whiteboard.

## Development setup

See the [README](README.md) for prerequisites, configuration, and how to run
each part of the project. The short version:

```sh
(cd web && pnpm install)
cd revscope && cargo run
```

Use pnpm, not npm.

The backend's end-to-end tests in `revscope/tests/` need a Postgres database.
Set `DATABASE_URL` in your environment or in `revscope/.e2e.env`, preferably
pointing at a database used only for tests. A worktree from
`scripts/worktree-new.sh` gets its own schema in that database through its own
`.e2e.env`, so test runs in different worktrees don't see each other's data.
An exported `DATABASE_URL` overrides that file, and so that schema.

## Checks

Run the checks for every part you touched before opening a pull request:

| Part        | Commands                                                                                   |
| ----------- | ------------------------------------------------------------------------------------------ |
| `tracker/`  | `pnpm check`, `pnpm test`, `pnpm build` (commit its output)                                 |
| `revscope/` | `cargo fmt`, then `cargo clippy` and `cargo test` with and without `--features self-hosted` |
| `web/`      | `pnpm tsc`, `pnpm check`, `pnpm test`                                                      |
| `e2e/`      | `pnpm tsc`, `pnpm check`, `pnpm test` (see below)                                          |
| `cli/`      | `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, in `cli/npm` `npm test`, and in `revscope/` `cargo test --test cli_e2e` |
| `crates/`   | `cargo fmt`, `cargo clippy`, `cargo test`, then the checks of `revscope/` and `cli/`        |

`pnpm check` uses biome with `--write`, so it may reformat files. Commit those
changes.

## Browser tests

`e2e/` holds Playwright tests that drive the public demo dashboard. They only read: a guard blocks
any write from the browser and fails the test, so they're safe to point at production. `E2E_BASE_URL` picks the
target.

```sh
cd e2e
pnpm install
pnpm exec playwright install chromium

pnpm test                                        # the app at http://localhost:4322
E2E_BASE_URL=http://localhost:4320/ui pnpm test  # a backend built with --features self-hosted
pnpm test:prod                                   # https://revscope.co
```

A local target needs demo data with traffic both in the last 30 days and before them, so
regenerate it once it's a few weeks old. Create or refresh it from `revscope/`, with the backend
stopped:

```sh
cargo run --release --features generate --bin generate -- 1 data --overwrite
```

The data changes, so assert relations (a column is ordered, a filter narrows a total, a wider
range raises it), not specific numbers, names or dates. Don't expect data
in very recent periods such as today, since the demo can be a day old. The tests follow `main`, so
against production, those covering undeployed changes fail until the next deploy.

## Code conventions

Match the style of the surrounding code. A few project-specific rules:

- Move a component or class into its own file once it's used from more than one
  file or grows past about 100 lines.

**Tracker (`tracker/`)**

- `tracker.js` stays a single file and targets ES5: no arrow functions, `let` or
  `const`, spread, destructuring, or other later syntax. `pnpm check` verifies
  that it parses as ES5.
- The backend embeds the built `tracker-min.js*`, which are committed. After
  changing `tracker.js`, run `pnpm build` and commit its output, including the
  version bump and `content-fingerprint`. CI fails if they are stale.

**Backend (`revscope/`)**

- There's no HTTP framework: the server uses tokio with the helpers in
  `http.rs`. Web frameworks add a lot of dependencies and slow down compile
  times for little benefit. Each route goes in its own file.
- Routes live under `/v2/{auth-policy}/…`. The router enforces the auth
  policy, so put new routes under the right prefix.
- Global state such as caches goes on `ServerContext`, so it can be isolated and
  mocked in tests.
- Use `FxHashMap` and `FxHashSet`, not `HashMap` and `HashSet`.
- Schema changes go in a **new** migration file in `migrations/` (per-project
  SQLite) or `migrations-main/` (Postgres). Never edit an existing migration,
  since it may already have been applied to production databases.
  Each Postgres migration runs in a transaction with its record, so it can't manage
  its own transaction or use statements that need to run outside one.

**Web app (`web/`)**

- Astro and Solid, with ark-ui for interactive components and ApexCharts for
  charts.
- Styles use vanilla-extract, scoped to each component, with no separate
  stylesheets. vanilla-extract doesn't support comma-separated selectors or
  nested child rules.

## Releasing the CLI

Deploy any server changes the release depends on first. Then bump the version in
`cli/Cargo.toml` and `cli/npm/package.json` and push a tag `cli-vX.Y.Z`;
`.github/workflows/release-cli.yml` does the rest.

## Pull requests

- Keep each pull request focused on one change. Unrelated cleanups are easier
  to review separately.
- Add or update tests for behavior changes, especially in the backend.
- Update the README or other docs when you change setup steps, configuration,
  or user-facing behavior.
- In the description, explain what the change does and why, and how you tested
  it.
- Sign off every commit (`git commit -s`); see [License](#license).

## Signing commits

This is separate from the sign-off above. A sign-off is a line in the commit
message, while a signature proves who made the commit. `main` requires verified
signatures, so maintainers landing with `scripts/land.sh` need a signing key,
and the script stops before changing anything if git can't sign. To set up an
SSH key:

```sh
ssh-keygen -t ed25519 -f ~/.ssh/git-signing -C "git signing"
ssh-add ~/.ssh/git-signing
git config --global gpg.format ssh
git config --global user.signingkey ~/.ssh/git-signing.pub
gh auth refresh -h github.com -s admin:ssh_signing_key
gh ssh-key add ~/.ssh/git-signing.pub --type signing --title "git signing"
```

GitHub marks a commit as verified only if the committer email is verified on
your account, so check that for each `user.email` you commit with, including
any set for a single repository.

## License

RevScope is distributed under the [Functional Source License, Version 1.1,
ALv2 Future License](LICENSE). Contributions come in under a more permissive
license than that, so the project can keep running the hosted service and
change its license terms later without tracking down every contributor.

By submitting a contribution, you certify the [Developer Certificate of
Origin](https://developercertificate.org), which says you have the right to
submit it, and you license your contribution to Daniel Eloff and to every
recipient of RevScope under the [Apache License
2.0](https://www.apache.org/licenses/LICENSE-2.0). Record this by signing off
each commit with `git commit -s`. If a commit includes someone else's work, such
as a co-author's, signing it off certifies that you have the right to submit
that work under these terms too.
