//! The command line: every subcommand and flag.
//!
//! Secrets are best piped in: `-` reads the value from stdin, where shell history
//! and the process list can't see it.

use clap::{ArgAction, Args, Parser, Subcommand};
use revscope_api::query::{MAX_LIMIT, MAX_OFFSET};

pub const DEFAULT_API_URL: &str = "https://api.revscope.co";

const AFTER_HELP: &str = "\
Add --json to any command to print its result as JSON, on stdout, including failures.

Exit status:
  0   success
  1   failure with no more specific status
  2   bad command line or input
  3   not signed in, or the credential was refused
  4   not allowed, such as a plan limit
  5   not found
  6   conflict, such as an account for this website already existing
  7   the command ran, and its answer is no, such as a verify that found no tracker
  75  try again later: rate limited, the server is busy, or it could not be reached

Environment:
  REVSCOPE_TOKEN     credential to use instead of the stored one
  REVSCOPE_PROJECT   the project, instead of --project
  REVSCOPE_API_URL   the API, instead of --api-url
  REVSCOPE_PASSWORD  the password for login and signup, instead of --password-stdin

Docs: https://revscope.co/docs/cli";

#[derive(Debug, Parser)]
#[command(
    name = "revscope",
    version,
    about = "Sign up for RevScope, install its tracker, connect integrations, and query analytics.",
    after_help = AFTER_HELP
)]
pub struct Cli {
    /// The API to talk to. Local development uses http://localhost:4320.
    #[arg(long, global = true, env = "REVSCOPE_API_URL", default_value = DEFAULT_API_URL, hide_default_value = false)]
    pub api_url: String,

    /// Print the result as JSON instead of text.
    #[arg(long, global = true)]
    pub json: bool,

    /// The project to work on, by name. Optional when the account has only one.
    #[arg(long, short, global = true, env = "REVSCOPE_PROJECT")]
    pub project: Option<String>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create a free-plan account and its first project, and sign in to it.
    Signup(SignupArgs),
    /// Sign in to an existing account.
    Login(LoginArgs),
    /// Sign out, revoking the credential on the server and forgetting it here. On an account
    /// with no password, the only way back is then `password-reset`.
    Logout,
    /// Show the signed-in user and their projects.
    Whoami,
    /// Email a link for setting the account's password, which also opens the dashboard.
    /// Completing it signs out every session, including this CLI. MCP and query tokens
    /// made earlier keep working until revoked, so review those when claiming an account.
    PasswordReset(PasswordResetArgs),
    /// List and create projects.
    #[command(subcommand)]
    Projects(ProjectsCommand),
    /// Show and change a project's settings.
    #[command(subcommand)]
    Settings(SettingsCommand),
    /// Install the tracker on a site and verify it.
    #[command(subcommand)]
    Install(InstallCommand),
    /// Connect payment, store, search, and CDN integrations.
    #[command(subcommand)]
    Integrations(IntegrationsCommand),
    /// Import history from Google Analytics.
    #[command(subcommand, name = "ga-import")]
    GaImport(GaImportCommand),
    /// Run one read-only SQL statement against a project's analytics database.
    ///
    /// New events are saved to it every 5 minutes, so the latest ones may not show yet.
    /// `revscope install status` counts those waiting.
    Query(QueryArgs),
    /// List the analytics tables.
    Tables,
    /// Print the analytics schema, or one table's definition.
    Schema(SchemaArgs),
}

#[derive(Debug, Args)]
pub struct SignupArgs {
    /// Your name.
    #[arg(long, value_parser = non_blank)]
    pub name: String,
    /// The account's email address. Nothing is sent to confirm it.
    #[arg(long, value_parser = non_blank)]
    pub email: String,
    /// The website to track, such as `https://shop.example.com`.
    #[arg(long, value_parser = non_blank)]
    pub website: String,
    /// The first project's name. Defaults to one derived from the website.
    #[arg(long)]
    pub project_name: Option<String>,
    /// The dashboards' time zone, as an IANA name such as `Europe/Berlin`. Defaults to UTC.
    #[arg(long)]
    pub timezone: Option<String>,
    /// Read a password for the account from the first line of stdin. Without a password
    /// (here or in REVSCOPE_PASSWORD), the account is reachable through this CLI's token
    /// until its owner sets one with `revscope password-reset`.
    #[arg(long)]
    pub password_stdin: bool,
}

#[derive(Debug, Args)]
pub struct LoginArgs {
    /// Sign in with this email and a password. Without it, sign in through a browser, as on
    /// the website, which also works for accounts made with Google or Apple.
    #[arg(long, value_parser = non_blank)]
    pub email: Option<String>,
    /// Read the password from the first line of stdin, instead of REVSCOPE_PASSWORD.
    #[arg(long, requires = "email")]
    pub password_stdin: bool,
    /// Print the sign-in URL without trying to open it. The browser must be on this machine,
    /// since it returns to a port here. Elsewhere, use REVSCOPE_TOKEN.
    #[arg(long, conflicts_with = "email")]
    pub no_browser: bool,
    /// How long to wait for the browser sign-in, in seconds.
    #[arg(long, default_value_t = 300, conflicts_with = "email", value_parser = clap::value_parser!(u64).range(1..=3600))]
    pub timeout: u64,
}

#[derive(Debug, Args)]
pub struct PasswordResetArgs {
    /// The account's email address.
    #[arg(long, value_parser = non_blank)]
    pub email: String,
}

#[derive(Debug, Subcommand)]
pub enum ProjectsCommand {
    /// List the projects on the account.
    List,
    /// Create a project for another website. The free plan allows one.
    Create(CreateProjectArgs),
}

#[derive(Debug, Args)]
pub struct CreateProjectArgs {
    /// The website to track, such as `https://blog.example.com`.
    #[arg(long, value_parser = non_blank)]
    pub website: String,
    /// IANA time zone for the dashboards. Defaults to UTC.
    #[arg(long)]
    pub timezone: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum SettingsCommand {
    /// Show the project's settings.
    Show,
    /// Change the project's settings. Only the flags given change.
    Set(SetSettingsArgs),
}

#[derive(Debug, Args)]
pub struct SetSettingsArgs {
    /// An IANA time zone such as `Europe/Berlin`.
    #[arg(long, value_parser = non_blank)]
    pub timezone: Option<String>,
    /// The path a visitor lands on after paying, such as `/thanks`. Empty clears it.
    #[arg(long)]
    pub payment_success_path: Option<String>,
    /// A hostname CNAMEd to RevScope to serve the tracker from. Empty clears it.
    #[arg(long)]
    pub custom_domain: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum InstallCommand {
    /// Print the snippet to paste into the <head> of each page.
    Snippet,
    /// Show recent event volume, including events not saved yet, and each integration's
    /// sync status.
    Status,
    /// Check a page for the tracker, and whether visits have arrived in the last hour.
    ///
    /// Succeeds once the tracker is in the page's HTML or a visit has been recorded, since
    /// a check run without a browser never causes a visit itself. Exits 7 otherwise. Sees
    /// visits the server hasn't saved to the database yet.
    Verify(VerifyArgs),
}

#[derive(Debug, Args)]
pub struct VerifyArgs {
    /// The page to fetch. Defaults to the site's URL, on an account with one project.
    #[arg(long)]
    pub url: Option<String>,
    /// How long to wait for a visit, in seconds, at most 600.
    #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u64).range(0..=600))]
    pub wait: u64,
    /// Also require a recorded visit, not just the tracker in the page.
    #[arg(long)]
    pub require_visit: bool,
}

#[derive(Debug, Subcommand)]
pub enum IntegrationsCommand {
    /// List the integrations, and the credentials each one takes.
    Types,
    /// Show the connected integrations and how their syncs went.
    List,
    /// Connect an integration, or change its credentials. A field left out keeps its stored
    /// value, so rotating one key doesn't need the others.
    #[command(subcommand)]
    Connect(ConnectCommand),
    /// Disconnect an integration, clearing its credentials and account.
    Disconnect(DisconnectArgs),
}

#[derive(Debug, Subcommand)]
pub enum ConnectCommand {
    /// Stripe: payments, subscriptions, and refunds.
    Stripe(StripeArgs),
    /// Shopify: orders, refunds, and subscription contracts.
    Shopify(ShopifyArgs),
    /// Square: payments, refunds, subscriptions, and invoices.
    Square(SquareArgs),
    /// Lemon Squeezy: subscriptions and subscription invoices.
    #[command(visible_alias = "lemon_squeezy")]
    LemonSqueezy(LemonSqueezyArgs),
    /// Polar: orders and subscriptions.
    Polar(PolarArgs),
    /// Dodo Payments: payments, subscriptions, and refunds.
    Dodo(DodoArgs),
    /// Google Search Console: organic search traffic.
    #[command(visible_alias = "google_search_console")]
    SearchConsole(SearchConsoleArgs),
    /// Cloudflare: AI crawler request counts.
    Cloudflare(CloudflareArgs),
}

/// How long `connect` waits for the first sync to report.
#[derive(Debug, Args)]
pub struct WaitArg {
    /// Wait this many seconds, at most 600, for the next sync to finish, so a bad credential
    /// usually shows up now. Exits 7 if it fails or doesn't finish in time.
    #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u64).range(0..=600))]
    pub wait: u64,
}

#[derive(Debug, Args)]
pub struct StripeArgs {
    /// A secret or restricted key, or `-` for stdin. Prefer a restricted key with read access
    /// to Balance, Charges, Checkout Sessions, Coupons, Disputes, Invoices, PaymentIntents,
    /// Refunds, and Subscriptions.
    #[arg(long, value_parser = non_blank)]
    pub secret_key: Option<String>,
    #[command(flatten)]
    pub wait: WaitArg,
}

#[derive(Debug, Args)]
pub struct ShopifyArgs {
    /// The store's myshopify.com domain or full admin URL.
    #[arg(long, value_parser = literal)]
    pub shop: Option<String>,
    /// An admin access token, or `-` for stdin. Requires read_orders and
    /// read_own_subscription_contracts.
    #[arg(long, value_parser = non_blank)]
    pub admin_token: Option<String>,
    #[command(flatten)]
    pub wait: WaitArg,
}

#[derive(Debug, Args)]
pub struct SquareArgs {
    /// An access token, or `-` for stdin. Requires PAYMENTS_READ, SUBSCRIPTIONS_READ,
    /// INVOICES_READ, MERCHANT_PROFILE_READ, and ITEMS_READ.
    #[arg(long, value_parser = non_blank)]
    pub access_token: Option<String>,
    #[command(flatten)]
    pub wait: WaitArg,
}

#[derive(Debug, Args)]
pub struct LemonSqueezyArgs {
    /// A live-mode API key, or `-` for stdin.
    #[arg(long, value_parser = non_blank)]
    pub api_key: Option<String>,
    #[command(flatten)]
    pub wait: WaitArg,
}

#[derive(Debug, Args)]
pub struct PolarArgs {
    /// An organization access token, or `-` for stdin. Requires orders:read,
    /// subscriptions:read, and refunds:read.
    #[arg(long, value_parser = non_blank)]
    pub access_token: Option<String>,
    #[command(flatten)]
    pub wait: WaitArg,
}

#[derive(Debug, Args)]
pub struct DodoArgs {
    /// A read-only API key, or `-` for stdin.
    #[arg(long, value_parser = non_blank)]
    pub api_key: Option<String>,
    #[command(flatten)]
    pub wait: WaitArg,
}

#[derive(Debug, Args)]
pub struct SearchConsoleArgs {
    /// The verified property to read, such as `sc-domain:example.com`. Add the service
    /// account to it as a user.
    #[arg(long, value_parser = literal)]
    pub property: Option<String>,
    /// A service account's JSON key file, or `-` for stdin. Supplies the email and private key.
    #[arg(long, value_parser = non_blank, conflicts_with_all = ["service_account_email", "private_key"])]
    pub service_account_file: Option<String>,
    /// The service account's email, if not using --service-account-file.
    #[arg(long, value_parser = literal)]
    pub service_account_email: Option<String>,
    /// The service account's PEM private key, or `-` for stdin, if not using
    /// --service-account-file.
    #[arg(long, value_parser = non_blank)]
    pub private_key: Option<String>,
    #[command(flatten)]
    pub wait: WaitArg,
}

#[derive(Debug, Args)]
pub struct CloudflareArgs {
    /// An API token, or `-` for stdin, with Zone Read and Analytics Read on every zone that
    /// serves the site.
    #[arg(long, value_parser = non_blank)]
    pub api_token: Option<String>,
    #[command(flatten)]
    pub wait: WaitArg,
}

#[derive(Debug, Args)]
pub struct DisconnectArgs {
    /// Which one, as `revscope integrations types` names it, such as `stripe`.
    pub integration: String,
}

#[derive(Debug, Subcommand)]
pub enum GaImportCommand {
    /// Connect a Google service account, and list the properties it can read.
    Connect(GaConnectArgs),
    /// Print a URL for connecting with a Google login instead, which finishes in the dashboard.
    OauthUrl,
    /// List imports, their progress, and the dates allowed.
    List,
    /// Start importing a property. It runs on the server; `ga-import list` shows progress.
    Start(GaStartArgs),
    /// Stop a running import, keeping what it brought in so far.
    Cancel(GaImportIdArgs),
    /// Delete an import and the data it brought in.
    Delete(GaImportIdArgs),
}

#[derive(Debug, Args)]
pub struct GaConnectArgs {
    /// A service account's JSON key file, or `-` for stdin. Add its email to the property
    /// as a viewer.
    #[arg(long, value_parser = non_blank)]
    pub service_account_file: String,
}

#[derive(Debug, Args)]
pub struct GaStartArgs {
    /// The connection id `ga-import connect` printed.
    #[arg(long, value_parser = non_blank)]
    pub connection: String,
    /// The property id `ga-import connect` listed.
    #[arg(long, value_parser = non_blank)]
    pub property: String,
    /// First day to import, as YYYY-MM-DD.
    #[arg(long, value_parser = non_blank)]
    pub start: String,
    /// Last day to import, as YYYY-MM-DD.
    #[arg(long, value_parser = non_blank)]
    pub end: String,
    /// Only import these hostnames. Repeat for several.
    #[arg(long)]
    pub hostname: Vec<String>,
    /// Import days RevScope also tracked itself, counting those visits twice.
    #[arg(long)]
    pub allow_duplicates: bool,
}

#[derive(Debug, Args)]
pub struct GaImportIdArgs {
    /// The import's id, from `ga-import list`.
    pub id: i32,
}

#[derive(Debug, Args)]
pub struct QueryArgs {
    /// One SELECT, WITH, VALUES, or EXPLAIN statement, or `-` for stdin.
    pub sql: String,
    /// Rows per page.
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..=MAX_LIMIT as i64))]
    pub limit: Option<u32>,
    /// Rows to skip. Use an ORDER BY so pages are stable.
    #[arg(long, value_parser = clap::value_parser!(u32).range(0..=MAX_OFFSET as i64))]
    pub offset: Option<u32>,
    /// Fetch every page, as far as the server's offset limit allows, and return all rows.
    #[arg(long, action = ArgAction::SetTrue, conflicts_with = "offset")]
    pub all: bool,
}

#[derive(Debug, Args)]
pub struct SchemaArgs {
    /// Show only this table's definition and its indexes. `revscope tables` lists the names.
    #[arg(long)]
    pub table: Option<String>,
}

/// A value where blank is a mistake, not a way to clear something.
fn non_blank(value: &str) -> Result<String, String> {
    if value.trim().is_empty() {
        return Err("must not be blank".to_string());
    }
    Ok(value.to_string())
}

/// A blank value, and also `-`, which on these flags would be taken literally
/// rather than read from stdin.
fn literal(value: &str) -> Result<String, String> {
    if value.trim() == "-" {
        return Err("takes the value itself; `-` reads stdin only for secrets".to_string());
    }
    non_blank(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn the_command_tree_is_well_formed() {
        Cli::command().debug_assert();
    }

    #[test]
    fn global_flags_work_after_the_subcommand() {
        let cli =
            Cli::try_parse_from(["revscope", "tables", "--project", "shop", "--json"]).unwrap();
        assert!(cli.json);
        assert_eq!(cli.project.as_deref(), Some("shop"));
    }

    #[test]
    fn blank_credentials_and_dashes_on_literal_flags_are_refused_by_the_parser() {
        for args in [
            vec![
                "revscope",
                "integrations",
                "connect",
                "stripe",
                "--secret-key",
                "  ",
            ],
            vec![
                "revscope",
                "integrations",
                "connect",
                "shopify",
                "--shop",
                "-",
            ],
            vec![
                "revscope",
                "integrations",
                "connect",
                "shopify",
                "--shop",
                "",
            ],
        ] {
            assert!(Cli::try_parse_from(&args).is_err(), "{args:?}");
        }
    }

    #[test]
    fn limits_come_from_the_shared_contract() {
        let over = (MAX_OFFSET + 1).to_string();
        assert!(Cli::try_parse_from(["revscope", "query", "SELECT 1", "--offset", &over]).is_err());
        let max = MAX_OFFSET.to_string();
        assert!(Cli::try_parse_from(["revscope", "query", "SELECT 1", "--offset", &max]).is_ok());
    }
}
