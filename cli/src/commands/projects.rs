//! Listing and creating projects.

use revscope_api::paths;
use revscope_api::projects::{CreateProjectRequest, CreateProjectResponse};
use revscope_api::settings::SettingsResponse;
use serde::Serialize;

use super::account::timezone_note;
use crate::cli::CreateProjectArgs;
use crate::context::{Context, Project};
use crate::error::Result;
use crate::output::{Render, Report, table};

pub fn list(context: &Context) -> Result<Report> {
    Report::of(&ProjectList {
        projects: context
            .projects()?
            .into_iter()
            .map(|Project { id, name }| ProjectRow { id, name })
            .collect(),
    })
}

pub fn create(context: &Context, args: &CreateProjectArgs) -> Result<Report> {
    let body = CreateProjectRequest {
        website: args.website.clone(),
        project_name: args.project_name.clone(),
        timezone: args.timezone.clone(),
    };
    let created: CreateProjectResponse = context.client.post(paths::PROJECTS, &body)?;
    // Only for the warning, so a failure here must not read as the create failing.
    let timezone = match &args.timezone {
        None => None,
        Some(_) => context
            .client
            .get::<SettingsResponse>(&paths::settings(&created.id))
            .ok()
            .and_then(|saved| timezone_note(args.timezone.as_deref(), Some(&saved.timezone))),
    };
    let note = [
        name_note(args.project_name.as_deref(), &created.name),
        timezone,
    ]
    .into_iter()
    .flatten()
    .reduce(|first, second| format!("{first}\n{second}"));
    Report::of(&Created { created, note })
}

/// Servers older than `--project-name` ignore it and name the project after the
/// website, so the name asked for may not be the one made.
fn name_note(requested: Option<&str>, created: &str) -> Option<String> {
    let requested = requested.map(str::trim).filter(|name| !name.is_empty())?;
    (requested != created).then(|| {
        format!(
            "Warning: the server named the project `{created}`, not `{requested}`, because it is too old for `--project-name`."
        )
    })
}

#[derive(Serialize)]
struct ProjectRow {
    id: String,
    name: String,
}

#[derive(Serialize)]
struct ProjectList {
    projects: Vec<ProjectRow>,
}

impl Render for ProjectList {
    fn render(&self) -> String {
        if self.projects.is_empty() {
            return "This account has no projects yet.".to_string();
        }
        let rows: Vec<Vec<String>> = self
            .projects
            .iter()
            .map(|project| vec![project.name.clone(), project.id.clone()])
            .collect();
        table(&["PROJECT", "ID"], &rows)
    }
}

#[derive(Serialize)]
struct Created {
    #[serde(flatten)]
    created: CreateProjectResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

impl Render for Created {
    fn render(&self) -> String {
        let name = &self.created.name;
        let mut out = format!(
            "Created project {name} (id {}).\nNext: `revscope install snippet --project {name}` prints the tracker snippet.",
            self.created.id
        );
        if let Some(note) = &self.note {
            out.push_str("\n\n");
            out.push_str(note);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::name_note;

    #[test]
    fn a_name_the_server_ignored_is_pointed_out() {
        assert_eq!(name_note(None, "blogexamplecom"), None);
        assert_eq!(name_note(Some("  "), "blogexamplecom"), None);
        assert_eq!(name_note(Some(" my_blog "), "my_blog"), None);
        assert!(
            name_note(Some("my_blog"), "blogexamplecom")
                .is_some_and(|note| note.contains("`blogexamplecom`, not `my_blog`"))
        );
    }
}
