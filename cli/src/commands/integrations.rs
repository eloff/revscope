//! Connecting, inspecting and disconnecting a project's integrations.

use std::thread;
use std::time::{Duration, Instant};

use revscope_api::integrations::{self, Column, INTEGRATIONS, Integration};
use revscope_api::paths;
use revscope_api::settings::{
    IntegrationInput, IntegrationView, SaveSettingsRequest, SettingsResponse,
};
use serde::{Deserialize, Serialize};

use super::value_or_stdin;
use crate::cli::{ConnectCommand, DisconnectArgs};
use crate::context::{Context, Project};
use crate::error::{CliError, Exit, Result};
use crate::output::{Render, Report, opt, table};

/// Syncs run on a server schedule, so there's no use asking often.
const SYNC_POLL_EVERY: Duration = Duration::from_secs(5);

pub fn types() -> Result<Report> {
    Report::of(&Catalog {
        integrations: INTEGRATIONS,
    })
}

pub fn list(context: &Context) -> Result<Report> {
    let project = context.project()?;
    Report::of(&Connections {
        integrations: connected(load_settings(context, &project)?),
    })
}

/// Clearing an integration keeps its row, so only rows still holding a credential count.
fn connected(settings: SettingsResponse) -> Vec<IntegrationView> {
    settings
        .integrations
        .into_iter()
        .filter(IntegrationView::is_connected)
        .collect()
}

fn load_settings(context: &Context, project: &Project) -> Result<SettingsResponse> {
    context.client.get(&paths::settings(&project.id))
}

/// What a `connect` command was given. Values are read here, once, so stdin is
/// consumed by at most one of them.
#[derive(Debug)]
struct Given {
    integration: &'static str,
    external_account_id: Option<String>,
    api_key: Option<String>,
    secret: Option<String>,
    wait: u64,
}

fn given(command: &ConnectCommand) -> Result<Given> {
    let secret_value = |value: &Option<String>, what| {
        value
            .as_deref()
            .map(|value| value_or_stdin(value, what))
            .transpose()
    };
    let simple = |integration, value: &Option<String>, what, wait: u64| -> Result<Given> {
        Ok(Given {
            integration,
            external_account_id: None,
            api_key: secret_value(value, what)?,
            secret: None,
            wait,
        })
    };
    match command {
        ConnectCommand::Stripe(args) => simple(
            integrations::STRIPE,
            &args.secret_key,
            "secret key",
            args.wait.wait,
        ),
        ConnectCommand::Square(args) => simple(
            integrations::SQUARE,
            &args.access_token,
            "access token",
            args.wait.wait,
        ),
        ConnectCommand::LemonSqueezy(args) => simple(
            integrations::LEMON_SQUEEZY,
            &args.api_key,
            "API key",
            args.wait.wait,
        ),
        ConnectCommand::Polar(args) => simple(
            integrations::POLAR,
            &args.access_token,
            "access token",
            args.wait.wait,
        ),
        ConnectCommand::Dodo(args) => {
            simple(integrations::DODO, &args.api_key, "API key", args.wait.wait)
        }
        ConnectCommand::Cloudflare(args) => simple(
            integrations::CLOUDFLARE,
            &args.api_token,
            "API token",
            args.wait.wait,
        ),
        ConnectCommand::Shopify(args) => Ok(Given {
            integration: integrations::SHOPIFY,
            external_account_id: args.shop.clone(),
            api_key: args.client_id.clone(),
            secret: secret_value(&args.client_secret, "client secret")?,
            wait: args.wait.wait,
        }),
        ConnectCommand::SearchConsole(args) => {
            let (api_key, secret) = match &args.service_account_file {
                Some(path) => {
                    let key = service_account_key(path)?;
                    (Some(key.client_email), Some(key.private_key))
                }
                None => (
                    args.service_account_email.clone(),
                    secret_value(&args.private_key, "private key")?,
                ),
            };
            Ok(Given {
                integration: integrations::GOOGLE_SEARCH_CONSOLE,
                external_account_id: args.property.clone(),
                api_key,
                secret,
                wait: args.wait.wait,
            })
        }
    }
}

/// The two fields of a Google service account's JSON key file that matter here.
#[derive(Deserialize)]
struct ServiceAccountKey {
    client_email: String,
    private_key: String,
}

fn service_account_key(path: &str) -> Result<ServiceAccountKey> {
    let json = if path == "-" {
        value_or_stdin("-", "service account key")?
    } else {
        std::fs::read_to_string(path)
            .map_err(|error| CliError::usage(format!("reading {path}: {error}")))?
    };
    // serde's message can quote the input, and this input holds a private key.
    serde_json::from_str(&json).map_err(|_| {
        CliError::usage(format!(
            "{path} isn't a service account key: expected JSON with client_email and private_key"
        ))
    })
}

pub fn connect(context: &Context, command: &ConnectCommand) -> Result<Report> {
    let given = given(command)?;
    let integration = integrations::find(given.integration)
        .expect("each connect command names an integration in the catalog");
    if given.external_account_id.is_none() && given.api_key.is_none() && given.secret.is_none() {
        return Err(CliError::usage(format!(
            "nothing to save; `revscope integrations connect {} --help` lists what it takes",
            integration.id
        )));
    }

    let project = context.project()?;
    let current = load_settings(context, &project)?
        .integrations
        .into_iter()
        .find(|view| view.integration_type == integration.id);
    check_complete(integration, &given, current.as_ref())?;

    // Omitted credentials keep their stored value, but an omitted account is
    // written as NULL, so a stored one is sent back along with the new key.
    let external_account_id = given.external_account_id.clone().or_else(|| {
        current
            .as_ref()
            .and_then(|view| view.external_account_id.clone())
    });
    let saved: SettingsResponse = save(
        context,
        &project,
        IntegrationInput {
            integration_type: integration.id.to_string(),
            external_account_id,
            api_key: given.api_key.clone(),
            secret: given.secret.clone(),
        },
    )?;
    let view = saved
        .integrations
        .into_iter()
        .find(|view| view.integration_type == integration.id)
        .ok_or_else(|| CliError::local("the server saved the integration but doesn't list it"))?;

    let (view, sync) = if given.wait == 0 {
        (view, SyncOutcome::NotChecked)
    } else {
        wait_for_sync(context, &project, view, Duration::from_secs(given.wait))
            .map_err(|error| error.after(format!("{} was saved", integration.label)))?
    };
    Report::of(&Connected {
        label: integration.label,
        project: project.name,
        sync,
        integration: view,
    })
}

/// Every field the integration takes must be given now or already stored.
fn check_complete(
    integration: &Integration,
    given: &Given,
    current: Option<&IntegrationView>,
) -> Result<()> {
    let missing: Vec<&str> = integration
        .fields
        .iter()
        .filter(|field| match field.column {
            Column::ApiKey => {
                given.api_key.is_none() && current.is_none_or(|view| view.api_key_masked.is_none())
            }
            Column::Secret => {
                given.secret.is_none() && current.is_none_or(|view| view.secret_masked.is_none())
            }
            Column::ExternalAccountId => {
                given.external_account_id.is_none()
                    && current.is_none_or(|view| view.external_account_id.is_none())
            }
        })
        .map(|field| field.label)
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    Err(CliError::usage(format!(
        "{} also needs: {}. `revscope integrations connect {} --help` lists the flags.",
        integration.label,
        missing.join(", "),
        integration.id
    )))
}

/// Waits for a sync attempt that started after this save. The server only reports
/// attempts made with the current key or account, so one using the old key doesn't count.
fn wait_for_sync(
    context: &Context,
    project: &Project,
    saved: IntegrationView,
    wait: Duration,
) -> Result<(IntegrationView, SyncOutcome)> {
    let baseline = saved.last_sync_attempted_at.clone();
    let deadline = Instant::now() + wait;
    let mut latest = saved;
    loop {
        if let Some(outcome) = sync_outcome(&latest, baseline.as_deref()) {
            return Ok((latest, outcome));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok((latest, SyncOutcome::TimedOut));
        }
        thread::sleep(SYNC_POLL_EVERY.min(remaining));
        if let Some(view) = load_settings(context, project)?
            .integrations
            .into_iter()
            .find(|view| view.integration_type == latest.integration_type)
        {
            latest = view;
        }
    }
}

/// How the attempt after `baseline` ended, or `None` if there isn't one yet or it's running.
fn sync_outcome(latest: &IntegrationView, baseline: Option<&str>) -> Option<SyncOutcome> {
    let attempted = latest.last_sync_attempted_at.as_deref()?;
    if Some(attempted) == baseline {
        return None;
    }
    if latest.last_sync_error.is_some() {
        return Some(SyncOutcome::Failed);
    }
    (latest.last_synced_at.as_deref() == Some(attempted)).then_some(SyncOutcome::Succeeded)
}

pub fn disconnect(context: &Context, args: &DisconnectArgs) -> Result<Report> {
    let Some(integration) = integrations::find(&args.integration) else {
        let names: Vec<&str> = INTEGRATIONS.iter().map(|item| item.id).collect();
        return Err(CliError::usage(format!(
            "unknown integration `{}`; one of: {}",
            args.integration,
            names.join(", ")
        )));
    };
    let project = context.project()?;
    let _: SettingsResponse = save(
        context,
        &project,
        IntegrationInput {
            integration_type: integration.id.to_string(),
            external_account_id: None,
            // An empty string clears a stored credential.
            api_key: Some(String::new()),
            secret: Some(String::new()),
        },
    )?;
    Report::of(&Disconnected {
        disconnected: integration.id,
        label: integration.label,
        project: project.name,
    })
}

fn save(context: &Context, project: &Project, input: IntegrationInput) -> Result<SettingsResponse> {
    context.client.post(
        &paths::settings(&project.id),
        &SaveSettingsRequest {
            integrations: vec![input],
            ..SaveSettingsRequest::default()
        },
    )
}

#[derive(Serialize)]
struct Catalog {
    integrations: &'static [Integration],
}

impl Render for Catalog {
    fn render(&self) -> String {
        let mut out = String::new();
        for integration in self.integrations {
            out.push_str(&format!(
                "{} ({}): {}\n  revscope integrations connect {} --help\n",
                integration.label, integration.id, integration.description, integration.id
            ));
            for field in integration.fields {
                out.push_str(&format!("  - {}", field.label));
                if let Some(example) = field.example {
                    out.push_str(&format!(", such as {example}"));
                }
                if let Some(help) = field.help {
                    out.push_str(&format!(". {help}"));
                }
                out.push('\n');
            }
            out.push('\n');
        }
        out.trim_end().to_string()
    }
}

#[derive(Serialize)]
struct Connections {
    integrations: Vec<IntegrationView>,
}

impl Render for Connections {
    fn render(&self) -> String {
        if self.integrations.is_empty() {
            return "No integrations connected. `revscope integrations types` lists them."
                .to_string();
        }
        let rows: Vec<Vec<String>> = self
            .integrations
            .iter()
            .map(|view| {
                vec![
                    view.integration_type.clone(),
                    opt(view.external_account_id.as_deref()).to_string(),
                    opt(view.api_key_masked.as_deref()).to_string(),
                    opt(view.last_synced_at.as_deref()).to_string(),
                    // An older server doesn't send the finished error.
                    opt(view
                        .last_finished_sync_error
                        .as_deref()
                        .or(view.last_sync_error.as_deref()))
                    .to_string(),
                ]
            })
            .collect();
        table(
            &[
                "INTEGRATION",
                "ACCOUNT",
                "API KEY",
                "LAST SYNC",
                "LAST ERROR",
            ],
            &rows,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum SyncOutcome {
    /// `--wait` wasn't given.
    NotChecked,
    Succeeded,
    Failed,
    TimedOut,
}

#[derive(Serialize)]
struct Connected {
    #[serde(skip)]
    label: &'static str,
    project: String,
    sync: SyncOutcome,
    integration: IntegrationView,
}

impl Render for Connected {
    fn render(&self) -> String {
        let saved = format!("Saved {} on {}.", self.label, self.project);
        match self.sync {
            SyncOutcome::NotChecked => format!(
                "{saved} The credential hasn't been tried yet: syncs run on a schedule. Add --wait 300 to wait for one, or check `revscope integrations list`."
            ),
            SyncOutcome::Succeeded => format!("{saved} A sync succeeded."),
            SyncOutcome::Failed => format!(
                "{saved} The sync failed: {}",
                opt(self.integration.last_sync_error.as_deref())
            ),
            SyncOutcome::TimedOut => format!(
                "{saved} No sync finished in time. Check `revscope integrations list` later."
            ),
        }
    }

    fn exit(&self) -> Option<Exit> {
        matches!(self.sync, SyncOutcome::Failed | SyncOutcome::TimedOut)
            .then_some(Exit::CheckFailed)
    }
}

#[derive(Serialize)]
struct Disconnected {
    disconnected: &'static str,
    #[serde(skip)]
    label: &'static str,
    project: String,
}

impl Render for Disconnected {
    fn render(&self) -> String {
        format!(
            "Disconnected {} from {}. Data it already synced stays.",
            self.label, self.project
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Command, IntegrationsCommand};
    use clap::{CommandFactory, Parser};

    fn parse_connect(args: &[&str]) -> ConnectCommand {
        let mut line = vec!["revscope", "integrations", "connect"];
        line.extend_from_slice(args);
        match Cli::try_parse_from(line).unwrap().command {
            Command::Integrations(IntegrationsCommand::Connect(command)) => command,
            other => panic!("parsed as {other:?}"),
        }
    }

    fn view(external: Option<&str>, key: Option<&str>, secret: Option<&str>) -> IntegrationView {
        IntegrationView {
            integration_type: "x".into(),
            external_account_id: external.map(str::to_string),
            api_key_masked: key.map(str::to_string),
            secret_masked: secret.map(str::to_string),
            last_synced_at: None,
            last_sync_error: None,
            last_sync_attempted_at: None,
            last_finished_sync_error: None,
        }
    }

    fn after_sync(attempted: &str, synced: Option<&str>, error: Option<&str>) -> IntegrationView {
        IntegrationView {
            last_sync_attempted_at: Some(attempted.to_string()),
            last_synced_at: synced.map(str::to_string),
            last_sync_error: error.map(str::to_string),
            ..view(None, Some("***"), None)
        }
    }

    #[test]
    fn the_list_shows_the_finished_error_or_an_older_servers_only_error() {
        let finished = IntegrationView {
            last_finished_sync_error: Some("finished failure".to_string()),
            ..after_sync("t2", None, None)
        };
        let older_server = after_sync("t2", None, Some("older server failure"));
        let rendered = Connections {
            integrations: vec![finished, older_server],
        }
        .render();

        assert!(rendered.contains("finished failure"), "{rendered}");
        assert!(rendered.contains("older server failure"), "{rendered}");
    }

    #[test]
    fn only_a_sync_that_started_after_the_save_says_anything_about_the_new_key() {
        // Already reported when it was saved, so it says nothing new.
        assert_eq!(
            sync_outcome(&after_sync("t1", Some("t1"), None), Some("t1")),
            None
        );
        // Started since: running, then done.
        assert_eq!(
            sync_outcome(&after_sync("t2", Some("t1"), None), Some("t1")),
            None
        );
        assert_eq!(
            sync_outcome(&after_sync("t2", Some("t2"), None), Some("t1")),
            Some(SyncOutcome::Succeeded)
        );
        // No attempt ever, then the first.
        assert_eq!(sync_outcome(&view(None, Some("***"), None), None), None);
        assert_eq!(
            sync_outcome(&after_sync("t1", Some("t1"), None), None),
            Some(SyncOutcome::Succeeded)
        );
    }

    #[test]
    fn the_same_error_again_is_still_a_new_failure() {
        assert_eq!(
            sync_outcome(&after_sync("t2", None, Some("bad key")), Some("t1")),
            Some(SyncOutcome::Failed)
        );
    }

    /// Fills every flag of every connect command, so a flag saved to a column the
    /// catalog doesn't list, or a column with no flag, fails here.
    #[test]
    fn each_command_saves_exactly_the_columns_its_catalog_entry_lists() {
        let commands: [&[&str]; 8] = [
            &["stripe", "--secret-key", "k"],
            &[
                "shopify",
                "--shop",
                "a.myshopify.com",
                "--client-id",
                "id",
                "--client-secret",
                "k",
            ],
            &["square", "--access-token", "k"],
            &["lemon-squeezy", "--api-key", "k"],
            &["polar", "--access-token", "k"],
            &["dodo", "--api-key", "k"],
            &[
                "search-console",
                "--property",
                "sc-domain:a.com",
                "--service-account-email",
                "a@b.c",
                "--private-key",
                "k",
            ],
            &["cloudflare", "--api-token", "k"],
        ];
        for args in commands {
            let given = given(&parse_connect(args)).unwrap();
            let integration = integrations::find(given.integration).unwrap();
            for (column, value) in [
                (Column::ApiKey, &given.api_key),
                (Column::Secret, &given.secret),
                (Column::ExternalAccountId, &given.external_account_id),
            ] {
                assert_eq!(
                    integration.field(column).is_some(),
                    value.is_some(),
                    "{} {column:?}",
                    integration.id
                );
            }
        }
        assert_eq!(commands.len(), INTEGRATIONS.len());
    }

    #[test]
    fn every_integration_is_reachable_by_its_catalog_id() {
        let cli = Cli::command();
        let connect = cli
            .find_subcommand("integrations")
            .and_then(|command| command.find_subcommand("connect"))
            .unwrap();
        for integration in INTEGRATIONS {
            assert!(
                connect.get_subcommands().any(|command| {
                    command.get_name() == integration.id
                        || command
                            .get_all_aliases()
                            .any(|alias| alias == integration.id)
                }),
                "no connect command answers to {}",
                integration.id
            );
        }
    }

    #[test]
    fn a_first_connection_needs_every_field() {
        let given = given(&parse_connect(&["shopify", "--client-secret", "k"])).unwrap();
        let shopify = integrations::find(integrations::SHOPIFY).unwrap();
        let error = check_complete(shopify, &given, None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("Shop domain"), "{error}");
        assert!(error.contains("Client ID"), "{error}");
        assert!(!error.contains("Client secret"), "{error}");
    }

    #[test]
    fn rotating_one_field_relies_on_the_stored_ones() {
        let given = given(&parse_connect(&["shopify", "--client-secret", "k"])).unwrap();
        let shopify = integrations::find(integrations::SHOPIFY).unwrap();
        let stored = view(Some("a.myshopify.com"), Some("***"), Some("***"));
        assert!(check_complete(shopify, &given, Some(&stored)).is_ok());
    }

    #[test]
    fn a_disconnected_row_does_not_count_as_a_connection() {
        let settings = SettingsResponse {
            timezone: "UTC".into(),
            payment_success_path: None,
            custom_domain: None,
            custom_domain_included: true,
            integrations: vec![view(None, None, None), view(None, Some("***"), None)],
        };
        assert_eq!(connected(settings).len(), 1);
    }

    #[test]
    fn a_failed_or_unfinished_wait_exits_as_a_failed_check() {
        let connected = |sync| Connected {
            label: "Stripe",
            project: "p".into(),
            sync,
            integration: view(None, Some("***"), None),
        };
        assert_eq!(connected(SyncOutcome::NotChecked).exit(), None);
        assert_eq!(connected(SyncOutcome::Succeeded).exit(), None);
        assert_eq!(
            connected(SyncOutcome::Failed).exit(),
            Some(Exit::CheckFailed)
        );
        assert_eq!(
            connected(SyncOutcome::TimedOut).exit(),
            Some(Exit::CheckFailed)
        );
    }

    #[test]
    fn a_bad_key_file_never_echoes_its_contents() {
        let path =
            std::env::temp_dir().join(format!("revscope-cli-key-{}.json", std::process::id()));
        std::fs::write(&path, "PRIVATE-KEY-MATERIAL not json").unwrap();
        let error = match service_account_key(path.to_str().unwrap()) {
            Ok(_) => panic!("parsed a bad key file"),
            Err(error) => error.to_string(),
        };
        std::fs::remove_file(&path).unwrap();
        assert!(!error.contains("PRIVATE-KEY-MATERIAL"), "{error}");
    }
}
