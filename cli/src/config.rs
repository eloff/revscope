//! Where the CLI keeps its credentials between runs: one per API URL, so a local
//! server and production never share one.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{CliError, Result};

#[derive(Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Credentials {
    /// API base URL -> the credential issued by it.
    #[serde(default)]
    tokens: BTreeMap<String, String>,
}

impl Credentials {
    pub fn token(&self, api_url: &str) -> Option<&str> {
        self.tokens.get(api_url).map(String::as_str)
    }

    pub fn set_token(&mut self, api_url: &str, token: String) {
        self.tokens.insert(api_url.to_string(), token);
    }

    pub fn clear_token(&mut self, api_url: &str) {
        self.tokens.remove(api_url);
    }
}

/// `$REVSCOPE_CONFIG_DIR`, else `$XDG_CONFIG_HOME/revscope`, else
/// `~/.config/revscope` (`%APPDATA%\revscope` on Windows).
pub fn default_dir() -> Result<PathBuf> {
    if let Some(dir) = env_dir("REVSCOPE_CONFIG_DIR") {
        return Ok(dir);
    }
    if let Some(dir) = env_dir("XDG_CONFIG_HOME") {
        return Ok(dir.join("revscope"));
    }
    if let Some(dir) = env_dir("HOME") {
        return Ok(dir.join(".config").join("revscope"));
    }
    if let Some(dir) = env_dir("APPDATA") {
        return Ok(dir.join("revscope"));
    }
    Err(CliError::local(
        "no home directory to keep credentials in; set REVSCOPE_CONFIG_DIR",
    ))
}

fn env_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// A missing file is an empty set of credentials, not an error.
pub fn load(path: &Path) -> Result<Credentials> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| {
            CliError::local(format!(
                "{} is not a valid credentials file ({error}); fix or delete it",
                path.display()
            ))
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Credentials::default()),
        Err(error) => Err(local_io("reading", path, error)),
    }
}

/// The new contents go to a temporary file created with owner-only permissions,
/// which then replaces the old file, so a token is never in a file another user
/// can read, not even briefly.
pub fn save(path: &Path, credentials: &Credentials) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|error| local_io("creating", dir, error))?;
    }
    let body = serde_json::to_vec_pretty(credentials)
        .map_err(|error| CliError::local(format!("encoding credentials: {error}")))?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    // A leftover from a crashed process with this pid would keep its old mode.
    let _ = fs::remove_file(&temporary);
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = options.open(&temporary).and_then(|mut file| {
        file.write_all(&body)?;
        file.sync_all()
    });
    if let Err(error) = written {
        let _ = fs::remove_file(&temporary);
        return Err(local_io("writing", &temporary, error));
    }
    fs::rename(&temporary, path).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        local_io("replacing", path, error)
    })
}

fn local_io(action: &str, path: &Path, error: std::io::Error) -> CliError {
    CliError::local(format!("{action} {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("revscope-cli-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_missing_file_is_empty_credentials() {
        let dir = scratch("missing");
        assert_eq!(
            load(&dir.join("credentials.json")).unwrap(),
            Credentials::default()
        );
    }

    #[test]
    fn tokens_are_kept_per_api_url_and_survive_a_round_trip() {
        let dir = scratch("roundtrip");
        let path = dir.join("credentials.json");
        let mut credentials = Credentials::default();
        credentials.set_token("https://api.revscope.co", "prod".into());
        credentials.set_token("http://localhost:4320", "dev".into());
        save(&path, &credentials).unwrap();

        let mut loaded = load(&path).unwrap();
        assert_eq!(loaded.token("https://api.revscope.co"), Some("prod"));
        assert_eq!(loaded.token("http://localhost:4320"), Some("dev"));
        loaded.clear_token("https://api.revscope.co");
        assert_eq!(loaded.token("https://api.revscope.co"), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn an_existing_world_readable_file_ends_up_readable_only_by_its_owner() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("mode");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("credentials.json");
        fs::write(&path, b"{}").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        save(&path, &Credentials::default()).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_file_is_reported_with_its_path() {
        let dir = scratch("corrupt");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("credentials.json");
        fs::write(&path, b"not json").unwrap();
        let error = load(&path).unwrap_err().to_string();
        assert!(error.contains("credentials.json"), "{error}");
        let _ = fs::remove_dir_all(&dir);
    }
}
