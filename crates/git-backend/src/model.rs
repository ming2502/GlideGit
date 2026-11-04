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

/// A file changed by a commit (compared with its first parent).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CommitFile {
    pub path: String,
    /// Old path of a rename or copy.
    pub orig_path: Option<String>,
    pub kind: ChangeKind,
    /// Lines added / deleted; `None` for binary files.
    pub added: Option<u32>,
    pub deleted: Option<u32>,
}

/// Everything the Log shows about one commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CommitDetails {
    pub oid: String,
    pub parents: Vec<String>,
    pub author_name: String,
    pub author_email: String,
    pub author_time: i64,
    pub committer_name: String,
    pub committer_email: String,
    pub committer_time: i64,
    /// Full message, trailing newlines removed.
    pub message: String,
    /// Changes against the first parent (everything, for a root commit).
    pub files: Vec<CommitFile>,
}

/// One entry of `git stash list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StashEntry {
    /// n in `stash@{n}`.
    pub index: u32,
    pub oid: String,
    /// The message without git's "WIP on" / "On <branch>:" prefix.
    pub message: String,
    /// Branch the stash was made on, when git recorded one.
    pub branch: Option<String>,
    /// Creation time, unix seconds.
    pub time: i64,
}

/// Options for `git stash push`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct StashOptions {
    pub message: Option<String>,
    pub include_untracked: bool,
    /// Leave staged changes in place (they are stashed too).
    pub keep_index: bool,
    /// Only these paths; empty = everything.
    pub paths: Vec<String>,
}

/// Both versions of a file, for the side-by-side diff editor.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FileTexts {
    pub path: String,
    /// Before the change; `None` when the file did not exist.
    pub old: Option<String>,
    /// After the change; `None` when the file was deleted.
    pub new: Option<String>,
    /// Either side is binary; no texts are returned then.
    pub binary: bool,
    /// Either side is over the size limit; no texts are returned then.
    pub too_large: bool,
    /// The staged version, for working-copy diffs against HEAD (used to show
    /// which blocks and lines are staged). `None` when not in the index.
    pub index: Option<String>,
    /// Some version is not valid UTF-8 or has bare CR line breaks: the texts
    /// are approximate and must not be written back.
    pub lossy: bool,
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
