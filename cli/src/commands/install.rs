//! Installing the tracker: the snippet, recent activity, and verifying it works.

use std::thread;
use std::time::{Duration, Instant};

use revscope_api::install::InstallResponse;
use revscope_api::paths;
use revscope_api::troubleshooting::{
    Severity, TroubleshootingCheckRequest, TroubleshootingCheckResponse,
    TroubleshootingIngestQuery, TroubleshootingIngestResponse, TroubleshootingStatusResponse,
};
use serde::Serialize;
use url::Url;

use crate::cli::VerifyArgs;
use crate::context::{Context, Project};
use crate::error::{CliError, Exit, Result};
use crate::output::{Render, Report, opt, table};

/// How often verify asks whether a visit has arrived.
const POLL_EVERY: Duration = Duration::from_secs(2);
/// How far back a visit counts. A visit that already happened proves the tracker
/// works, however recently the check ran, and one from long ago proves little.
const VISIT_LOOKBACK_MS: i64 = 60 * 60 * 1000;
/// The server fetches the page itself, following redirects, each with its own timeout.
const CHECK_TIMEOUT: Duration = Duration::from_secs(90);

pub fn snippet(context: &Context) -> Result<Report> {
    let project = context.project()?;
    let install: InstallResponse = context.client.get(&paths::install(&project.id))?;
    Report::of(&Snippet(install))
}

pub fn status(context: &Context) -> Result<Report> {
    let project = context.project()?;
    let status: TroubleshootingStatusResponse =
        context.client.get(&paths::troubleshooting(&project.id))?;
    Report::of(&Status(status))
}

pub fn verify(context: &Context, args: &VerifyArgs) -> Result<Report> {
    let project = context.project()?;
    let url = match &args.url {
        Some(url) => url.clone(),
        None => {
            // The server's suggestion comes from the account's website, which only
            // describes the project when there's just the one.
            if context.projects()?.len() > 1 {
                return Err(CliError::usage(
                    "this account has several projects, so name the page to check with --url",
                ));
            }
            let status: TroubleshootingStatusResponse =
                context.client.get(&paths::troubleshooting(&project.id))?;
            status.default_url
        }
    };
    if url.is_empty() {
        return Err(CliError::usage(
            "no URL to check; pass --url https://your-site.example/",
        ));
    }
    let check: TroubleshootingCheckResponse = context.client.post_with_timeout(
        &paths::troubleshooting_check(&project.id),
        &TroubleshootingCheckRequest { url: url.clone() },
        CHECK_TIMEOUT,
    )?;
    let visit = wait_for_visit(
        context,
        &project,
        checked_host(&check),
        check.wait_started_at_ms.saturating_sub(VISIT_LOOKBACK_MS),
        Duration::from_secs(args.wait),
    )?;
    let state = verify_state(&check, visit.as_ref());
    Report::of(&Verified {
        state,
        require_visit: args.require_visit,
        check,
        visit,
    })
}

/// The host visits to this page are recorded under: where its redirects ended.
fn checked_host(check: &TroubleshootingCheckResponse) -> Option<String> {
    let url = check.fetch.final_url.as_deref().unwrap_or(&check.url);
    Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
}

fn verify_state(
    check: &TroubleshootingCheckResponse,
    visit: Option<&TroubleshootingIngestResponse>,
) -> VerifyState {
    if visit.is_some() {
        VerifyState::ReceivingData
    } else if !check.fetch.ok {
        VerifyState::PageUnreachable
    } else if check.install.ok {
        VerifyState::TrackerFound
    } else {
        VerifyState::TrackerMissing
    }
}

/// Asks for a visit at or after `since_ms`, at least once, until `wait` runs out.
fn wait_for_visit(
    context: &Context,
    project: &Project,
    hostname: Option<String>,
    since_ms: i64,
    wait: Duration,
) -> Result<Option<TroubleshootingIngestResponse>> {
    let path = paths::troubleshooting_ingest(
        &project.id,
        &TroubleshootingIngestQuery { since_ms, hostname },
    );
    let deadline = Instant::now() + wait;
    loop {
        let ingest: TroubleshootingIngestResponse = context.client.get(&path)?;
        if ingest.detected {
            return Ok(Some(ingest));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(None);
        }
        thread::sleep(POLL_EVERY.min(remaining));
    }
}

#[derive(Serialize)]
#[serde(transparent)]
struct Snippet(InstallResponse);

impl Render for Snippet {
    fn render(&self) -> String {
        format!(
            "Paste this into the <head> of every page on the site, before any other script:\n\n{}\n\nThen check it with `revscope install verify`.",
            self.0.snippet
        )
    }
}

#[derive(Serialize)]
#[serde(transparent)]
struct Status(TroubleshootingStatusResponse);

impl Render for Status {
    fn render(&self) -> String {
        let status = &self.0;
        let mut out = format!(
            "Project {}\nSite: {}\nEvents in the last 30 days: {}\nEvents waiting to be saved: {} (saved every 5 minutes)",
            status.project_name, status.default_url, status.events_last_month, status.events_queued
        );
        if !status.integrations.is_empty() {
            let rows: Vec<Vec<String>> = status
                .integrations
                .iter()
                .map(|integration| {
                    vec![
                        integration.integration_type.clone(),
                        serde_json::to_value(integration.status)
                            .ok()
                            .and_then(|value| value.as_str().map(str::to_string))
                            .unwrap_or_default(),
                        opt(integration.last_synced_at.as_deref()).to_string(),
                    ]
                })
                .collect();
            out.push_str("\n\n");
            out.push_str(&table(&["INTEGRATION", "STATUS", "LAST SYNC"], &rows));
        }
        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum VerifyState {
    /// The server could not download the page.
    PageUnreachable,
    /// The page loaded, but the tracker isn't in its HTML.
    TrackerMissing,
    /// The tracker is in the page's HTML. Nothing has reported yet.
    TrackerFound,
    /// A visit has been recorded, which proves it end to end.
    ReceivingData,
}

#[derive(Serialize)]
struct Verified {
    state: VerifyState,
    check: TroubleshootingCheckResponse,
    visit: Option<TroubleshootingIngestResponse>,
    #[serde(skip)]
    require_visit: bool,
}

impl Render for Verified {
    fn render(&self) -> String {
        let check = &self.check;
        let mut out = match (&check.fetch.ok, &check.fetch.error) {
            (true, _) => format!(
                "Fetched {} (HTTP {}).",
                check.url,
                check.fetch.status_code.unwrap_or(0)
            ),
            (false, error) => format!("Could not fetch {}: {}.", check.url, opt(error.as_deref())),
        };
        for finding in &check.install.findings {
            let severity = match finding.severity {
                Severity::Ok => "ok",
                Severity::Warning => "warning",
                Severity::Error => "error",
                Severity::Unknown => "note",
            };
            out.push_str(&format!("\n[{severity}] {}", finding.message));
        }
        if let Some(note) = &check.fetch.note {
            out.push_str(&format!("\n[note] {note}"));
        }
        out.push_str("\n\n");
        out.push_str(&match (self.state, &self.visit) {
            (VerifyState::ReceivingData, Some(visit)) => format!(
                "Working: a visit to {} on {} was recorded.",
                opt(visit.path.as_deref()),
                opt(visit.hostname.as_deref()),
            ),
            (VerifyState::TrackerFound, _) => {
                "The tracker is in the page. No visit in the last hour: this check only reads the HTML, so a visit needs a browser to load the page. Add --wait 60 and open the page to prove it end to end.".to_string()
            }
            (VerifyState::TrackerMissing, _) => {
                "The tracker isn't in the page. `revscope install snippet` prints what to add.".to_string()
            }
            _ => "The page could not be checked.".to_string(),
        });
        out
    }

    fn exit(&self) -> Option<Exit> {
        let passed = match self.state {
            VerifyState::ReceivingData => true,
            VerifyState::TrackerFound => !self.require_visit,
            VerifyState::TrackerMissing | VerifyState::PageUnreachable => false,
        };
        (!passed).then_some(Exit::CheckFailed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use revscope_api::troubleshooting::{FetchResult, InstallResult};

    fn check(fetched: bool, tracker: bool) -> TroubleshootingCheckResponse {
        TroubleshootingCheckResponse {
            url: "https://example.com/".to_string(),
            fetch: FetchResult {
                ok: fetched,
                status_code: Some(200),
                error: None,
                note: None,
                final_url: None,
            },
            install: InstallResult {
                ok: tracker,
                findings: Vec::new(),
            },
            wait_started_at_ms: 0,
        }
    }

    fn visit() -> TroubleshootingIngestResponse {
        TroubleshootingIngestResponse {
            detected: true,
            detected_at_ms: Some(1),
            path: Some("/".to_string()),
            hostname: Some("example.com".to_string()),
        }
    }

    fn verified(
        check: TroubleshootingCheckResponse,
        visit: Option<TroubleshootingIngestResponse>,
        require_visit: bool,
    ) -> Option<Exit> {
        Verified {
            state: verify_state(&check, visit.as_ref()),
            check,
            visit,
            require_visit,
        }
        .exit()
    }

    #[test]
    fn visits_are_looked_for_where_the_page_redirected_to() {
        let mut redirected = check(true, true);
        assert_eq!(checked_host(&redirected).as_deref(), Some("example.com"));
        redirected.fetch.final_url = Some("https://shop.example.com/".to_string());
        assert_eq!(
            checked_host(&redirected).as_deref(),
            Some("shop.example.com")
        );
    }

    #[test]
    fn the_tracker_in_the_page_passes_without_a_browser() {
        assert_eq!(verified(check(true, true), None, false), None);
    }

    #[test]
    fn requiring_a_visit_fails_until_one_arrives() {
        assert_eq!(
            verified(check(true, true), None, true),
            Some(Exit::CheckFailed)
        );
        assert_eq!(verified(check(true, true), Some(visit()), true), None);
    }

    #[test]
    fn no_tracker_or_no_page_fails() {
        assert_eq!(
            verified(check(true, false), None, false),
            Some(Exit::CheckFailed)
        );
        assert_eq!(
            verified(check(false, false), None, false),
            Some(Exit::CheckFailed)
        );
    }

    #[test]
    fn a_visit_proves_it_even_when_the_html_check_cannot() {
        // A client-rendered page injects the tracker after the HTML the check reads.
        assert_eq!(verified(check(true, false), Some(visit()), false), None);
        assert_eq!(verified(check(false, false), Some(visit()), true), None);
    }
}
