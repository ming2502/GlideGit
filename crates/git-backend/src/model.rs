//! Domain model shared with the frontend. Every type here is exported to
//! `ui/src/bindings/*.ts` by ts-rs — do not hand-edit the generated files.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ChangeKind {
    Modified,
    Added,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Untracked,
    Conflicted,
}

/// One path in `git status`. `staged` is HEAD→index, `unstaged` is index→worktree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FileChange {
    pub path: String,
    pub orig_path: Option<String>,
    pub staged: Option<ChangeKind>,
    pub unstaged: Option<ChangeKind>,
    pub conflicted: bool,
}

impl FileChange {
    pub fn is_untracked(&self) -> bool {
        self.unstaged == Some(ChangeKind::Untracked)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HeadInfo {
    /// Current branch short name; `None` when detached.
    pub branch: Option<String>,
    /// Commit id of HEAD; `None` on an unborn branch (no commits yet).
    pub oid: Option<String>,
    pub detached: bool,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum RepoOperation {
    Merge,
    Rebase,
    CherryPick,
    Revert,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Branch {
    /// Short name, e.g. `main` or `origin/main`.
    pub name: String,
    pub full_ref: String,
    pub oid: String,
    pub upstream: Option<String>,
    pub upstream_gone: bool,
    pub ahead: u32,
    pub behind: u32,
    pub is_head: bool,
    pub is_remote: bool,
    /// Committer time of the tip, unix seconds.
    pub commit_time: i64,
    pub subject: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BranchList {
    pub local: Vec<Branch>,
    pub remote: Vec<Branch>,
    /// Recently checked-out local branch names, most recent first (from the reflog).
    pub recent: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RepoSnapshot {
    pub root: String,
    pub head: HeadInfo,
    pub operation: Option<RepoOperation>,
    pub changes: Vec<FileChange>,
    pub branches: BranchList,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CommitSummary {
    pub oid: String,
    pub parents: Vec<String>,
    pub author_name: String,
    pub author_email: String,
    /// Author time, unix seconds.
    pub author_time: i64,
    pub subject: String,
    /// Decorations, e.g. `HEAD -> main`, `origin/main`, `tag: v1.0`.
    pub refs: Vec<String>,
}

/// Which two sides a file diff compares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DiffMode {
    /// index → worktree (unstaged changes)
    WorkingTree,
    /// HEAD → index (staged changes)
    Staged,
    /// HEAD → worktree (everything, IDEA "no staging area" view)
    Head,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum LineKind {
    Context,
    Added,
    Deleted,
    NoNewline,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiffHunk {
    pub header: String,
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FileDiff {
    pub path: String,
    pub binary: bool,
    /// The diff exceeded the render limit and was cut.
    pub truncated: bool,
    pub added: u32,
    pub deleted: u32,
    pub hunks: Vec<DiffHunk>,
}

/// One parsed `--progress` update, e.g. `Receiving objects: 45% (450/1000), 1.2 MiB | 2 MiB/s`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Progress {
    /// e.g. `Receiving objects`, `Writing objects`, `Counting objects`.
    pub phase: String,
    pub percent: Option<u8>,
    /// Remaining text, e.g. `(450/1000), 1.2 MiB | 2 MiB/s`.
    pub detail: String,
}

/// What `git push` would send: shown in the push dialog before pushing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PushPreview {
    pub branch: Option<String>,
    /// Remote that will receive the push (`None` when no remote is configured).
    pub remote: Option<String>,
    /// Target, e.g. `origin/main`.
    pub target: Option<String>,
    /// The branch does not exist on the remote yet; the push sets upstream.
    pub new_branch: bool,
    /// Commits that will be pushed, newest first.
    pub commits: Vec<CommitSummary>,
    /// Commits on the upstream that are not local (push would be rejected).
    pub behind: u32,
}
