# revscope CLI

Command line access to RevScope, for coding agents and people: create an account, install the tracker on a site, connect integrations, and query analytics, with no browser needed.

## Install

```sh
curl -fsSL https://github.com/eloff/revscope/releases/latest/download/install.sh | sh
```

Or through npm, which also works on Windows, and fetches the native binary on first run:

```sh
npx revscope --help
```

From a checkout: `cargo install --path cli`.

## Quick start

```sh
# Create a free account and its first project. This signs you in.
revscope signup --name "Dana Example" --email dana@example.com \
  --website https://shop.example.com

# Print the snippet to add to every page's <head>, then check it.
revscope install snippet
revscope install verify

# Connect a payment provider. - reads the key from stdin.
revscope integrations types
printf '%s' "$STRIPE_KEY" | revscope integrations connect stripe --secret-key -

# Ask questions.
revscope tables
revscope schema --table pageviews
revscope query "SELECT p.path, COUNT(*) AS views
  FROM pageviews v JOIN paths p ON p.id = v.path_id
  GROUP BY p.path ORDER BY views DESC" --limit 20
```

## Commands

| Command | What it does |
| --- | --- |
| `signup` | Creates a free-plan account and its first project, and signs in. |
| `login` | Signs in through a browser, or with `--email` and a password. |
| `logout` | Revokes the credential on the server and forgets it here. |
| `whoami` | Shows the signed-in user and their projects. |
| `password-reset` | Emails a link for setting the account's password. |
| `projects list`, `create` | Lists projects, or adds one, named after its website or `--project-name`. The free plan allows one. |
| `settings show`, `set` | Time zone, payment success path, and custom domain. |
| `install snippet` | Prints the tracker snippet, pointing at this project's server. |
| `install status` | Recent events, including those not saved yet, and integration sync status. |
| `install verify` | Checks a page for the tracker, and whether visits have arrived in the last hour. Needs `--url` on an account with several projects. |
| `integrations types` | Lists the integrations and what each takes. Needs no sign-in. |
| `integrations list` | Shows connected integrations and how their syncs went. |
| `integrations connect <name>` | Connects one, or changes its credentials. |
| `integrations disconnect <name>` | Clears its credentials and account. Synced data stays. |
| `ga-import connect`, `oauth-url` | Connects a Google credential. |
| `ga-import list`, `start`, `cancel`, `delete` | Imports history from Google Analytics. |
| `query` | Runs one read-only SQL statement. `--all` follows the pages. |
| `tables` | Lists the analytics tables. |
| `schema` | Prints the schema, or one table's with `--table`. |

Run `revscope <command> --help` for every flag. Commands that work on a project take `--project <name>` or `REVSCOPE_PROJECT`, and can leave it out when the account has one.

## Accounts and signing in

`signup` without a password gives the CLI a token and nothing else, which suits an agent acting for someone. To open the dashboard, the account's owner runs `revscope password-reset --email ...` and follows the link. Setting a password signs out every session, including the CLI's, so run `revscope login` afterwards. MCP and query tokens made earlier keep working until revoked, so whoever claims an account made by someone else should review those, and its users.

`login` with no flags uses a browser, which works for anyone who can sign in on the website, including accounts made with Google or Apple. The browser must be on the same machine, since it returns to a port there; `--no-browser` only skips opening it. On a machine with no browser, use `REVSCOPE_TOKEN`.

`signup` and `login` refuse while a live credential is stored, or while `REVSCOPE_TOKEN` is set, since replacing a token can lose the only way into its account. Set `REVSCOPE_CONFIG_DIR` to keep accounts apart. `revscope logout` ends the token for good, so on an account with no password, the only way back is then a reset email.

The credential lasts a year and is kept in `$XDG_CONFIG_HOME/revscope/credentials.json` (or `~/.config/revscope/`), readable only by its owner, one per API URL. `REVSCOPE_CONFIG_DIR` moves it, and `REVSCOPE_TOKEN` overrides it.

## Scripts and agents

`--json` prints the result as JSON on stdout, including failures:

```json
{
  "error": {
    "code": "forbidden",
    "exit_code": 4,
    "message": "Your plan includes one website. Upgrade to Growth to add more.",
    "status": 403
  }
}
```

| Exit status | Meaning |
| --- | --- |
| 0 | Success |
| 1 | Failure with no more specific status |
| 2 | Bad command line or input |
| 3 | Not signed in, or the credential was refused |
| 4 | Not allowed, such as a plan limit |
| 5 | Not found |
| 6 | Conflict, such as an account for this website already existing |
| 7 | The command ran, and its answer is no, such as a verify that found no tracker |
| 75 | Try again later: rate limited, the server is busy, or it could not be reached |

Errors that say when to retry have `retry_after_seconds`.

`install verify` succeeds once the tracker is in the page's HTML, or a visit arrived in the last hour, since a check run without a browser never causes a visit itself. `--require-visit` demands the visit, and `--wait` says how long to wait for one.

## Fresh data

The server saves new events to the database every five minutes. `install verify` and `install status` see them right away. `query` reads the database, so the latest events can take that long to show up.

## Secrets

Pass `-` as the value of a key flag to read it from stdin, which keeps it out of shell history and the process list. Only one value on a command can come from stdin, so Search Console's `--service-account-file` takes a whole Google key file. A value typed as a flag also works, but is visible in both places.

Passwords come from `--password-stdin` or `REVSCOPE_PASSWORD`, never a flag.

## Another server

```sh
REVSCOPE_API_URL=http://localhost:4320 revscope whoami
```

or `--api-url`. Plain `http://` is refused except to a loopback address, so a credential never crosses the network unencrypted. Accounts on a self-hosted server need a password, since a reset link needs outgoing email.

## Limits

- `signup` creates free-plan accounts. Paid plans go through checkout on the site.
- The CLI doesn't change your site's code. `install snippet` prints what to add.
- Credentials are saved without being tried. Syncs run on a schedule, and a changed credential is due on the next pass, so `integrations connect --wait 300` waits for one to finish. A sync with the old credential doesn't count, but the wait can run out if another integration's sync is ahead of it.
- `query` offsets stop at 5000, so narrow a larger result with `WHERE`.
