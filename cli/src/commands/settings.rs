//! A project's settings.

use revscope_api::paths;
use revscope_api::settings::{SaveSettingsRequest, SettingsResponse};
use serde::Serialize;

use super::account::timezone_note;
use crate::cli::SetSettingsArgs;
use crate::context::Context;
use crate::error::{CliError, Result};
use crate::output::{Render, Report, opt};

pub fn show(context: &Context) -> Result<Report> {
    let project = context.project()?;
    let settings: SettingsResponse = context.client.get(&paths::settings(&project.id))?;
    Report::of(&Shown::of(settings, None))
}

pub fn set(context: &Context, args: &SetSettingsArgs) -> Result<Report> {
    if args.timezone.is_none()
        && args.payment_success_path.is_none()
        && args.custom_domain.is_none()
    {
        return Err(CliError::usage(
            "nothing to change; pass --timezone, --payment-success-path, or --custom-domain",
        ));
    }
    let project = context.project()?;
    let body = SaveSettingsRequest {
        timezone: args.timezone.clone(),
        // The server clears these on an empty string; JSON null would mean no change.
        payment_success_path: args.payment_success_path.clone().map(Some),
        custom_domain: args.custom_domain.clone().map(Some),
        integrations: Vec::new(),
    };
    let settings: SettingsResponse = context.client.post(&paths::settings(&project.id), &body)?;
    let note = timezone_note(args.timezone.as_deref(), Some(&settings.timezone));
    Report::of(&Shown::of(settings, note))
}

#[derive(Serialize)]
struct Shown {
    timezone: String,
    payment_success_path: Option<String>,
    custom_domain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

impl Shown {
    /// Integrations have their own command, so they're left out here.
    fn of(settings: SettingsResponse, note: Option<String>) -> Self {
        Self {
            timezone: settings.timezone,
            payment_success_path: settings.payment_success_path,
            custom_domain: settings.custom_domain,
            note,
        }
    }
}

impl Render for Shown {
    fn render(&self) -> String {
        let mut out = format!(
            "Time zone: {}\nPayment success path: {}\nCustom domain: {}",
            self.timezone,
            opt(self.payment_success_path.as_deref()),
            opt(self.custom_domain.as_deref()),
        );
        if let Some(note) = &self.note {
            out.push_str("\n\n");
            out.push_str(note);
        }
        out
    }
}
