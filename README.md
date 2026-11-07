# RevScope

RevScope is revenue attribution analytics for websites. A small script on your
pages records what visitors do (the pages they view, the searches and
referrers that brought them, the events they trigger). RevScope then joins that
activity to the payments they make in Stripe, Polar, Shopify, and other
billing systems. The result shows which pages, channels, and campaigns
actually bring in revenue, not just traffic.

There are two ways to use it:

- **The hosted service** at [revscope.co](https://revscope.co). There is
  nothing to install.
- **Your own server.** This repository has everything needed to run a private
  instance. The fastest route is Docker, described next.

Agents and scripts can set up the hosted service from the command line, with no
browser: create an account, install the tracker, connect integrations, and query
analytics. See [`cli/`](cli/README.md).

## Run your own instance with Docker

You need a machine with Docker and Compose v2. `compose.yaml` builds a single
backend image that also serves the web app, and runs it next to Postgres.

1. Create the configuration file:

   ```sh
   cp docker.env.example .env
   ```

2. In `.env`, fill in `INTEGRATION_ENCRYPTION_KEY` and `POSTGRES_PASSWORD`. The
   comments above each one show the command that generates it.

3. Build and start:

   ```sh
   docker compose up -d --build
   ```

   The first build compiles the Rust backend and takes several minutes.

4. Open <http://localhost:4320/ui/signup> and create your login right away.
   The first person to sign up owns the instance, and signup closes after that.
   Everyone else joins by invitation (see
   [How accounts work](#how-accounts-work-on-your-own-instance)).

### Make it reachable from the internet

The backend listens only on `127.0.0.1:4320`, so at first nothing outside the
server can reach it. To put it on a public domain, choose one of these:

- **Use the bundled Caddy server.** Point your domain's DNS at the server and
  open ports 80 and 443. Then set these in `.env`:

  ```sh
  COMPOSE_PROFILES=caddy
  REVSCOPE_DOMAIN=analytics.example.com
  REVSCOPE_WEB_BASE_URL=https://analytics.example.com/ui
  ```

  Run `docker compose up -d` again. Caddy gets a certificate from Let's Encrypt
  and serves HTTPS.

- **Use your own reverse proxy** (nginx, Traefik, and so on). Proxy to
  `127.0.0.1:4320` and make sure the proxy sets the `X-Forwarded-For` header.
  The container reads the visitor's IP address from that header. This is safe
  only because nothing but this machine can reach port 4320, so don't publish
  the port more widely.

`REVSCOPE_WEB_BASE_URL` matters in both cases, because it is the address used
in emailed links such as invitations and password resets.

### Add the tracker to your site

Copy the snippet from [`tracker/INSTALL.md`](tracker/INSTALL.md) into the
`<head>` of every page. The snippet sends data to the hosted service unless you
give it your instance's URL, so add that URL as the third argument on its last
line:

```js
})(window, "examplecom", "https://analytics.example.com/");
```

`examplecom` stands for your project's name: the one you chose at signup, or
for a project you add later, the name the dashboard shows.

### Back up, upgrade, and configure

Your data lives in two Docker volumes. `revscope-data` holds one SQLite file per
project with the analytics events, and `postgres-data` holds logins, settings,
and integrations. Back up both.

To upgrade, run `git pull` and then `docker compose up -d --build`. Database
migrations run automatically on startup.

Every variable in `.env` is passed to the backend, so any option from
[`revscope/.env.example`](revscope/.env.example) works there too, such as
email, AI chat, or error reporting. Run `docker compose up -d` after changing
it.

Visitor locations (country and city) need MaxMind's free GeoLite2-City
database, which can't be redistributed in this repository. Create a free
MaxMind account and set `MAXMIND_ACCOUNT_ID` and `MAXMIND_LICENSE_KEY` in `.env`
before building, and the build downloads the database and embeds it. If
`revscope/GeoLite2-City.mmdb` already exists in the checkout, the build uses
that file instead. Without either, geolocation is turned off.

### How accounts work on your own instance

A self-hosted instance has one account, and every user sees all of its projects.
Signup creates the first project, named as you choose, and you can add more from
the dashboard. After the first person signs up, they invite others from
Settings. An invitation link lets the invited person choose a password and
expires after 7 days, and someone who hasn't accepted 30 days after their latest
invitation is removed, so they stop reserving their email address. RevScope emails the link if email is configured
(`MAILCHANNELS_API_KEY` and `MAILCHANNELS_FROM_EMAIL`), and Settings always
shows it so you can send it yourself.

The hosted service's plan limits don't apply. There is no cap on events and no
monthly budget for AI chat. Links to documentation point at revscope.co.

## Run your own instance without Docker

The self-hosted build is a single binary. It contains the backend, the web app,
the tracker script, the database migrations, and optionally the GeoLite2
database. You need the [prerequisites](#prerequisites) installed. Build it
like this:

```sh
(cd web && pnpm install && pnpm build:bundle)
cd revscope && cargo build --release --bin revscope --features self-hosted
```

The binary is `revscope/target/release/revscope`. Build it on a machine with
the same OS and CPU architecture as your server, or cross-compile.

To run it:

1. Create a Postgres database for it (`createdb revscope`).
2. Copy [`revscope/.env.example`](revscope/.env.example) to `.env` in the
   directory you will run the binary from, and fill it in. Only `DATABASE_URL`
   and `INTEGRATION_ENCRYPTION_KEY` are required. Set `REVSCOPE_WEB_BASE_URL` to
   your public URL followed by `/ui`, for example
   `https://analytics.example.com/ui`.
3. Run the binary as an unprivileged user under a process manager such as
   systemd.
4. Put it behind a reverse proxy that handles HTTPS, and set
   `REVSCOPE_TRUST_X_FORWARDED_FOR=true` so visitor IPs come from the proxy's
   `X-Forwarded-For` header. Make sure the backend's port can be reached only
   through the proxy, because otherwise anyone could forge that header.

The web app is served under `/ui`, and `/` redirects to `/ui/dashboard`. It
calls the API on the same origin, so no cross-origin (CORS) setup is needed.
Project databases are stored in `data/` inside the working directory, so back
up that directory along with Postgres.

Instead of a reverse proxy, the backend can serve HTTPS itself with
certificates from Let's Encrypt. Set `REVSCOPE_LISTEN_TLS=0.0.0.0:443`,
`REVSCOPE_LISTEN=0.0.0.0:80` (Let's Encrypt validates the domain over port 80),
and `REVSCOPE_TLS_DOMAINS` to your hostname, for example
`analytics.example.com`. Binding ports below 1024 as an unprivileged user needs
the `CAP_NET_BIND_SERVICE` capability.

## Configuration

The backend reads its settings from environment variables, and also from a
`.env` file in its working directory. Real environment variables take
precedence. [`revscope/.env.example`](revscope/.env.example) lists every option
with a comment explaining it. The main groups are:

| Variables                                                                  | Purpose                                                 |
| -------------------------------------------------------------------------- | ------------------------------------------------------- |
| `DATABASE_URL`, `INTEGRATION_ENCRYPTION_KEY`                               | Required: the Postgres URL and the credential key       |
| `REVSCOPE_LISTEN`                                                          | Address for plain HTTP (default `127.0.0.1:4320`)       |
| `REVSCOPE_WEB_BASE_URL`, `REVSCOPE_API_BASE_URL`                           | Public URLs used in emailed links and MCP metadata      |
| `REVSCOPE_TRUST_X_FORWARDED_FOR`                                           | Read visitor IPs from a reverse proxy's header          |
| `REVSCOPE_MAX_CONNECTIONS`                                                 | Connection cap (100,000, or less if `ulimit -n` is low) |
| `MAILCHANNELS_API_KEY`, `MAILCHANNELS_FROM_EMAIL`                          | Sending email (invitations, password resets)            |
| `REVSCOPE_GOOGLE_CLIENT_ID`, `REVSCOPE_APPLE_CLIENT_ID`                    | Sign in with Google and Apple, hidden unless configured |
| `REVSCOPE_GOOGLE_CLIENT_ID`, `REVSCOPE_GOOGLE_CLIENT_SECRET`               | "Connect with Google" for Google Analytics imports      |
| `REVSCOPE_CHAT_URL`, `REVSCOPE_CHAT_API_KEY`, `REVSCOPE_CHAT_*`            | The AI chat tab, which is hidden unless configured      |
| `SENTRY_DSN`                                                               | Error reporting to Sentry                               |
| `REVSCOPE_CORS_ORIGINS`                                                    | Hosts of a web app deployed separately from the backend |
| `REVSCOPE_LISTEN_TLS`, `REVSCOPE_TLS_DOMAINS`, `REVSCOPE_ACME_*`           | Built-in HTTPS with Let's Encrypt certificates          |
| `REVSCOPE_REQUIRE_CLOUDFLARE`                                              | Settings for the hosted service's servers               |

`INTEGRATION_ENCRYPTION_KEY` encrypts the API keys you enter for integrations
such as Stripe before they are stored. If you lose it, those keys can't be
decrypted and have to be entered again.

The AI chat tab sends requests to an OpenAI-compatible chat completions
endpoint. It was built for Cloudflare AI Gateway forwarding to xAI, so
`REVSCOPE_CHAT_GATEWAY_TOKEN` is sent as Cloudflare's `cf-aig-authorization`
header and the default model is `grok/grok-4.7`.

## Development

The repository has these parts:

| Directory              | What it is                                                                                                      |
| ---------------------- | --------------------------------------------------------------------------------------------------------------- |
| `tracker/`             | `tracker.js`, the single-file ES5 script that site owners embed on their pages.                                 |
| `revscope/`            | The Rust backend: event ingest, the query API, accounts, and integrations. SQLite per project, Postgres for accounts. |
| `web/`                 | A pnpm workspace with the web app (`web/app`, Astro and Solid) and its auth client (`web/shared`).              |
| `cli/`                 | The `revscope` command line tool, which calls only the HTTP API.                                                |
| `crates/revscope-api/` | The HTTP contract the backend and the CLI share: request and response types, route paths, and limits.           |
| `e2e/`                 | Playwright tests that drive the public demo dashboard. They only read.                                          |

See [CONTRIBUTING.md](CONTRIBUTING.md) for code conventions, the checks to run,
and how to send a pull request.

### Prerequisites

- [Rust](https://rustup.rs). `rust-toolchain.toml` pins the toolchain, and
  rustup installs it on the first build.
- [Node.js](https://nodejs.org) 24 and [pnpm](https://pnpm.io) 8. Use pnpm, not
  npm. Each JavaScript project has its own lockfile.
- PostgreSQL 17 or newer, for the backend's accounts database and its tests.
- Optionally, the MaxMind GeoLite2-City database for geolocation.
  `scripts/download-maxmind.sh` downloads it into `revscope/` given your
  MaxMind credentials.

### Get a local setup running

```sh
(cd web && pnpm install)

createdb revscope
cd revscope
cp .env.example .env    # set DATABASE_URL and INTEGRATION_ENCRYPTION_KEY
cargo run               # serves on http://localhost:4320
```

In a second terminal, start the web app:

```sh
cd web
pnpm dev                # http://localhost:4322
```

The web app calls the backend at `localhost:4320` by default. Use `localhost`
rather than `127.0.0.1` in the browser, or it won't send the session cookie.

To fill a project with synthetic data for trying out the dashboard, run
`cargo generate -- <project-id>` in `revscope/`. Run it without arguments to
see its options.

### Tests and checks

| Part        | Commands                                                                                      |
| ----------- | --------------------------------------------------------------------------------------------- |
| `tracker/`  | `pnpm check` (lint, and verifies the file is valid ES5), `pnpm test`, `pnpm build` (commit its output) |
| `revscope/` | `cargo fmt`, then `cargo clippy` and `cargo test` with and without `--features self-hosted`   |
| `web/`      | `pnpm tsc` (type check), `pnpm check` (lint and format), `pnpm test`, `pnpm build`, `pnpm build:bundle` |
| `e2e/`      | `pnpm tsc`, `pnpm check`, `pnpm test` (see [CONTRIBUTING.md](CONTRIBUTING.md#browser-tests))  |
| `cli/`      | `cargo fmt`, `cargo clippy`, `cargo test`, and in `cli/npm` `npm test`                        |
| `crates/`   | `cargo fmt`, `cargo clippy`, `cargo test`, then the checks of `revscope/` and `cli/`          |

The backend's end-to-end tests in `revscope/tests/` need Postgres. They read
`DATABASE_URL` from the environment or from `revscope/.e2e.env`. Point it at a
database used only for tests. CI runs all of the above on every pull request.

To try the tracker against a local backend, see the example site in
`tracker/example` and the steps at the end of
[`tracker/INSTALL.md`](tracker/INSTALL.md).

## How the hosted service is deployed

This section describes revscope.co's own setup. You don't need it to self-host.

The hosted service builds the backend with the `billing` feature instead of
`self-hosted`. It supports many accounts and paid plans, and it doesn't serve
the web app. Its plans, checkout and billing webhooks live in a private
repository checked out at `revscope/src/billing`; a build without that feature
gives every signup a free, unlimited account. The
web app instead runs as the `revscope-app` Cloudflare Worker (see
`web/app/wrangler.jsonc`), deployed from `web/` with `pnpm push`. The
`SERVER_HOST` variable, read at build time (see `web/app/.env.example`), tells
it where the API is. The backend accepts logged-in requests from the web app's
hosts, as listed in `REVSCOPE_CORS_ORIGINS`.

The backend answers on several hostnames: `api.revscope.co` for the API,
`in.revscope.co` for the tracker and event ingest, and `eu1.revscope.co`, which
customers point a CNAME at to use their own domain. With `REVSCOPE_LISTEN_TLS`
set, the backend gets and renews Let's Encrypt certificates for `eu1` and `api`
(the default `REVSCOPE_TLS_DOMAINS`) and for customers' domains itself.
`REVSCOPE_REQUIRE_CLOUDFLARE=true` makes the server answer plain HTTP only from
Cloudflare's IP ranges, so it can't be reached around the Cloudflare proxy that
way.

## Security

Please report vulnerabilities privately, with the "Report a vulnerability"
button on the repository's Security tab, not in a public issue.

## License

RevScope is licensed under the [Functional Source License, Version 1.1, ALv2
Future License](LICENSE) (FSL-1.1-ALv2).

In plain English (this summary isn't legally binding; the [LICENSE](LICENSE)
text is what counts):

- You can use, modify, and self-host RevScope for your own sites and business,
  including commercially.
- You can't offer it, or a modified version, to others as a competing
  commercial product or service, such as a hosted analytics service.
- Two years after each version is released, it also becomes available under the
  [Apache License 2.0](https://www.apache.org/licenses/LICENSE-2.0), with no
  restrictions beyond that license.

RevScope's charts use [ApexCharts](https://apexcharts.com), which has [its own
license](https://apexcharts.com/license) that RevScope's doesn't replace. Its
free Community License is for organizations with under $2M USD in annual
revenue. If your organization earns more and self-hosts RevScope, you need a
commercial license from ApexCharts.
