//! Signing up, signing in and out, and who is signed in.

use std::time::Duration;

use revscope_api::auth::{
    ForgotPasswordRequest, LoginRequest, MeResponse, NewAccountRequest, SignupRequest,
};
use revscope_api::paths;
use serde::Serialize;

use super::{browser_login, stdin_line};
use crate::cli::{LoginArgs, PasswordResetArgs, SignupArgs};
use crate::context::Context;
use crate::error::{CliError, Exit, Result};
use crate::output::{Render, Report, table};

/// Every CLI signup is on the free plan; paid plans go through checkout.
const SIGNUP_PLAN: &str = "free";
const PASSWORD_ENV: &str = "REVSCOPE_PASSWORD";

pub fn signup(context: &mut Context, args: &SignupArgs) -> Result<Report> {
    let password = optional_password(args.password_stdin)?;
    let claimable = password.is_empty();
    context.refuse_if_signed_in()?;
    // Without a password, the token is the only way into the account about to exist.
    context.check_can_keep_token()?;
    let body = SignupRequest {
        name: args.name.clone(),
        email: args.email.clone(),
        password,
        account: NewAccountRequest {
            account_name: args.website.clone(),
            project_name: args.project_name.clone(),
            plan: SIGNUP_PLAN.to_string(),
            timezone: args.timezone.clone(),
            ..NewAccountRequest::default()
        },
        issue_token: true,
    };
    let mut me: MeResponse = context
        .client
        .post(paths::SIGNUP, &body)
        .map_err(|error| signup_failure(error, &args.email))?;
    let saved_timezone = me
        .projects
        .values()
        .next()
        .map(|project| project.settings.timezone.clone());
    let token = issued_token(&mut me)?;
    if let Err(error) = context.remember_token(token.clone()) {
        return Err(error.after(format!(
            "the account was created, but its token could not be saved. It's shown only here: {token}. Set it as REVSCOPE_TOKEN"
        )));
    }
    let mut notes = Vec::new();
    notes.extend(timezone_note(
        args.timezone.as_deref(),
        saved_timezone.as_deref(),
    ));
    if claimable {
        notes.push(format!(
            "The account has no password yet. To open the dashboard, its owner can run `revscope password-reset --email {}`.",
            args.email
        ));
    }
    Report::of(&SignedIn {
        me,
        lead: "Created the account",
        notes,
    })
}

pub fn login(context: &mut Context, args: &LoginArgs) -> Result<Report> {
    context.refuse_if_signed_in()?;
    let Some(email) = &args.email else {
        let token = browser_login::sign_in(
            context,
            &browser_login::Options {
                open_browser: !args.no_browser,
                timeout: Duration::from_secs(args.timeout),
            },
        )?;
        context.remember_token(token)?;
        return Report::of(&SignedIn {
            me: context.me()?,
            lead: "Signed in",
            notes: Vec::new(),
        });
    };
    let password = if args.password_stdin {
        stdin_line("password")?
    } else {
        std::env::var(PASSWORD_ENV)
            .ok()
            .filter(|password| !password.trim().is_empty())
            .ok_or_else(|| {
                CliError::usage(format!(
                    "provide the password with --password-stdin or {PASSWORD_ENV}"
                ))
            })?
    };
    let body = LoginRequest {
        username: email.clone(),
        password,
        issue_token: true,
    };
    let mut me: MeResponse = context.client.post(paths::LOGIN, &body)?;
    let token = issued_token(&mut me)?;
    context.remember_token(token)?;
    Report::of(&SignedIn {
        me,
        lead: "Signed in",
        notes: Vec::new(),
    })
}

/// Takes the token out of the response, so it can't reach the output.
fn issued_token(me: &mut MeResponse) -> Result<String> {
    me.api_token.take().ok_or_else(|| {
        CliError::local("the server signed in but issued no token; is it older than this CLI?")
    })
}

/// The credential is forgotten only once it's dead on the server. If the server can't be
/// reached it's still live, so it's kept, for the retry the exit status asks for.
pub fn logout(context: &mut Context) -> Result<Report> {
    match context
        .client
        .post_empty::<serde_json::Value>(paths::LOGOUT)
    {
        Ok(_) => {}
        Err(error) if error.exit() == Exit::Auth => {}
        Err(error) => {
            return Err(error.hint("The credential is still valid, and still kept here"));
        }
    }
    context.forget_token()?;
    Report::of(&SignedOut {
        signed_out: true,
        token_from_env: context.token_from_env(),
    })
}

pub fn whoami(context: &Context) -> Result<Report> {
    Report::of(&SignedIn {
        me: context.me()?,
        lead: "Signed in",
        notes: Vec::new(),
    })
}

pub fn password_reset(context: &Context, args: &PasswordResetArgs) -> Result<Report> {
    let body = ForgotPasswordRequest {
        email: args.email.clone(),
    };
    let _: serde_json::Value = context.client.post(paths::FORGOT_PASSWORD, &body)?;
    Report::of(&ResetRequested {
        requested: true,
        email: args.email.clone(),
    })
}

/// A signup whose answer was lost may still have made the account, so its retry can
/// meet its own account. Both need saying, since the token is then gone.
fn signup_failure(error: CliError, email: &str) -> CliError {
    let claim = format!("`revscope password-reset --email {email}` reaches it");
    match error.exit() {
        Exit::Retry => error.hint(format!(
            "The account may have been created anyway. If a retry says it exists, {claim}"
        )),
        Exit::Conflict => error.hint(format!("If this is your account, {claim}")),
        _ => error,
    }
}

fn optional_password(from_stdin: bool) -> Result<String> {
    if from_stdin {
        return stdin_line("password");
    }
    Ok(std::env::var(PASSWORD_ENV)
        .ok()
        .filter(|password| !password.trim().is_empty())
        .unwrap_or_default())
}

/// What to say when the server saved a different time zone than was asked for:
/// signup and project creation save UTC for a name it doesn't recognize, and a
/// settings save keeps the current one.
pub fn timezone_note(requested: Option<&str>, saved: Option<&str>) -> Option<String> {
    let requested = requested?.trim();
    let saved = saved.unwrap_or("UTC");
    (requested != saved).then(|| {
        format!(
            "Warning: the time zone `{requested}` was not recognized, so the dashboards use {saved}. Set an IANA name with `revscope settings set --timezone <name>`."
        )
    })
}

#[derive(Serialize)]
struct SignedIn {
    #[serde(flatten)]
    me: MeResponse,
    #[serde(skip)]
    lead: &'static str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    notes: Vec<String>,
}

impl Render for SignedIn {
    fn render(&self) -> String {
        let me = &self.me;
        let mut out = format!(
            "{}: {} <{}> on the {} plan.",
            self.lead, me.name, me.email, me.plan
        );
        if me.projects.is_empty() {
            out.push_str("\nNo projects yet. Run `revscope projects create --website ...`.");
        } else {
            let rows: Vec<Vec<String>> = me
                .projects
                .iter()
                .map(|(name, project)| vec![name.clone(), project.id.clone()])
                .collect();
            out.push_str("\n\n");
            out.push_str(&table(&["PROJECT", "ID"], &rows));
        }
        if !me.account_confirmed {
            out.push_str(&format!(
                "\n\nAsk AI, Google Analytics imports and integrations stay off until someone on the account confirms their email address. New accounts are emailed a link, and `revscope password-reset --email {}` confirms it too.",
                me.email
            ));
        }
        for note in &self.notes {
            out.push_str("\n\n");
            out.push_str(note);
        }
        out
    }
}

#[derive(Serialize)]
struct SignedOut {
    signed_out: bool,
    #[serde(skip)]
    token_from_env: bool,
}

impl Render for SignedOut {
    fn render(&self) -> String {
        if self.token_from_env {
            "Signed out the credential in REVSCOPE_TOKEN. Any stored credential is untouched."
                .to_string()
        } else {
            "Signed out.".to_string()
        }
    }
}

#[derive(Serialize)]
struct ResetRequested {
    requested: bool,
    email: String,
}

impl Render for ResetRequested {
    fn render(&self) -> String {
        format!(
            "If {} has an account, a link for setting its password is on its way. Completing it signs out every session, including this CLI.",
            self.email
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_time_zone_is_flagged_only_when_the_saved_one_differs() {
        let note = timezone_note(Some("Europe/Berln"), Some("UTC")).unwrap();
        assert!(note.contains("`Europe/Berln` was not recognized"), "{note}");
        assert_eq!(
            timezone_note(Some(" Europe/Berlin "), Some("Europe/Berlin")),
            None
        );
        assert_eq!(timezone_note(None, Some("UTC")), None);
    }
}
