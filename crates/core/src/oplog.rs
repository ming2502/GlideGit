//! Operation log and undo.
//!
//! Every recorded write compares the refs before and after (HEAD, branches,
//! tags, the stash) and derives how to put them back:
//!
//! - HEAD moved on the same branch: `reset --soft` for a commit or amend
//!   (the changes stay staged), `reset --keep` otherwise.
//! - HEAD switched branch: switch back.
//! - Branches / tags created, deleted or moved: put them back with
//!   `update-ref`, guarded by their current value.
//! - A stash was created: pop it; one was dropped: store it again.
//! - Destructive operations take a backup first: copies of the files a
//!   rollback or discard touches (restored only if they are still as the
//!   operation left them), and `git stash create` before a hard reset.
//!
//! Undo checks that the repository is still where the operation left it, so
//! it never overwrites work done since.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};

use glide_git_backend::{GitError, RefsState, Repo, ResetMode};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Entries kept in the log.
const KEEP: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum OpKind {
    Commit,
    Amend,
    Checkout,
    Branch,
    Tag,
    Reset,
    CherryPick,
    Revert,
    Rebase,
    Pull,
    Push,
    Rollback,
    Discard,
    Stash,
    Shelve,
    /// Finishing a stopped rebase / cherry-pick / revert / merge.
    Continue,
    Undo,
    Other,
}

/// One step of an undo, with what it expects to find.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum UndoStep {
    ResetSoft {
        expect_head: String,
        to: String,
    },
    ResetMixed {
        expect_head: String,
        to: String,
    },
    ResetKeep {
        expect_head: String,
        to: String,
    },
    /// Switch back to `branch` (full ref) or, detached, to `oid`.
    Switch {
        branch: Option<String>,
        oid: Option<String>,
        expect_head_ref: Option<String>,
        expect_head: Option<String>,
    },
    SetRef {
        name: String,
        oid: String,
        expect: Option<String>,
    },
    DeleteRef {
        name: String,
        expect: String,
    },
    StashPop {
        expect_top: String,
    },
    StashStore {
        oid: String,
        message: String,
    },
    RestoreBackup {
        stash: Option<String>,
        dir: Option<PathBuf>,
        files: Vec<SavedFile>,
    },
}

/// A file (or directory) saved before a destructive operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedFile {
    pub path: String,
    /// It existed before the operation (and was copied into the backup).
    pub existed: bool,
    /// Its fingerprint right after the operation; undo only restores it
    /// while it still matches.
    pub after: Option<u64>,
}

/// One recorded operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OpEntry {
    pub id: u64,
    /// Unix seconds.
    pub time: i64,
    pub kind: OpKind,
    pub title: String,
    /// What undoing does, when it can be undone.
    pub undo: Option<String>,
    /// Why it cannot be undone.
    pub blocked: Option<String>,
    pub undone: bool,
    #[ts(skip)]
    #[serde(default)]
    pub steps: Vec<UndoStep>,
}

/// What to save before a destructive operation.
#[derive(Debug, Clone, Default)]
pub struct Backup {
    /// All local changes to tracked files (`git stash create`), for
    /// operations that reset the whole worktree.
    pub stash: bool,
    /// Files the operation overwrites or deletes.
    pub files: Vec<String>,
}

/// What a caller says about the operation it is about to run.
#[derive(Debug, Clone)]
pub struct OpMeta {
    pub kind: OpKind,
    pub title: String,
    /// Back up local changes first (destructive operations).
    pub backup: Option<Backup>,
    /// A stash entry the operation removes (drop / pop): oid and message.
    pub dropped_stash: Option<(String, String)>,
    /// How to move HEAD back; by default `soft` after a commit and `keep`
    /// otherwise.
    pub reset_back: Option<ResetMode>,
}

impl OpMeta {
    pub fn new(kind: OpKind, title: impl Into<String>) -> Self {
        Self {
            kind,
            title: title.into(),
            backup: None,
            dropped_stash: None,
            reset_back: None,
        }
    }
    /// Save these files first.
    pub fn backup_files(mut self, files: Vec<String>) -> Self {
        self.backup = Some(Backup {
            stash: false,
            files,
        });
        self
    }
    /// Save all local changes first.
    pub fn backup_all(mut self) -> Self {
        self.backup = Some(Backup {
            stash: true,
            files: Vec::new(),
        });
        self
    }
    pub fn reset_back(mut self, mode: ResetMode) -> Self {
        self.reset_back = Some(mode);
        self
    }
    pub fn dropped_stash(mut self, oid: String, message: String) -> Self {
        self.dropped_stash = Some((oid, message));
        self
    }
}

/// A backup taken before the operation ran.
#[derive(Debug, Clone, Default)]
pub struct Saved {
    pub stash: Option<String>,
    pub dir: Option<PathBuf>,
    pub files: Vec<SavedFile>,
}

impl Saved {
    fn is_empty(&self) -> bool {
        self.stash.is_none() && self.files.is_empty()
    }
}

/// Where the repository stood around an operation, beyond its refs.
#[derive(Debug, Clone, Default)]
pub struct Context {
    /// A rebase, merge, cherry-pick or revert is stopped half-way.
    pub in_progress: bool,
    /// `ORIG_HEAD` after the operation (where a finished rebase started).
    pub orig_head: Option<String>,
}

/// Stable fingerprint of a file's content; directories all share one value.
/// `None` when the path does not exist.
pub fn fingerprint(path: &Path) -> Option<u64> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if meta.is_dir() {
        return Some(1);
    }
    let bytes = if meta.file_type().is_symlink() {
        std::fs::read_link(path)
            .ok()?
            .to_string_lossy()
            .into_owned()
            .into_bytes()
    } else {
        std::fs::read(path).ok()?
    };
    Some(crate::cache::fnv1a(&bytes))
}

fn short(oid: &str) -> &str {
    &oid[..oid.len().min(7)]
}

fn branch_name(r: &str) -> &str {
    r.trim_start_matches("refs/heads/")
}

/// Derive the undo of an operation from the refs before and after it.
/// Returns the steps and a description, or why it cannot be undone.
pub fn plan(
    meta: &OpMeta,
    before: &RefsState,
    after: &RefsState,
    saved: &Saved,
    ctx: &Context,
) -> Result<(Vec<UndoStep>, String), String> {
    match meta.kind {
        OpKind::Push => return Err("已推送到远程，无法在本地撤销".into()),
        OpKind::Undo => return Err("撤销操作本身不能再撤销".into()),
        _ => {}
    }
    if ctx.in_progress {
        return Err("操作在中途停下了，可在顶部横幅中中止".into());
    }
    let mut steps = Vec::new();
    let mut what: Vec<String> = Vec::new();

    // Finishing a stopped rebase: HEAD goes from detached back to the branch,
    // whose old tip is ORIG_HEAD.
    let finished_rebase = meta.kind == OpKind::Continue
        && before.head_ref.is_none()
        && after.head_ref.is_some()
        && ctx.orig_head.is_some();
    if finished_rebase {
        let (to, at) = (
            ctx.orig_head.clone().unwrap(),
            after.head_oid.clone().unwrap_or_default(),
        );
        what.push(format!(
            "{} 回到变基前的 {}",
            branch_name(after.head_ref.as_deref().unwrap()),
            short(&to)
        ));
        steps.push(UndoStep::ResetKeep {
            expect_head: at,
            to,
        });
    } else if before.head_ref != after.head_ref {
        steps.push(UndoStep::Switch {
            branch: before.head_ref.clone(),
            oid: before.head_oid.clone(),
            expect_head_ref: after.head_ref.clone(),
            expect_head: after.head_oid.clone(),
        });
        what.push(match &before.head_ref {
            Some(r) => format!("切换回 {}", branch_name(r)),
            None => format!(
                "回到 {}",
                before.head_oid.as_deref().map(short).unwrap_or("HEAD")
            ),
        });
    } else if before.head_oid != after.head_oid {
        match (&before.head_oid, &after.head_oid) {
            (Some(b), Some(a)) => {
                let commit = matches!(meta.kind, OpKind::Commit | OpKind::Amend);
                let mode = meta.reset_back.unwrap_or(if commit {
                    ResetMode::Soft
                } else {
                    ResetMode::Keep
                });
                let (expect_head, to) = (a.clone(), b.clone());
                steps.push(match mode {
                    ResetMode::Soft => UndoStep::ResetSoft { expect_head, to },
                    ResetMode::Mixed => UndoStep::ResetMixed { expect_head, to },
                    _ => UndoStep::ResetKeep { expect_head, to },
                });
                what.push(match meta.kind {
                    OpKind::Amend => "恢复修改前的提交，修改保留在暂存区".into(),
                    OpKind::Commit => "撤销提交，修改保留在暂存区".into(),
                    _ => format!("回到操作前的提交 {}", short(b)),
                });
            }
            (None, Some(_)) => return Err("这是仓库的第一个提交，无法撤销".into()),
            _ => return Err("HEAD 已无法解析".into()),
        }
    }

    // Branches and tags, except the current branch (moved by the reset above).
    let head_branch = after
        .head_ref
        .as_ref()
        .filter(|_| before.head_ref == after.head_ref);
    let names: BTreeSet<&String> = before.refs.keys().chain(after.refs.keys()).collect();
    for name in names {
        if name == "refs/stash" || Some(name) == head_branch {
            continue;
        }
        let label = if name.starts_with(glide_git_backend::SHELF_REFS) {
            "搁置记录"
        } else {
            name.trim_start_matches("refs/heads/")
                .trim_start_matches("refs/tags/")
        };
        match (before.refs.get(name), after.refs.get(name)) {
            (None, Some(a)) => {
                steps.push(UndoStep::DeleteRef {
                    name: name.clone(),
                    expect: a.clone(),
                });
                what.push(format!("删除 {label}"));
            }
            (Some(b), None) => {
                steps.push(UndoStep::SetRef {
                    name: name.clone(),
                    oid: b.clone(),
                    expect: None,
                });
                what.push(format!("恢复 {label}"));
            }
            (Some(b), Some(a)) if a != b => {
                steps.push(UndoStep::SetRef {
                    name: name.clone(),
                    oid: b.clone(),
                    expect: Some(a.clone()),
                });
                what.push(format!("{label} 移回 {}", short(b)));
            }
            _ => {}
        }
    }

    // The stash: a new entry on top, or one that went away.
    let (bs, as_) = (before.refs.get("refs/stash"), after.refs.get("refs/stash"));
    if let Some((oid, msg)) = &meta.dropped_stash {
        steps.push(UndoStep::StashStore {
            oid: oid.clone(),
            message: msg.clone(),
        });
        what.push("把 stash 放回列表".into());
    } else if let Some(top) = as_.filter(|t| Some(*t) != bs) {
        steps.push(UndoStep::StashPop {
            expect_top: top.clone(),
        });
        what.push("把修改从 stash 取回".into());
    }

    if !saved.is_empty() {
        steps.push(UndoStep::RestoreBackup {
            stash: saved.stash.clone(),
            dir: saved.dir.clone(),
            files: saved.files.clone(),
        });
        what.push(if meta.kind == OpKind::Reset {
            "恢复重置前的本地修改".into()
        } else if meta.kind == OpKind::Shelve {
            "恢复文件到操作前的内容".into()
        } else {
            "恢复被丢弃的修改（作为未暂存的修改）".into()
        });
    }

    if steps.is_empty() {
        return Err("没有可撤销的变化".into());
    }
    Ok((steps, what.join("，")))
}

/// Take the backup a destructive operation asked for, into `dir`.
pub fn take_backup(repo: &Repo, backup: &Backup, dir: &Path) -> Result<Saved, GitError> {
    let mut saved = Saved::default();
    if backup.stash {
        saved.stash = repo.stash_create()?;
    }
    for p in &backup.files {
        let src = repo.root().join(p.trim_end_matches('/'));
        let dst = dir.join(p.trim_end_matches('/'));
        let meta = std::fs::symlink_metadata(&src);
        let existed = meta.is_ok();
        if let Ok(meta) = meta {
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent)?;
            }
            if meta.is_dir() {
                copy_dir(&src, &dst)?;
            } else {
                std::fs::copy(&src, &dst)?;
            }
        }
        saved.files.push(SavedFile {
            path: p.trim_end_matches('/').to_string(),
            existed,
            after: None,
        });
    }
    if !saved.files.is_empty() {
        saved.dir = Some(dir.to_path_buf());
    }
    Ok(saved)
}

/// Record what the saved files look like once the operation has run; files
/// it did not change are dropped from the backup.
pub fn seal_backup(repo: &Repo, saved: &mut Saved) {
    let dir = saved.dir.clone();
    saved.files.retain_mut(|f| {
        f.after = fingerprint(&repo.root().join(&f.path));
        let before = match (&dir, f.existed) {
            (Some(d), true) => fingerprint(&d.join(&f.path)),
            _ => None,
        };
        before != f.after || before == Some(1)
    });
}

fn remove_path(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(_) => Ok(()),
    }
}

fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let to = dst.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &to)?;
        } else {
            std::fs::copy(e.path(), to)?;
        }
    }
    Ok(())
}

/// Check every step's expectation against the repository as it is now.
pub fn check(repo: &Repo, steps: &[UndoStep]) -> Result<(), String> {
    let now = repo.refs_state().map_err(|e| e.to_string())?;
    for s in steps {
        let ok = match s {
            UndoStep::ResetSoft { expect_head, .. }
            | UndoStep::ResetMixed { expect_head, .. }
            | UndoStep::ResetKeep { expect_head, .. } => {
                now.head_oid.as_deref() == Some(expect_head)
            }
            UndoStep::Switch {
                expect_head_ref,
                expect_head,
                ..
            } => &now.head_ref == expect_head_ref && &now.head_oid == expect_head,
            // SetRef / DeleteRef are compare-and-swap in git itself.
            UndoStep::SetRef { .. } | UndoStep::DeleteRef { .. } => true,
            UndoStep::StashPop { expect_top } => now.refs.get("refs/stash") == Some(expect_top),
            UndoStep::StashStore { .. } => true,
            UndoStep::RestoreBackup { files, .. } => {
                if let Some(f) = files
                    .iter()
                    .find(|f| fingerprint(&repo.root().join(&f.path)) != f.after)
                {
                    return Err(format!(
                        "{} 在这次操作之后又被修改过，恢复会覆盖这些修改",
                        f.path
                    ));
                }
                true
            }
        };
        if !ok {
            return Err("仓库在这次操作之后又有变化，无法安全撤销".into());
        }
    }
    Ok(())
}

/// Run undo steps, in order.
pub fn run(repo: &Repo, steps: &[UndoStep]) -> Result<(), GitError> {
    for s in steps {
        match s {
            UndoStep::ResetSoft { to, .. } => repo.reset(to, ResetMode::Soft)?,
            UndoStep::ResetMixed { to, .. } => repo.reset(to, ResetMode::Mixed)?,
            UndoStep::ResetKeep { to, .. } => repo.reset(to, ResetMode::Keep)?,
            UndoStep::Switch { branch, oid, .. } => match (branch, oid) {
                (Some(b), _) => repo.checkout_branch(branch_name(b), false)?,
                (None, Some(o)) => repo.checkout_detached(o)?,
                (None, None) => {}
            },
            UndoStep::SetRef { name, oid, expect } => {
                repo.update_ref_checked(name, oid, expect.as_deref())?
            }
            UndoStep::DeleteRef { name, expect } => repo.delete_ref_checked(name, expect)?,
            UndoStep::StashPop { expect_top } => {
                let list = repo.stash_list()?;
                match list.first() {
                    Some(e) if &e.oid == expect_top => {
                        repo.stash_apply(e.index, &e.oid, true, true)?
                    }
                    _ => return Err(GitError::Invalid("the stash changed".into())),
                }
            }
            UndoStep::StashStore { oid, message } => repo.stash_store(oid, message)?,
            UndoStep::RestoreBackup { stash, dir, files } => {
                if let Some(s) = stash {
                    repo.stash_apply_commit(s)?;
                }
                for f in files {
                    let dst = repo.root().join(&f.path);
                    remove_path(&dst)?;
                    if let (true, Some(dir)) = (f.existed, dir) {
                        let src = dir.join(&f.path);
                        if let Some(parent) = dst.parent() {
                            std::fs::create_dir_all(parent)?;
                        }
                        if src.is_dir() {
                            copy_dir(&src, &dst)?;
                        } else {
                            std::fs::copy(&src, &dst)?;
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// The log of one repository, kept in memory and (optionally) on disk as
/// JSON lines.
pub struct OpLog {
    entries: Vec<OpEntry>,
    file: Option<PathBuf>,
    next_id: u64,
}

impl OpLog {
    pub fn open(file: Option<PathBuf>) -> Self {
        let entries: Vec<OpEntry> = file
            .as_deref()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .map(|text| {
                text.lines()
                    .filter_map(|l| serde_json::from_str(l).ok())
                    .collect()
            })
            .unwrap_or_default();
        let next_id = entries.iter().map(|e| e.id).max().unwrap_or(0) + 1;
        Self {
            entries,
            file,
            next_id,
        }
    }

    pub fn next_id(&self) -> u64 {
        self.next_id
    }

    /// Newest first.
    pub fn entries(&self) -> Vec<OpEntry> {
        self.entries.iter().rev().cloned().collect()
    }

    pub fn get(&self, id: u64) -> Option<&OpEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    pub fn push(&mut self, mut e: OpEntry) {
        e.id = self.next_id;
        self.next_id += 1;
        self.entries.push(e);
        let trimmed = self.entries.len() > KEEP;
        if trimmed {
            let drop = self.entries.len() - KEEP;
            for old in self.entries.drain(..drop) {
                for step in old.steps {
                    if let UndoStep::RestoreBackup { dir: Some(d), .. } = step {
                        let _ = std::fs::remove_dir_all(d);
                    }
                }
            }
        }
        self.save(trimmed);
    }

    pub fn mark_undone(&mut self, id: u64) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.undone = true;
        }
        self.save(true);
    }

    fn save(&self, rewrite: bool) {
        let Some(file) = &self.file else { return };
        let result = (|| -> std::io::Result<()> {
            if let Some(dir) = file.parent() {
                std::fs::create_dir_all(dir)?;
            }
            if rewrite {
                let mut text = String::new();
                for e in &self.entries {
                    text.push_str(&serde_json::to_string(e).map_err(std::io::Error::other)?);
                    text.push('\n');
                }
                let tmp = file.with_extension("jsonl.tmp");
                std::fs::write(&tmp, text)?;
                std::fs::rename(&tmp, file)
            } else if let Some(e) = self.entries.last() {
                let mut f = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(file)?;
                writeln!(
                    f,
                    "{}",
                    serde_json::to_string(e).map_err(std::io::Error::other)?
                )
            } else {
                Ok(())
            }
        })();
        if let Err(e) = result {
            tracing::debug!("operation log not saved: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(head_ref: Option<&str>, head: Option<&str>, refs: &[(&str, &str)]) -> RefsState {
        RefsState {
            head_ref: head_ref.map(str::to_string),
            head_oid: head.map(str::to_string),
            refs: refs
                .iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect(),
        }
    }

    #[test]
    fn plans() {
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        let main = Some("refs/heads/main");
        let before = state(main, Some(&a), &[("refs/heads/main", &a)]);

        // Commit: soft reset back.
        let after = state(main, Some(&b), &[("refs/heads/main", &b)]);
        let (steps, what) = plan(
            &OpMeta::new(OpKind::Commit, "c"),
            &before,
            &after,
            &Saved::default(),
            &Context::default(),
        )
        .unwrap();
        assert_eq!(
            steps,
            [UndoStep::ResetSoft {
                expect_head: b.clone(),
                to: a.clone()
            }]
        );
        assert!(what.contains("暂存区"));

        // New branch + checkout: switch back, then delete it.
        let after = state(
            Some("refs/heads/x"),
            Some(&a),
            &[("refs/heads/main", &a), ("refs/heads/x", &a)],
        );
        let (steps, _) = plan(
            &OpMeta::new(OpKind::Branch, "x"),
            &before,
            &after,
            &Saved::default(),
            &Context::default(),
        )
        .unwrap();
        assert!(
            matches!(&steps[0], UndoStep::Switch { branch: Some(r), .. } if r == "refs/heads/main")
        );
        assert!(matches!(&steps[1], UndoStep::DeleteRef { name, .. } if name == "refs/heads/x"));

        // Deleted branch comes back.
        let before2 = state(
            main,
            Some(&a),
            &[("refs/heads/main", &a), ("refs/heads/old", &b)],
        );
        let after2 = state(main, Some(&a), &[("refs/heads/main", &a)]);
        let (steps, what) = plan(
            &OpMeta::new(OpKind::Branch, "d"),
            &before2,
            &after2,
            &Saved::default(),
            &Context::default(),
        )
        .unwrap();
        assert_eq!(
            steps,
            [UndoStep::SetRef {
                name: "refs/heads/old".into(),
                oid: b.clone(),
                expect: None
            }]
        );
        assert!(what.contains("恢复 old"));

        // Push, stopped operations and no-ops cannot be undone.
        assert!(plan(
            &OpMeta::new(OpKind::Push, "p"),
            &before,
            &before,
            &Saved::default(),
            &Context::default()
        )
        .is_err());
        assert!(plan(
            &OpMeta::new(OpKind::Rebase, "r"),
            &before,
            &after2,
            &Saved::default(),
            &Context {
                in_progress: true,
                ..Context::default()
            }
        )
        .is_err());
        assert!(plan(
            &OpMeta::new(OpKind::Other, "o"),
            &before,
            &before,
            &Saved::default(),
            &Context::default()
        )
        .is_err());

        // Rollback with a backup.
        let saved = Saved {
            stash: Some(b.clone()),
            ..Saved::default()
        };
        let (steps, _) = plan(
            &OpMeta::new(OpKind::Rollback, "r"),
            &before,
            &before,
            &saved,
            &Context::default(),
        )
        .unwrap();
        assert!(matches!(&steps[0], UndoStep::RestoreBackup { stash: Some(s), .. } if *s == b));
    }
}
