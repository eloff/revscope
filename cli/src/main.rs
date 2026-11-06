//! The `revscope` command line client.

mod cli;
mod client;
mod commands;
mod config;
mod context;
mod error;
mod output;

use std::io::{ErrorKind, Write};
use std::process::ExitCode;

use clap::Parser;

use cli::{
    Cli, Command, GaImportCommand, InstallCommand, IntegrationsCommand, ProjectsCommand,
    SettingsCommand,
};
use commands::{account, ga_import, install, integrations, projects, query, settings};
use context::Context;
use error::{CliError, Exit, Result};
use output::Report;

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => return refuse_command_line(&error),
    };
    match run(&cli) {
        Ok(report) => {
            let printed = if cli.json {
                print_json(&report.json)
            } else {
                print_stdout(&report.text)
            };
            match printed {
                Ok(()) => report.exit.map_or(ExitCode::SUCCESS, ExitCode::from),
                Err(error) => print_failure(&error, false),
            }
        }
        Err(error) => print_failure(&error, cli.json),
    }
}

fn print_failure(error: &CliError, json: bool) -> ExitCode {
    // On stdout with --json, so a script reads one stream either way.
    if json {
        let _ = print_json(&error.to_json());
    } else {
        output::print_stderr(&format!("error: {error}"));
    }
    error.exit().into()
}

/// `--json` promises JSON for failures too, including ones found while parsing it.
/// Help and version are clap's "errors" as well, and print as they always do.
fn refuse_command_line(error: &clap::Error) -> ExitCode {
    if !error.use_stderr() {
        let _ = error.print();
        return ExitCode::SUCCESS;
    }
    // Not `args()`, which panics on an argument that isn't UTF-8, the very thing being
    // refused here. After `--`, a `--json` is a value rather than the flag.
    let asked_for_json = std::env::args_os()
        .take_while(|argument| argument != "--")
        .any(|argument| argument == "--json");
    if asked_for_json {
        let _ = print_json(&CliError::usage(error.render().to_string()).to_json());
    } else {
        let _ = error.print();
    }
    Exit::Usage.into()
}

fn print_json(value: &serde_json::Value) -> Result<()> {
    print_stdout(&serde_json::to_string_pretty(value).expect("a JSON value always serializes"))
}

/// A reader that went away, as with `| head`, wanted no more, so that's fine. Any other
/// failure, such as a full disk, means the result wasn't delivered, which must show.
fn print_stdout(text: &str) -> Result<()> {
    match writeln!(std::io::stdout(), "{text}") {
        Err(error) if error.kind() != ErrorKind::BrokenPipe => {
            Err(CliError::local(format!("writing the output: {error}")))
        }
        _ => Ok(()),
    }
}

fn run(cli: &Cli) -> Result<Report> {
    // Before anything is loaded, since it needs no credentials or network.
    if let Command::Integrations(IntegrationsCommand::Types) = &cli.command {
        return integrations::types();
    }
    let mut context = Context::open(&cli.api_url, cli.project.clone())?;
    match &cli.command {
        Command::Signup(args) => account::signup(&mut context, args),
        Command::Login(args) => account::login(&mut context, args),
        Command::Logout => account::logout(&mut context),
        Command::Whoami => account::whoami(&context),
        Command::PasswordReset(args) => account::password_reset(&context, args),
        Command::Projects(ProjectsCommand::List) => projects::list(&context),
        Command::Projects(ProjectsCommand::Create(args)) => projects::create(&context, args),
        Command::Settings(SettingsCommand::Show) => settings::show(&context),
        Command::Settings(SettingsCommand::Set(args)) => settings::set(&context, args),
        Command::Install(InstallCommand::Snippet) => install::snippet(&context),
        Command::Install(InstallCommand::Status) => install::status(&context),
        Command::Install(InstallCommand::Verify(args)) => install::verify(&context, args),
        Command::Integrations(IntegrationsCommand::Types) => integrations::types(),
        Command::Integrations(IntegrationsCommand::List) => integrations::list(&context),
        Command::Integrations(IntegrationsCommand::Connect(command)) => {
            integrations::connect(&context, command)
        }
        Command::Integrations(IntegrationsCommand::Disconnect(args)) => {
            integrations::disconnect(&context, args)
        }
        Command::GaImport(GaImportCommand::Connect(args)) => ga_import::connect(&context, args),
        Command::GaImport(GaImportCommand::OauthUrl) => ga_import::oauth_url(&context),
        Command::GaImport(GaImportCommand::List) => ga_import::list(&context),
        Command::GaImport(GaImportCommand::Start(args)) => ga_import::start(&context, args),
        Command::GaImport(GaImportCommand::Cancel(args)) => ga_import::cancel(&context, args),
        Command::GaImport(GaImportCommand::Delete(args)) => ga_import::delete(&context, args),
        Command::Query(args) => query::query(&context, args),
        Command::Tables => query::tables(&context),
        Command::Schema(args) => query::schema(&context, args),
    }
}
