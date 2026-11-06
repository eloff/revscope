//! What a signed-in command works with: the API client, the stored credential it
//! was built from, and how to find the project a command means.

use std::path::{Path, PathBuf};

use revscope_api::auth::MeResponse;
use revscope_api::paths;

use crate::client::{ApiClient, normalize_api_url};
use crate::config;
use crate::error::{CliError, Exit, Result};

/// Overrides the stored credential, for CI and for agents that are handed one.
const TOKEN_ENV: &str = "REVSCOPE_TOKEN";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub id: String,
    pub name: String,
}

pub struct Context {
    pub client: ApiClient,
    /// From `--project` or its variable.
    pub project: Option<String>,
    api_url: String,
    /// `None` with a token from the environment and nowhere to keep one, which is fine
    /// until something needs to keep one.
    credentials_path: Option<PathBuf>,
    token_from_env: bool,
}

impl Context {
    pub fn open(api_url: &str, project: Option<String>) -> Result<Self> {
        let api_url = normalize_api_url(api_url)?;
        let env_token = std::env::var(TOKEN_ENV)
            .ok()
            .filter(|token| !token.trim().is_empty());
        let token_from_env = env_token.is_some();
        let credentials_path = match config::default_dir() {
            Ok(dir) => Some(dir.join("credentials.json")),
            Err(_) if token_from_env => None,
            Err(error) => return Err(error),
        };
        let credential = match (env_token, &credentials_path) {
            (Some(token), _) => Some(token),
            (None, Some(path)) => config::load(path)?.token(&api_url).map(str::to_string),
            (None, None) => None,
        };
        if credential
            .as_deref()
            .is_some_and(|token| !is_header_safe(token))
        {
            return Err(CliError::usage(
                "the credential holds characters no token has; check REVSCOPE_TOKEN or the credentials file",
            ));
        }
        Ok(Self {
            client: ApiClient::new(&api_url, credential)?,
            project,
            api_url,
            credentials_path,
            token_from_env,
        })
    }

    pub fn token_from_env(&self) -> bool {
        self.token_from_env
    }

    /// Fails now if a token couldn't be kept, before something that only issues one once.
    pub fn check_can_keep_token(&self) -> Result<()> {
        let path = self.credentials_path()?;
        config::save(path, &config::load(path)?)
    }

    fn credentials_path(&self) -> Result<&Path> {
        self.credentials_path.as_deref().ok_or_else(|| {
            CliError::local("no home directory to keep credentials in; set REVSCOPE_CONFIG_DIR")
        })
    }

    /// Keeps a token the server just issued, so later runs stay signed in. Reads the
    /// file again first, so a token another process saved for another API survives.
    pub fn remember_token(&mut self, token: String) -> Result<()> {
        let path = self.credentials_path()?;
        let mut credentials = config::load(path)?;
        credentials.set_token(&self.api_url, token.clone());
        config::save(path, &credentials)?;
        self.client = ApiClient::new(&self.api_url, Some(token))?;
        Ok(())
    }

    /// Forgets the credential in use. One from the environment isn't stored, and the
    /// stored one wasn't what got revoked, so it's left alone rather than orphaned.
    pub fn forget_token(&mut self) -> Result<()> {
        if self.token_from_env {
            return Ok(());
        }
        let path = self.credentials_path()?;
        let mut credentials = config::load(path)?;
        credentials.clear_token(&self.api_url);
        config::save(path, &credentials)?;
        self.client = ApiClient::new(&self.api_url, None)?;
        Ok(())
    }

    /// Refuses a new sign-in that would be lost or would lose something: an environment
    /// token keeps taking precedence over what it stores, so later commands would act on
    /// the wrong account, and overwriting a live token can lose the only way into its
    /// account. A dead one is fine to replace.
    pub fn refuse_if_signed_in(&self) -> Result<()> {
        let refused = |message: String| CliError::Failed {
            exit: Exit::Conflict,
            message,
        };
        if self.token_from_env {
            return Err(refused(
                "REVSCOPE_TOKEN is set and would keep overriding this sign-in; unset it first"
                    .to_string(),
            ));
        }
        if !self.client.has_credential() {
            return Ok(());
        }
        match self.me() {
            Ok(me) => Err(refused(refuse_message(&me.email))),
            Err(error) if error.exit() == Exit::Auth => Ok(()),
            Err(error) => Err(error),
        }
    }

    pub fn me(&self) -> Result<MeResponse> {
        self.client.get(paths::ME)
    }

    pub fn projects(&self) -> Result<Vec<Project>> {
        Ok(self
            .me()?
            .projects
            .into_iter()
            .map(|(name, project)| Project {
                id: project.id,
                name,
            })
            .collect())
    }

    /// The project a command means: the one named, or the only one on the account.
    pub fn project(&self) -> Result<Project> {
        choose_project(self.projects()?, self.project.as_deref())
    }
}

fn refuse_message(email: &str) -> String {
    format!(
        "already signed in as {email}, and this would replace that token. Set REVSCOPE_CONFIG_DIR to keep a second account apart. `revscope logout` ends the token for good, so if the account has no password, set one with `revscope password-reset` first"
    )
}

fn is_header_safe(token: &str) -> bool {
    !token.is_empty() && token.bytes().all(|byte| byte.is_ascii_graphic())
}

fn choose_project(projects: Vec<Project>, named: Option<&str>) -> Result<Project> {
    if let Some(name) = named {
        return projects
            .into_iter()
            .find(|project| project.name == name)
            .ok_or_else(|| CliError::usage(format!("no project named `{name}` on this account")));
    }
    let names: Vec<&str> = projects
        .iter()
        .map(|project| project.name.as_str())
        .collect();
    match names.as_slice() {
        [] => Err(CliError::usage(
            "this account has no projects yet; run `revscope projects create`",
        )),
        [_] => Ok(projects.into_iter().next().expect("one project")),
        names => Err(CliError::usage(format!(
            "this account has several projects; name one with --project ({})",
            names.join(", ")
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(name: &str) -> Project {
        Project {
            id: "1".into(),
            name: name.into(),
        }
    }

    #[test]
    fn a_token_with_control_characters_is_refused() {
        assert!(is_header_safe("rsc_0190-abc"));
        assert!(!is_header_safe("abc\ndef"));
        assert!(!is_header_safe("abc def"));
    }

    #[test]
    fn the_only_project_needs_no_name() {
        assert_eq!(choose_project(vec![project("a")], None).unwrap().name, "a");
    }

    #[test]
    fn several_projects_need_a_name_and_the_error_lists_them() {
        let error = choose_project(vec![project("a"), project("b")], None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("a, b"), "{error}");
        assert_eq!(
            choose_project(vec![project("a"), project("b")], Some("b"))
                .unwrap()
                .name,
            "b"
        );
    }

    #[test]
    fn an_unknown_name_is_refused() {
        assert!(choose_project(vec![project("a")], Some("z")).is_err());
    }
}
