//! Changelists: named groups of changed files (IntelliJ's changelists).
//!
//! Purely a GlideGit notion: git knows nothing about them. The state lives
//! in `<git dir>/glidegit/changelists.json`, so it follows the repository
//! (and each worktree has its own) without touching anything git tracks.
//! Files without an entry belong to the active list.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const DEFAULT_ID: &str = "default";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Changelist {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Changelists {
    pub lists: Vec<Changelist>,
    /// Where new changes go.
    pub active: String,
    /// Path → list id, for files not in the active list.
    pub files: BTreeMap<String, String>,
}

impl Default for Changelists {
    fn default() -> Self {
        Self {
            lists: vec![Changelist {
                id: DEFAULT_ID.into(),
                name: "Changes".into(),
            }],
            active: DEFAULT_ID.into(),
            files: BTreeMap::new(),
        }
    }
}

impl Changelists {
    /// Repair anything a hand edit or an older version could leave behind:
    /// the default list exists, ids are unique, `active` and every file
    /// point at an existing list.
    pub fn normalized(mut self) -> Self {
        let mut seen = std::collections::HashSet::new();
        self.lists
            .retain(|l| !l.id.is_empty() && seen.insert(l.id.clone()));
        if !self.lists.iter().any(|l| l.id == DEFAULT_ID) {
            self.lists.insert(0, Changelists::default().lists.remove(0));
        }
        if !self.lists.iter().any(|l| l.id == self.active) {
            self.active = DEFAULT_ID.into();
        }
        let ids: std::collections::HashSet<&String> = self.lists.iter().map(|l| &l.id).collect();
        let active = self.active.clone();
        self.files.retain(|_, id| ids.contains(id) && *id != active);
        self
    }
}

fn file(git_dir: &Path) -> PathBuf {
    git_dir.join("glidegit").join("changelists.json")
}

pub fn load(git_dir: &Path) -> Changelists {
    std::fs::read_to_string(file(git_dir))
        .ok()
        .and_then(|t| serde_json::from_str::<Changelists>(&t).ok())
        .unwrap_or_default()
        .normalized()
}

pub fn save(git_dir: &Path, state: &Changelists) -> std::io::Result<Changelists> {
    let state = state.clone().normalized();
    let path = file(git_dir);
    std::fs::create_dir_all(path.parent().unwrap())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(&state).map_err(std::io::Error::other)?,
    )?;
    std::fs::rename(&tmp, &path)?;
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_repair() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()), Changelists::default());
        let mut s = Changelists::default();
        s.lists.push(Changelist {
            id: "p".into(),
            name: "Parser".into(),
        });
        s.lists.push(Changelist {
            id: "p".into(),
            name: "dup".into(),
        });
        s.files.insert("a.rs".into(), "p".into());
        s.files.insert("b.rs".into(), "gone".into());
        s.files.insert("c.rs".into(), DEFAULT_ID.into());
        let saved = save(dir.path(), &s).unwrap();
        assert_eq!(saved.lists.len(), 2);
        // Unknown lists and the active list itself are not stored per file.
        assert_eq!(saved.files.len(), 1);
        assert_eq!(load(dir.path()), saved);

        // A list that went missing: back to the default.
        let broken = Changelists {
            lists: vec![],
            active: "x".into(),
            files: BTreeMap::new(),
        };
        let fixed = broken.normalized();
        assert_eq!(fixed.active, DEFAULT_ID);
        assert_eq!(fixed.lists[0].id, DEFAULT_ID);
    }
}
