//! Settings that must be known before any repository is opened, kept in
//! `<app config dir>/settings.json`. Display preferences (theme, shortcuts)
//! live in the UI's own storage.

use std::path::{Path, PathBuf};

use glide_git_backend::{tr, GitCli};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// The git executable to use; `None` finds one automatically.
    pub git_path: Option<String>,
}

fn file(dir: &Path) -> PathBuf {
    dir.join("settings.json")
}

pub fn load(dir: &Path) -> Settings {
    std::fs::read_to_string(file(dir))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save(dir: &Path, settings: &Settings) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let tmp = file(dir).with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(settings).map_err(std::io::Error::other)?,
    )?;
    std::fs::rename(tmp, file(dir))
}

/// Which git the app runs, and why.
#[derive(Debug, Clone)]
pub struct GitInfo {
    pub version: String,
    pub path: String,
    /// The MinGit shipped with the app.
    pub bundled: bool,
    /// Something to tell the user (then `version` may be empty).
    pub problem: Option<GitProblem>,
}

/// Found at start-up, put into words when the UI asks (its language is
/// known by then).
#[derive(Debug, Clone)]
pub enum GitProblem {
    /// The configured git does not run; another one is used.
    ConfiguredFails(String),
    /// No git at all.
    NotFound(String),
}

impl GitProblem {
    pub fn message(&self) -> String {
        match self {
            GitProblem::ConfiguredFails(e) => tr!(
                "设置中指定的 git 无法运行（{e}），已改用自动找到的 git",
                "The git set in the settings does not run ({e}); using the one found automatically"
            ),
            GitProblem::NotFound(e) => tr!(
                "没有找到 git：{e}。请安装 git，或在设置中指定 git 的位置。",
                "No git found: {e}. Install git, or set where it is in the settings."
            ),
        }
    }
}

/// Pick the git to use: the configured one if it runs, else discovery
/// (`bundled` last). `None` when there is no git at all.
pub fn resolve_git(
    settings: &Settings,
    bundled: &[PathBuf],
) -> (Option<GitCli>, Option<GitProblem>) {
    let mut error = None;
    if let Some(p) = settings
        .git_path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        match GitCli::with_executable(p) {
            Ok(cli) => return (Some(cli), None),
            Err(e) => error = Some(GitProblem::ConfiguredFails(e.to_string())),
        }
    }
    match GitCli::discover_with(bundled) {
        Ok(cli) => (Some(cli), error),
        Err(e) => (None, Some(GitProblem::NotFound(e.to_string()))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_bad_git_path() {
        let dir = std::env::temp_dir().join(format!("glidegit-settings-{}", std::process::id()));
        assert_eq!(load(&dir), Settings::default());
        let s = Settings {
            git_path: Some("/nonexistent/git".into()),
        };
        save(&dir, &s).unwrap();
        assert_eq!(load(&dir), s);
        // A configured git that does not run falls back to discovery, and says so.
        let (cli, error) = resolve_git(&s, &[]);
        assert!(cli.is_some());
        assert!(matches!(error, Some(GitProblem::ConfiguredFails(_))));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
