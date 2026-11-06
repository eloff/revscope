//! Loads `.e2e.env` for tests that need the main Postgres database. Public
//! only so the integration tests in `tests/` can share it.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Once;

static LOAD: Once = Once::new();

/// `.e2e.env` is gitignored, so a linked worktree has none of its own; there
/// the main checkout's copy is used instead. A missing file is left for the
/// caller's `DATABASE_URL` assert to report.
pub fn load_e2e_env() {
    LOAD.call_once(|| {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let local = manifest_dir.join(".e2e.env");
        let path = if local.is_file() {
            Some(local)
        } else {
            main_checkout_dir(manifest_dir).map(|dir| dir.join(".e2e.env"))
        };
        if let Some(path) = path {
            let _ = dotenv::from_path(&path);
        }
    });
}

/// The directory in the main checkout that corresponds to `dir` in a linked
/// worktree. Assumes the main checkout's git dir is the default `.git` inside it.
fn main_checkout_dir(dir: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .args([
            "rev-parse",
            "--path-format=absolute",
            "--git-common-dir",
            "--show-prefix",
        ])
        .current_dir(dir)
        // Inherited from a git hook or the shell, these override discovery from `current_dir`.
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    let mut lines = stdout.lines();
    let common_dir = Path::new(lines.next()?);
    let prefix = lines.next().unwrap_or("");
    Some(common_dir.parent()?.join(prefix))
}
