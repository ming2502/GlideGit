//! Repository-level operations. All methods are blocking; callers on an async
//! runtime should use `spawn_blocking` (see `glide-core`).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::cli::{CancelToken, GitCli, ProgressSink, RunOptions};
use crate::error::{GitError, Result};
use crate::model::*;
use crate::parse;

/// Diffs larger than this many lines are truncated for display.
pub const MAX_DIFF_LINES: usize = 50_000;
/// Untracked files larger than this are not loaded for preview.
const MAX_PREVIEW_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Repo {
    cli: GitCli,
    root: PathBuf,
    git_dir: PathBuf,
    /// Shared by all worktrees: refs, packed-refs, objects. Equals `git_dir`
    /// except in linked worktrees.
    common_dir: PathBuf,
    /// Branch list from the last snapshot, reused while refs are unchanged.
    branch_cache: Arc<Mutex<Option<BranchCache>>>,
    /// "abc1234 Subject" by commit id, for operation details.
    describe_cache: Arc<Mutex<std::collections::HashMap<String, String>>>,
}

#[derive(Debug)]
struct BranchCache {
    fingerprint: u64,
    at: Instant,
    branches: BranchList,
}

/// Re-read refs at least this often even if the fingerprint did not change
/// (guards against coarse file timestamps).
const REFS_MAX_AGE: Duration = Duration::from_secs(30);

/// Text the diff editor cannot round-trip: invalid UTF-8, or a CR that does
/// not start a CRLF (the editor would treat it as a line break).
fn is_lossy(d: &[u8]) -> bool {
    std::str::from_utf8(d).is_err()
        || d.iter()
            .enumerate()
            .any(|(i, &b)| b == b'\r' && d.get(i + 1) != Some(&b'\n'))
}

/// FNV-1a over bytes and file metadata.
struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv {
    fn bytes(&mut self, b: &[u8]) {
        for &x in b {
            self.0 ^= x as u64;
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
        // Separator, so ("ab","c") and ("a","bc") differ.
        self.0 = self.0.wrapping_mul(0x0100_0000_01b3) ^ 0xff;
    }

    fn meta(&mut self, path: &Path) {
        let Ok(m) = std::fs::metadata(path) else {
            self.bytes(b"-");
            return;
        };
        let mtime = m
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        self.bytes(&mtime.to_le_bytes());
        self.bytes(&m.len().to_le_bytes());
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            self.bytes(&m.ino().to_le_bytes());
        }
    }
}

/// Where one side of a file comparison comes from.
enum Side<'a> {
    Missing,
    Worktree(&'a str),
    Index(&'a str),
    Rev(&'a str, &'a str),
}

enum Blob {
    Missing,
    TooLarge,
    Data(Vec<u8>),
}

impl Blob {
    fn into_bytes(self) -> Option<Vec<u8>> {
        match self {
            Blob::Data(d) => Some(d),
            _ => None,
        }
    }
}

/// How `git reset` treats the index and working tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ResetMode {
    /// Move the branch only; changes stay staged.
    Soft,
    /// Also reset the index; changes stay in the working tree.
    Mixed,
    /// Discard all changes.
    Hard,
    /// Like hard, but refuse if local changes would be lost.
    Keep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullMode {
    Merge,
    Rebase,
    FastForwardOnly,
}

/// New branch and tag names reach git as arguments. git validates the
/// details; this rules out anything that could be read as an option.
fn check_ref_name(name: &str) -> Result<()> {
    let bad = name.is_empty()
        || name.starts_with('-')
        || name.starts_with('/')
        || name.ends_with('/')
        || name.ends_with('.')
        || name.ends_with(".lock")
        || name.contains("..")
        || name.contains("@{")
        || name
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || "~^:?*[\\".contains(c));
    if bad {
        Err(GitError::Invalid(format!("invalid name: {name:?}")))
    } else {
        Ok(())
    }
}

/// Full ref names the operation log may change.
fn check_ref_target(name: &str) -> Result<()> {
    let ok =
        (name.starts_with("refs/heads/") || name.starts_with("refs/tags/") || name == "refs/stash")
            && check_ref_name(name.trim_start_matches("refs/")).is_ok();
    if ok {
        Ok(())
    } else {
        Err(GitError::Invalid(format!(
            "not a branch or tag ref: {name:?}"
        )))
    }
}

/// Commit ids reach git as arguments: accept hex only, never an option.
fn check_oid(oid: &str) -> Result<()> {
    if (4..=64).contains(&oid.len()) && oid.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(GitError::Invalid(format!("not a commit id: {oid:?}")))
    }
}

impl Repo {
    /// Open the repository containing `path` (any sub-directory works).
    pub fn open(cli: GitCli, path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if !path.is_dir() {
            return Err(GitError::NotARepository(path.display().to_string()));
        }
        let out = cli
            .run(
                path,
                &["rev-parse", "--show-toplevel", "--absolute-git-dir"],
                RunOptions::read(),
            )
            .map_err(|e| match e {
                GitError::Failed { .. } => GitError::NotARepository(path.display().to_string()),
                other => other,
            })?;
        let text = out.stdout_str();
        let mut lines = text.lines();
        let (Some(root), Some(git_dir)) = (lines.next(), lines.next()) else {
            return Err(GitError::NotARepository(path.display().to_string()));
        };
        let git_dir = PathBuf::from(git_dir);
        // `--path-format` needs git 2.31; older git: assume no linked worktree.
        let common_dir = cli
            .run(
                path,
                &["rev-parse", "--path-format=absolute", "--git-common-dir"],
                RunOptions::read(),
            )
            .ok()
            .map(|o| PathBuf::from(o.stdout_str().trim()))
            .filter(|p| p.is_dir())
            .unwrap_or_else(|| git_dir.clone());
        Ok(Self {
            cli,
            root: PathBuf::from(root),
            git_dir,
            common_dir,
            branch_cache: Arc::default(),
            describe_cache: Arc::default(),
        })
    }

    /// A cheap fingerprint of every ref, HEAD and the HEAD reflog, from file
    /// metadata only (no git process). Equal fingerprints mean no ref moved,
    /// so branch lists and the commit index need not be re-read.
    ///
    /// Covers loose refs, `packed-refs` and reftable; ref updates replace the
    /// file (new inode / timestamp), so even same-length rewrites show up.
    pub fn refs_fingerprint(&self) -> u64 {
        let mut h = Fnv::default();
        // HEAD's content (branch name or detached id), per worktree.
        h.bytes(&std::fs::read(self.git_dir.join("HEAD")).unwrap_or_default());
        h.meta(&self.git_dir.join("logs").join("HEAD"));
        h.meta(&self.common_dir.join("packed-refs"));
        h.meta(&self.common_dir.join("reftable").join("tables.list"));
        fn walk(dir: &Path, h: &mut Fnv) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            let mut entries: Vec<_> = entries.flatten().collect();
            entries.sort_by_key(|e| e.file_name());
            for e in entries {
                let p = e.path();
                h.bytes(p.as_os_str().as_encoded_bytes());
                match e.file_type() {
                    Ok(t) if t.is_dir() => walk(&p, h),
                    _ => h.meta(&p),
                }
            }
        }
        walk(&self.common_dir.join("refs"), &mut h);
        h.0
    }

    /// [`Repo::branches`], reused while [`Repo::refs_fingerprint`] is unchanged.
    fn branches_cached(&self) -> Result<BranchList> {
        let fingerprint = self.refs_fingerprint();
        {
            let cache = self.branch_cache.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(c) = cache.as_ref() {
                if c.fingerprint == fingerprint && c.at.elapsed() < REFS_MAX_AGE {
                    return Ok(c.branches.clone());
                }
            }
        }
        let list = self.branches()?;
        *self.branch_cache.lock().unwrap_or_else(|e| e.into_inner()) = Some(BranchCache {
            fingerprint,
            at: Instant::now(),
            branches: list.clone(),
        });
        Ok(list)
    }

    /// Write a commit-graph with changed-path Bloom filters if the repository
    /// has none. Speeds up `git log` in topological order (the first screen of
    /// the Log) and path filters a lot. It is a cache file under
    /// `objects/info`, the same one `git gc` / `git maintenance` write.
    pub fn ensure_commit_graph(&self) -> Result<bool> {
        let info = self.common_dir.join("objects").join("info");
        if info.join("commit-graph").is_file() || info.join("commit-graphs").is_dir() {
            return Ok(false);
        }
        self.write(&["commit-graph", "write", "--reachable", "--changed-paths"])?;
        Ok(true)
    }

    /// `git init` a new repository and open it.
    pub fn init(cli: GitCli, path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        std::fs::create_dir_all(path)?;
        cli.run(path, &["init"], RunOptions::write())?;
        Self::open(cli, path)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn git_dir(&self) -> &Path {
        &self.git_dir
    }

    pub fn cli(&self) -> &GitCli {
        &self.cli
    }

    fn read(&self, args: &[&str]) -> Result<String> {
        Ok(self
            .cli
            .run(&self.root, args, RunOptions::read())?
            .stdout_str()
            .into_owned())
    }

    fn write(&self, args: &[&str]) -> Result<()> {
        self.cli.run(&self.root, args, RunOptions::write())?;
        Ok(())
    }

    /// Run a write command with paths passed via stdin (no command-line length limits).
    fn write_with_paths(&self, args: &[&str], paths: &[String]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        let mut all: Vec<&str> = args.to_vec();
        all.extend(["--pathspec-from-file=-", "--pathspec-file-nul"]);
        let mut input = Vec::new();
        for p in paths {
            input.extend_from_slice(p.as_bytes());
            input.push(0);
        }
        self.cli
            .run(&self.root, &all, RunOptions::write().stdin(input))?;
        Ok(())
    }

    // ----------------------------------------------------------------- reads

    pub fn status(&self) -> Result<(HeadInfo, Vec<FileChange>)> {
        let out = self.cli.run(
            &self.root,
            &[
                "status",
                "--porcelain=v2",
                "-z",
                "--branch",
                "--untracked-files=all",
                "--find-renames",
            ],
            RunOptions::read(),
        )?;
        parse::parse_status_v2(&out.stdout)
    }

    pub fn operation(&self) -> Option<RepoOperation> {
        let g = &self.git_dir;
        if g.join("rebase-merge").is_dir() || g.join("rebase-apply").is_dir() {
            Some(RepoOperation::Rebase)
        } else if g.join("MERGE_HEAD").is_file() {
            Some(RepoOperation::Merge)
        } else if g.join("CHERRY_PICK_HEAD").is_file() {
            Some(RepoOperation::CherryPick)
        } else if g.join("REVERT_HEAD").is_file() {
            Some(RepoOperation::Revert)
        } else {
            None
        }
    }

    /// Progress and names for the operation in progress (read from git's
    /// state files; commit descriptions are cached by id).
    pub fn operation_info(&self) -> Option<OperationInfo> {
        let op = self.operation()?;
        let g = &self.git_dir;
        let read = |p: PathBuf| {
            std::fs::read_to_string(p)
                .ok()
                .map(|s| s.trim().to_string())
        };
        let num = |p: PathBuf| read(p).and_then(|s| s.parse::<u32>().ok());
        let head_file =
            |name: &str| read(g.join(name)).and_then(|s| s.lines().next().map(str::to_string));
        let mut info = OperationInfo {
            can_skip: op != RepoOperation::Merge,
            ..OperationInfo::default()
        };
        match op {
            RepoOperation::Rebase => {
                let (dir, step, total) = if g.join("rebase-merge").is_dir() {
                    (g.join("rebase-merge"), "msgnum", "end")
                } else {
                    (g.join("rebase-apply"), "next", "last")
                };
                info.step = num(dir.join(step));
                info.total = num(dir.join(total));
                info.branch = read(dir.join("head-name"))
                    .filter(|h| h != "detached HEAD")
                    .map(|h| h.trim_start_matches("refs/heads/").to_string());
                info.onto = read(dir.join("onto")).and_then(|o| self.describe(&o));
                info.commit = head_file("REBASE_HEAD").and_then(|o| self.describe(&o));
            }
            RepoOperation::Merge => {
                info.commit = head_file("MERGE_HEAD").and_then(|o| self.describe(&o))
            }
            RepoOperation::CherryPick => {
                info.commit = head_file("CHERRY_PICK_HEAD").and_then(|o| self.describe(&o))
            }
            RepoOperation::Revert => {
                info.commit = head_file("REVERT_HEAD").and_then(|o| self.describe(&o))
            }
        }
        Some(info)
    }

    /// "abc1234 Subject" for a commit id, cached (ids never change meaning).
    fn describe(&self, oid: &str) -> Option<String> {
        check_oid(oid).ok()?;
        if let Some(d) = self
            .describe_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(oid)
        {
            return Some(d.clone());
        }
        let out = self
            .read(&["log", "-1", "--format=%h %s", oid, "--"])
            .ok()?;
        let d = out.trim().to_string();
        if d.is_empty() {
            return None;
        }
        let mut cache = self
            .describe_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if cache.len() > 256 {
            cache.clear();
        }
        cache.insert(oid.to_string(), d.clone());
        Some(d)
    }

    /// The commits an interactive rebase "from `oid`" covers: `oid` itself
    /// and everything after it up to HEAD, oldest first. Refused when `oid`
    /// is not an ancestor of HEAD or the range contains merge commits
    /// (rewriting them would flatten the history).
    pub fn rebase_plan(&self, oid: &str) -> Result<Vec<CommitSummary>> {
        check_oid(oid)?;
        let ancestor = self
            .cli
            .run(
                &self.root,
                &["merge-base", "--is-ancestor", oid, "HEAD"],
                RunOptions::read(),
            )
            .is_ok();
        if !ancestor {
            return Err(GitError::Invalid(
                "this commit is not part of the current branch".into(),
            ));
        }
        let range = format!("{oid}^..HEAD");
        let root = !self.has_parent(oid);
        let mut args = vec!["log", "--reverse", parse::LOG_FORMAT, "--decorate=short"];
        if root {
            args.push("HEAD");
        } else {
            args.push(&range);
        }
        args.push("--");
        let commits = parse::parse_log(&self.read(&args)?);
        if commits.iter().any(|c| c.parents.len() > 1) {
            return Err(GitError::Invalid(
                "the range contains merge commits; interactive rebase would flatten them".into(),
            ));
        }
        Ok(commits)
    }

    fn has_parent(&self, oid: &str) -> bool {
        self.cli
            .run(
                &self.root,
                &["rev-parse", "--verify", "-q", &format!("{oid}^")],
                RunOptions::read(),
            )
            .is_ok()
    }

    /// Run an interactive rebase of the commits from `from` (inclusive) to
    /// HEAD with `steps` (oldest first): a permutation of
    /// [`Repo::rebase_plan`]'s commits with an action each.
    ///
    /// The todo list is written up front and handed to git through
    /// `GIT_SEQUENCE_EDITOR`; rewording is `pick` + `exec git commit --amend
    /// -F <file>`, so no editor ever opens. Stops (with an error) on
    /// conflicts or `edit`; the operation banner takes it from there.
    /// Returns whether the rebase finished (false: it stopped part-way).
    pub fn interactive_rebase(
        &self,
        from: &str,
        steps: &[RebaseStep],
        autostash: bool,
    ) -> Result<bool> {
        let plan = self.rebase_plan(from)?;
        let mut want: Vec<&str> = plan.iter().map(|c| c.oid.as_str()).collect();
        let mut got: Vec<&str> = steps.iter().map(|s| s.oid.as_str()).collect();
        want.sort_unstable();
        got.sort_unstable();
        if want != got {
            return Err(GitError::Invalid(
                "the rebase plan does not match the current branch; reload it".into(),
            ));
        }
        match steps.iter().find(|s| s.action != RebaseAction::Drop) {
            Some(first) if matches!(first.action, RebaseAction::Squash | RebaseAction::Fixup) => {
                return Err(GitError::Invalid(
                    "the first commit cannot be squashed: there is nothing before it".into(),
                ));
            }
            _ => {}
        }
        if steps.iter().all(|s| s.action == RebaseAction::Drop) {
            return Err(GitError::Invalid(
                "dropping every commit is not supported here; use reset".into(),
            ));
        }

        // Scratch files live in the git dir; the previous run's are removed.
        let scratch = self.git_dir.join("glidegit-rebase");
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch)?;
        let quote = |p: &Path| {
            let s = p.to_string_lossy().replace('\\', "/");
            format!("'{}'", s.replace('\'', "'\\''"))
        };
        let mut todo = String::new();
        for (i, s) in steps.iter().enumerate() {
            check_oid(&s.oid)?;
            let word = match s.action {
                RebaseAction::Pick | RebaseAction::Reword => "pick",
                RebaseAction::Edit => "edit",
                RebaseAction::Squash => "squash",
                RebaseAction::Fixup => "fixup",
                RebaseAction::Drop => "drop",
            };
            todo.push_str(&format!("{word} {}\n", s.oid));
            if s.action == RebaseAction::Reword {
                let msg = s.message.as_deref().map(str::trim).unwrap_or("");
                if msg.is_empty() {
                    return Err(GitError::Invalid(
                        "a reworded commit needs a message".into(),
                    ));
                }
                let file = scratch.join(format!("msg-{i}.txt"));
                std::fs::write(&file, format!("{msg}\n"))?;
                todo.push_str(&format!(
                    "exec git commit --amend --allow-empty --quiet --cleanup=strip -F {}\n",
                    quote(&file)
                ));
            }
        }
        let todo_file = scratch.join("todo");
        std::fs::write(&todo_file, todo)?;

        let mut args = vec!["rebase", "--interactive"];
        if autostash {
            args.push("--autostash");
        }
        let base = format!("{from}^");
        if self.has_parent(from) {
            args.push(&base);
        } else {
            args.push("--root");
        }
        self.cli.run(
            &self.root,
            &args,
            RunOptions::write()
                .env("GIT_SEQUENCE_EDITOR", &format!("cp {}", quote(&todo_file)))
                // Squash keeps git's combined message; nothing to edit.
                .env("GIT_EDITOR", "true"),
        )?;
        Ok(self.operation().is_none())
    }

    // ------------------------------------------------- operation log support

    /// HEAD plus all branches, tags and the stash, for before/after comparison.
    pub fn refs_state(&self) -> Result<RefsState> {
        let out = self.read(&[
            "for-each-ref",
            "--format=%(refname)%00%(objectname)",
            "refs/heads",
            "refs/tags",
            "refs/stash",
        ])?;
        let refs = out
            .lines()
            .filter_map(|l| l.split_once('\0'))
            .map(|(r, o)| (r.to_string(), o.to_string()))
            .collect();
        let head = std::fs::read_to_string(self.git_dir.join("HEAD")).unwrap_or_default();
        let head_ref = head
            .trim()
            .strip_prefix("ref: ")
            .map(|r| r.trim().to_string());
        let head_oid = self
            .read(&["rev-parse", "--verify", "-q", "HEAD"])
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| check_oid(s).is_ok());
        Ok(RefsState {
            head_ref,
            head_oid,
            refs,
        })
    }

    /// A commit holding the current index and working-tree changes (tracked
    /// files), without touching any ref or file: a backup before destructive
    /// operations. `None` when there is nothing to save.
    pub fn stash_create(&self) -> Result<Option<String>> {
        if !self.has_head() {
            return Ok(None);
        }
        let out = self.cli.run(
            &self.root,
            &["stash", "create"],
            RunOptions::write().env("GIT_LITERAL_PATHSPECS", "0"),
        )?;
        let oid = out.stdout_str().trim().to_string();
        Ok(check_oid(&oid).ok().map(|_| oid))
    }

    /// Re-apply a backup made by [`Repo::stash_create`] (or any stash commit).
    pub fn stash_apply_commit(&self, oid: &str) -> Result<()> {
        check_oid(oid)?;
        self.cli.run(
            &self.root,
            &["stash", "apply", "--index", oid],
            RunOptions::write().env("GIT_LITERAL_PATHSPECS", "0"),
        )?;
        Ok(())
    }

    /// Put a stash commit back on the stash list (undo of a drop / pop).
    pub fn stash_store(&self, oid: &str, message: &str) -> Result<()> {
        check_oid(oid)?;
        self.cli.run(
            &self.root,
            &["stash", "store", "-m", message, oid],
            RunOptions::write().env("GIT_LITERAL_PATHSPECS", "0"),
        )?;
        Ok(())
    }

    /// Point `name` at `oid`, but only if it currently is `expect` (`None`:
    /// must not exist). The compare-and-swap keeps undo from overwriting
    /// anything that moved since.
    pub fn update_ref_checked(&self, name: &str, oid: &str, expect: Option<&str>) -> Result<()> {
        check_ref_target(name)?;
        check_oid(oid)?;
        let zero = "0".repeat(oid.len());
        let old = expect.unwrap_or(&zero);
        check_oid(old)?;
        self.write(&["update-ref", "-m", "glidegit: undo", name, oid, old])
    }

    /// Delete `name` if it still points at `expect`.
    pub fn delete_ref_checked(&self, name: &str, expect: &str) -> Result<()> {
        check_ref_target(name)?;
        check_oid(expect)?;
        self.write(&["update-ref", "-d", name, expect])
    }

    /// Resolve a branch name, tag or commit id to a commit id.
    pub fn resolve_commit(&self, rev: &str) -> Result<String> {
        if rev.is_empty() || rev.starts_with('-') || rev.contains(char::is_whitespace) {
            return Err(GitError::Invalid(format!("invalid revision: {rev:?}")));
        }
        let out = self
            .read(&[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("{rev}^{{commit}}"),
            ])
            .map_err(|_| GitError::Invalid(format!("no such branch or commit: {rev}")))?;
        let oid = out.trim().to_string();
        check_oid(&oid)?;
        Ok(oid)
    }

    /// Compare two branches / commits: what each has that the other has not.
    pub fn compare(&self, left: &str, right: &str, limit: usize) -> Result<BranchCompare> {
        let l = self.resolve_commit(left)?;
        let r = self.resolve_commit(right)?;
        let merge_base = self
            .read(&["merge-base", &l, &r])
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| check_oid(s).is_ok());
        let counts = self.read(&["rev-list", "--left-right", "--count", &format!("{l}...{r}")])?;
        let mut it = counts
            .split_whitespace()
            .map(|n| n.parse::<u32>().unwrap_or(0));
        let (lc, rc) = (it.next().unwrap_or(0), it.next().unwrap_or(0));
        let n = format!("-n{limit}");
        let side = |range: String| -> Result<Vec<CommitSummary>> {
            Ok(parse::parse_log(&self.read(&[
                "log",
                parse::LOG_FORMAT,
                "--decorate=short",
                &n,
                &range,
                "--",
            ])?))
        };
        Ok(BranchCompare {
            only_left: side(format!("{r}..{l}"))?,
            only_right: side(format!("{l}..{r}"))?,
            only_left_count: lc,
            only_right_count: rc,
            left_oid: l,
            right_oid: r,
            merge_base,
        })
    }

    /// Files that differ between two commits (renames detected).
    pub fn diff_files(&self, from: &str, to: &str) -> Result<Vec<CommitFile>> {
        check_oid(from)?;
        check_oid(to)?;
        let out = self.cli.run(
            &self.root,
            &[
                "diff-tree",
                "-r",
                "-z",
                "-M",
                "--raw",
                "--numstat",
                from,
                to,
            ],
            RunOptions::read(),
        )?;
        Ok(parse::parse_diff_tree(&out.stdout))
    }

    /// Commits that changed `path`, newest first, following renames.
    /// `skip` / `limit` page through long histories.
    pub fn file_history(&self, path: &str, skip: u32, limit: u32) -> Result<Vec<FileRevision>> {
        self.resolve(path)?;
        // No --skip / --max-count: with --follow git counts commits that do
        // not touch the file too, so page here instead.
        let text = self.read(&[
            "log",
            "--follow",
            "-M",
            "--name-status",
            parse::HISTORY_FORMAT,
            "--decorate=short",
            "--",
            path,
        ])?;
        Ok(parse::parse_file_history(&text, path)
            .into_iter()
            .skip(skip as usize)
            .take(limit as usize)
            .collect())
    }

    /// Commits that changed lines `start..=end` (1-based) of `path` as it is
    /// at `rev` (default HEAD), newest first (`git log -L`).
    pub fn line_history(
        &self,
        path: &str,
        start: u32,
        end: u32,
        rev: Option<&str>,
    ) -> Result<Vec<CommitSummary>> {
        self.resolve(path)?;
        if start == 0 || end < start {
            return Err(GitError::Invalid(format!(
                "invalid line range {start}-{end}"
            )));
        }
        // The path travels inside the `-L` argument, so it is never an option.
        let range = format!("-L{start},{end}:{path}");
        let mut args = vec!["log", parse::HISTORY_FORMAT, "--decorate=short", &range];
        if let Some(r) = rev {
            check_oid(r)?;
            args.push(r);
        }
        Ok(parse::parse_line_history(&self.read(&args)?))
    }

    /// Who last changed each line of `path`: in the working copy (`rev`
    /// `None`; uncommitted lines get an all-zero id) or at a commit.
    pub fn blame(&self, path: &str, rev: Option<&str>) -> Result<Blame> {
        self.resolve(path)?;
        let mut args = vec!["blame", "--porcelain"];
        if let Some(r) = rev {
            check_oid(r)?;
            args.push(r);
        }
        args.extend(["--", path]);
        let out = self.cli.run(&self.root, &args, RunOptions::read())?;
        Ok(parse::parse_blame(&out.stdout, path))
    }

    /// Skip the commit the rebase / cherry-pick / revert stopped at.
    pub fn skip_operation(&self) -> Result<()> {
        let cmd = match self.operation() {
            Some(RepoOperation::Rebase) => "rebase",
            Some(RepoOperation::CherryPick) => "cherry-pick",
            Some(RepoOperation::Revert) => "revert",
            Some(RepoOperation::Merge) => {
                return Err(GitError::Invalid(
                    "a merge cannot skip; continue or abort it".into(),
                ))
            }
            None => return Ok(()),
        };
        self.write(&["-c", "core.editor=true", cmd, "--skip"])
    }

    pub fn branches(&self) -> Result<BranchList> {
        let text = self.read(&[
            "for-each-ref",
            "--sort=-committerdate",
            parse::FOR_EACH_REF_FORMAT,
            "refs/heads",
            "refs/remotes",
        ])?;
        let (local, remote) = parse::parse_for_each_ref(&text);
        // Reflog is missing on unborn branches; recent list is best-effort.
        let recent = self
            .read(&["reflog", "show", "--format=%gs", "-n", "500", "HEAD", "--"])
            .map(|r| parse::parse_recent_branches(&r, &local, 10))
            .unwrap_or_default();
        Ok(BranchList {
            local,
            remote,
            recent,
        })
    }

    pub fn snapshot(&self) -> Result<RepoSnapshot> {
        let (head, changes) = self.status()?;
        let branches = self.branches_cached()?;
        Ok(RepoSnapshot {
            root: self.root.to_string_lossy().into_owned(),
            head,
            operation: self.operation(),
            operation_info: self.operation_info(),
            changes,
            branches,
            stale: false,
        })
    }

    /// Most recent commits reachable from `rev` (default HEAD).
    pub fn log(&self, rev: Option<&str>, limit: usize) -> Result<Vec<CommitSummary>> {
        let n = format!("-n{limit}");
        let mut args = vec!["log", parse::LOG_FORMAT, "--decorate=short", &n];
        if let Some(r) = rev {
            args.push(r);
        }
        args.push("--");
        match self.read(&args) {
            Ok(text) => Ok(parse::parse_log(&text)),
            // Unborn branch: no commits yet.
            Err(GitError::Failed { message, .. })
                if message.contains("does not have any commits")
                    || message.contains("bad default revision") =>
            {
                Ok(Vec::new())
            }
            Err(e) => Err(e),
        }
    }

    pub fn last_commit_message(&self) -> Result<String> {
        Ok(self
            .read(&["log", "-1", "--format=%B"])?
            .trim_end()
            .to_string())
    }

    fn has_head(&self) -> bool {
        self.cli
            .run(
                &self.root,
                &["rev-parse", "--verify", "-q", "HEAD"],
                RunOptions::read(),
            )
            .is_ok()
    }

    pub fn diff_file(&self, path: &str, mode: DiffMode, untracked: bool) -> Result<FileDiff> {
        let unborn = mode != DiffMode::WorkingTree && !self.has_head();
        if untracked || (unborn && mode == DiffMode::Head) {
            return self.preview_new_file(path);
        }
        let mut args = vec![
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            "-U3",
        ];
        match mode {
            DiffMode::WorkingTree => {}
            DiffMode::Staged => args.push("--cached"),
            DiffMode::Head => args.push("HEAD"),
        }
        args.extend(["--", path]);
        let text = self.read(&args)?;
        Ok(parse::parse_unified_diff(&text, path, MAX_DIFF_LINES))
    }

    /// Message, people, dates and changed files of a commit. A merge is
    /// compared with its first parent, a root commit with the empty tree.
    pub fn commit_details(&self, oid: &str) -> Result<CommitDetails> {
        check_oid(oid)?;
        let text = self.read(&["log", "-1", parse::DETAILS_FORMAT, oid, "--"])?;
        let mut d = parse::parse_details(&text)
            .ok_or_else(|| GitError::Invalid(format!("no such commit: {oid}")))?;
        let mut args = vec![
            "diff-tree",
            "-r",
            "-z",
            "-M",
            "--raw",
            "--numstat",
            "--no-commit-id",
        ];
        let parent = d.parents.first().cloned();
        match &parent {
            Some(p) => args.push(p),
            None => args.push("--root"),
        }
        args.push(oid);
        let out = self.cli.run(&self.root, &args, RunOptions::read())?;
        d.files = parse::parse_diff_tree(&out.stdout);
        Ok(d)
    }

    /// Diff of one file in a commit against `parent` (its first parent, or
    /// `None` for a root commit). `orig_path` is the old name of a rename.
    pub fn commit_file_diff(
        &self,
        oid: &str,
        parent: Option<&str>,
        path: &str,
        orig_path: Option<&str>,
    ) -> Result<FileDiff> {
        check_oid(oid)?;
        let mut args = vec![
            "diff-tree",
            "-p",
            "-M",
            "--no-color",
            "--no-ext-diff",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            "-U3",
        ];
        match parent {
            Some(p) => {
                check_oid(p)?;
                args.push(p);
            }
            None => args.push("--root"),
        }
        args.extend([oid, "--", path]);
        // Both names, so rename detection can pair them.
        if let Some(o) = orig_path {
            args.push(o);
        }
        let text = self.read(&args)?;
        Ok(parse::parse_unified_diff(&text, path, MAX_DIFF_LINES))
    }

    /// Both versions of a working-copy file for the diff editor, compared
    /// as in [`Repo::diff_file`]. `orig_path` is the old name of a rename.
    pub fn file_texts(
        &self,
        path: &str,
        orig_path: Option<&str>,
        mode: DiffMode,
        untracked: bool,
    ) -> Result<FileTexts> {
        let before = orig_path.unwrap_or(path);
        let (old, new) = if untracked {
            (Side::Missing, Side::Worktree(path))
        } else {
            match mode {
                DiffMode::WorkingTree => (Side::Index(path), Side::Worktree(path)),
                DiffMode::Staged => (Side::Rev("HEAD", before), Side::Index(path)),
                DiffMode::Head => (Side::Rev("HEAD", before), Side::Worktree(path)),
            }
        };
        let mut texts = self.texts(path, old, new)?;
        // HEAD ↔ worktree is the view with stage checkboxes: add the index.
        if mode == DiffMode::Head && !texts.binary && !texts.too_large {
            if let Blob::Data(d) = self.read_side(&Side::Index(path))? {
                texts.lossy |= is_lossy(&d);
                texts.index = Some(String::from_utf8_lossy(&d).into_owned());
            }
        }
        Ok(texts)
    }

    /// Set the staged content of `path` to `content` (blocks or lines staged
    /// from the diff editor). The content is in working-tree form; git's
    /// clean filters (e.g. CRLF conversion) apply as for `git add`.
    pub fn stage_content(&self, path: &str, content: &str) -> Result<()> {
        let full = self.resolve(path)?;
        let mode = self.index_mode(path, &full)?;
        let oid = self
            .cli
            .run(
                &self.root,
                &["hash-object", "-w", "--stdin", &format!("--path={path}")],
                RunOptions::write().stdin(content.as_bytes().to_vec()),
            )?
            .stdout_str()
            .trim()
            .to_string();
        check_oid(&oid)?;
        self.write(&[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("{mode},{oid},{path}"),
        ])
    }

    /// File mode for an index entry: the current entry's, else HEAD's, else
    /// from the working-tree file.
    fn index_mode(&self, path: &str, full: &Path) -> Result<String> {
        for args in [
            vec!["ls-files", "-s", "--", path],
            vec!["ls-tree", "HEAD", "--", path],
        ] {
            if let Ok(out) = self.cli.run(&self.root, &args, RunOptions::read()) {
                if let Some(mode) = out.stdout_str().split_whitespace().next() {
                    if mode.len() == 6 && mode.bytes().all(|b| b.is_ascii_digit()) {
                        return Ok(mode.to_string());
                    }
                }
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if std::fs::metadata(full).is_ok_and(|m| m.permissions().mode() & 0o111 != 0) {
                return Ok("100755".into());
            }
        }
        let _ = full;
        Ok("100644".into())
    }

    /// Replace a working-tree file's content (discarding blocks from the diff
    /// editor), but only if it still holds `expected`: never overwrite edits
    /// made since the diff was loaded.
    pub fn write_worktree_file(&self, path: &str, content: &str, expected: &str) -> Result<()> {
        let full = self.resolve(path)?;
        let current = std::fs::read(&full)?;
        if current != expected.as_bytes() {
            return Err(GitError::Invalid(format!(
                "{path} changed on disk since the diff was loaded; reload and try again"
            )));
        }
        std::fs::write(&full, content)?;
        Ok(())
    }

    /// Both versions of a file in a commit (against `parent`, or nothing for
    /// a root commit).
    pub fn commit_file_texts(
        &self,
        oid: &str,
        parent: Option<&str>,
        path: &str,
        orig_path: Option<&str>,
    ) -> Result<FileTexts> {
        check_oid(oid)?;
        let old = match parent {
            Some(p) => {
                check_oid(p)?;
                Side::Rev(p, orig_path.unwrap_or(path))
            }
            None => Side::Missing,
        };
        self.texts(path, old, Side::Rev(oid, path))
    }

    fn texts(&self, path: &str, old: Side, new: Side) -> Result<FileTexts> {
        let mut out = FileTexts {
            path: path.to_string(),
            ..FileTexts::default()
        };
        let a = self.read_side(&old)?;
        let b = self.read_side(&new)?;
        if matches!(a, Blob::TooLarge) || matches!(b, Blob::TooLarge) {
            out.too_large = true;
            return Ok(out);
        }
        let (a, b) = (a.into_bytes(), b.into_bytes());
        // Same heuristic as git: a NUL in the first 8000 bytes means binary.
        let binary = |d: &Option<Vec<u8>>| {
            d.as_ref()
                .is_some_and(|d| d[..d.len().min(8000)].contains(&0))
        };
        if binary(&a) || binary(&b) {
            out.binary = true;
            return Ok(out);
        }
        out.lossy = a.as_deref().is_some_and(is_lossy) || b.as_deref().is_some_and(is_lossy);
        let text = |d: Option<Vec<u8>>| d.map(|d| String::from_utf8_lossy(&d).into_owned());
        out.old = text(a);
        out.new = text(b);
        Ok(out)
    }

    fn read_side(&self, side: &Side) -> Result<Blob> {
        match side {
            Side::Missing => Ok(Blob::Missing),
            Side::Worktree(p) => {
                let full = self.resolve(p)?;
                match std::fs::metadata(&full) {
                    Ok(m) if m.is_file() && m.len() > MAX_PREVIEW_BYTES => Ok(Blob::TooLarge),
                    Ok(m) if m.is_file() => Ok(Blob::Data(std::fs::read(&full)?)),
                    _ => Ok(Blob::Missing),
                }
            }
            Side::Index(p) => self.read_blob(&format!(":{p}")),
            Side::Rev(rev, p) => self.read_blob(&format!("{rev}:{p}")),
        }
    }

    /// Read `rev:path` with `git cat-file --batch`, stopping as soon as the
    /// header says the blob is over the size limit.
    fn read_blob(&self, spec: &str) -> Result<Blob> {
        if spec.contains('\n') {
            return Err(GitError::Invalid(format!("invalid path: {spec:?}")));
        }
        let mut buf: Vec<u8> = Vec::new();
        let mut header: Option<Option<usize>> = None; // Some(None) = missing
        self.cli.run_streaming(
            &self.root,
            &["cat-file", "--batch"],
            RunOptions::read().stdin(format!("{spec}\n")),
            |chunk| {
                buf.extend_from_slice(chunk);
                if header.is_none() {
                    let Some(nl) = buf.iter().position(|&b| b == b'\n') else {
                        return true;
                    };
                    let line = String::from_utf8_lossy(&buf[..nl]).into_owned();
                    buf.drain(..=nl);
                    // "<oid> blob <size>", or "<spec> missing" / "ambiguous".
                    let mut f = line.split(' ');
                    let size = match (f.next(), f.next(), f.next()) {
                        (Some(_), Some("blob"), Some(n)) => n.parse::<usize>().ok(),
                        _ => None,
                    };
                    header = Some(size);
                    if size.is_none_or(|n| n as u64 > MAX_PREVIEW_BYTES) {
                        return false;
                    }
                }
                true
            },
        )?;
        Ok(match header {
            Some(Some(n)) if n as u64 > MAX_PREVIEW_BYTES => Blob::TooLarge,
            Some(Some(n)) => {
                buf.truncate(n);
                Blob::Data(buf)
            }
            _ => Blob::Missing,
        })
    }

    /// Base, ours and theirs of a conflicted file, from the index stages.
    pub fn conflict_texts(&self, path: &str) -> Result<ConflictTexts> {
        self.resolve(path)?;
        let mut out = ConflictTexts {
            path: path.to_string(),
            ..ConflictTexts::default()
        };
        (out.ours_label, out.theirs_label) = self.conflict_labels();
        let blobs = [
            self.read_blob(&format!(":1:{path}"))?,
            self.read_blob(&format!(":2:{path}"))?,
            self.read_blob(&format!(":3:{path}"))?,
        ];
        if blobs.iter().any(|b| matches!(b, Blob::TooLarge)) {
            out.too_large = true;
            return Ok(out);
        }
        let [base, ours, theirs] = blobs.map(Blob::into_bytes);
        let all = [&base, &ours, &theirs];
        if all.iter().any(|d| {
            d.as_ref()
                .is_some_and(|d| d[..d.len().min(8000)].contains(&0))
        }) {
            out.binary = true;
            return Ok(out);
        }
        out.lossy = all.iter().any(|d| d.as_deref().is_some_and(is_lossy));
        let text = |d: Option<Vec<u8>>| d.map(|d| String::from_utf8_lossy(&d).into_owned());
        (out.base, out.ours, out.theirs) = (text(base), text(ours), text(theirs));
        Ok(out)
    }

    /// Names for the two sides of the operation in progress. During a rebase
    /// "ours" is the branch being rebased onto and "theirs" the commit being
    /// replayed, which is the opposite of what people expect, so say so.
    fn conflict_labels(&self) -> (String, String) {
        let describe = |rev: &str| -> Option<String> {
            let out = self
                .read(&["log", "-1", "--format=%h %s", rev, "--"])
                .ok()?;
            let s = out.trim();
            (!s.is_empty()).then(|| s.to_string())
        };
        let head = self
            .status()
            .ok()
            .and_then(|(h, _)| h.branch)
            .or_else(|| describe("HEAD"))
            .unwrap_or_else(|| "HEAD".into());
        let theirs_of = |file: &str| {
            std::fs::read_to_string(self.git_dir.join(file))
                .ok()
                .and_then(|oid| {
                    let oid = oid.lines().next()?.trim().to_string();
                    check_oid(&oid).ok()?;
                    describe(&oid)
                })
        };
        match self.operation() {
            Some(RepoOperation::Rebase) => (
                format!(
                    "变基目标 {}",
                    describe("HEAD").unwrap_or_else(|| "HEAD".into())
                ),
                theirs_of("REBASE_HEAD").unwrap_or_else(|| "正在应用的提交".into()),
            ),
            Some(RepoOperation::Merge) => (
                head,
                theirs_of("MERGE_HEAD").unwrap_or_else(|| "合并进来的分支".into()),
            ),
            Some(RepoOperation::CherryPick) => (
                head,
                theirs_of("CHERRY_PICK_HEAD").unwrap_or_else(|| "cherry-pick 的提交".into()),
            ),
            Some(RepoOperation::Revert) => (
                head,
                theirs_of("REVERT_HEAD")
                    .map(|s| format!("撤销 {s}"))
                    .unwrap_or_else(|| "revert".into()),
            ),
            // e.g. a conflicting `git stash apply`
            None => (head, "他们的".into()),
        }
    }

    /// Mark a conflicted file resolved: write `content` to it and stage it,
    /// or (with `None`) resolve by deleting the file.
    pub fn resolve_conflict(&self, path: &str, content: Option<&str>) -> Result<()> {
        let full = self.resolve(path)?;
        match content {
            Some(c) => {
                std::fs::write(&full, c)?;
                self.write_with_paths(&["add", "-A"], &[path.to_string()])
            }
            None => self.write_with_paths(&["rm", "-q", "--ignore-unmatch"], &[path.to_string()]),
        }
    }

    /// Resolve a conflicted file by taking one side as a whole (deleting it
    /// if that side deleted it).
    pub fn resolve_with_side(&self, path: &str, side: ConflictSide) -> Result<()> {
        self.resolve(path)?;
        let stage = match side {
            ConflictSide::Ours => ":2:",
            ConflictSide::Theirs => ":3:",
        };
        if matches!(self.read_blob(&format!("{stage}{path}"))?, Blob::Missing) {
            return self.resolve_conflict(path, None);
        }
        let flag = match side {
            ConflictSide::Ours => "--ours",
            ConflictSide::Theirs => "--theirs",
        };
        self.write_with_paths(&["checkout", flag], &[path.to_string()])?;
        self.write_with_paths(&["add", "-A"], &[path.to_string()])
    }

    /// Join a repo-relative path, rejecting anything that could escape the worktree.
    fn resolve(&self, rel: &str) -> Result<PathBuf> {
        let p = Path::new(rel);
        let safe = p.components().all(|c| {
            matches!(
                c,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        });
        if !safe || rel.is_empty() {
            return Err(GitError::Invalid(format!("invalid repository path: {rel}")));
        }
        Ok(self.root.join(p))
    }

    fn preview_new_file(&self, path: &str) -> Result<FileDiff> {
        let full = self.resolve(path)?;
        let meta = match std::fs::metadata(&full) {
            Ok(m) => m,
            Err(_) => {
                return Ok(FileDiff {
                    path: path.to_string(),
                    ..FileDiff::default()
                })
            }
        };
        if meta.is_dir() {
            return Ok(FileDiff {
                path: path.to_string(),
                ..FileDiff::default()
            });
        }
        if meta.len() > MAX_PREVIEW_BYTES {
            return Ok(FileDiff {
                path: path.to_string(),
                truncated: true,
                ..FileDiff::default()
            });
        }
        let bytes = std::fs::read(&full)?;
        Ok(parse::synthesize_added(path, &bytes, MAX_DIFF_LINES))
    }

    // ---------------------------------------------------------------- writes

    pub fn stage(&self, paths: &[String]) -> Result<()> {
        self.write_with_paths(&["add", "-A"], paths)
    }

    pub fn unstage(&self, paths: &[String]) -> Result<()> {
        if self.has_head() {
            self.write_with_paths(&["reset", "-q", "HEAD"], paths)
        } else {
            self.write_with_paths(&["rm", "--cached", "-r", "-q", "--ignore-unmatch"], paths)
        }
    }

    pub fn stage_all(&self) -> Result<()> {
        self.write(&["add", "-A"])
    }

    /// Revert tracked paths to HEAD (index and worktree) and delete untracked ones.
    /// This is IDEA's "Rollback".
    pub fn rollback(&self, tracked: &[String], untracked: &[String]) -> Result<()> {
        if !tracked.is_empty() {
            if self.has_head() {
                self.write_with_paths(
                    &["restore", "--source=HEAD", "--staged", "--worktree"],
                    tracked,
                )?;
            } else {
                // Nothing committed yet: dropping from the index is the only rollback.
                self.write_with_paths(
                    &["rm", "--cached", "-r", "-q", "--ignore-unmatch"],
                    tracked,
                )?;
            }
        }
        for p in untracked {
            let full = self.resolve(p)?;
            if full.is_dir() {
                std::fs::remove_dir_all(&full)?;
            } else if full.exists() {
                std::fs::remove_file(&full)?;
            }
        }
        Ok(())
    }

    /// Commit what is staged. Returns the new HEAD commit id.
    pub fn commit(&self, message: &str, amend: bool) -> Result<String> {
        if message.trim().is_empty() {
            return Err(GitError::Invalid("commit message is empty".into()));
        }
        let mut args = vec!["commit", "-F", "-"];
        if amend {
            args.push("--amend");
        }
        self.cli.run(
            &self.root,
            &args,
            RunOptions::write().stdin(message.as_bytes().to_vec()),
        )?;
        Ok(self.read(&["rev-parse", "HEAD"])?.trim().to_string())
    }

    /// Check out a local branch, or create a tracking branch for a remote one.
    pub fn checkout_branch(&self, name: &str, remote: bool) -> Result<()> {
        if !remote {
            // Ref names can never start with '-', so this cannot be read as an option.
            return self.write(&["switch", name]);
        }
        let local = name.split_once('/').map(|(_, b)| b).unwrap_or(name);
        let exists = self
            .cli
            .run(
                &self.root,
                &[
                    "rev-parse",
                    "--verify",
                    "-q",
                    &format!("refs/heads/{local}"),
                ],
                RunOptions::read(),
            )
            .is_ok();
        if exists {
            self.write(&["switch", local])
        } else {
            self.write(&["switch", "-c", local, "--track", name])
        }
    }

    pub fn create_branch(&self, name: &str, start: Option<&str>, checkout: bool) -> Result<()> {
        check_ref_name(name)?;
        if let Some(s) = start {
            if s.starts_with('-') {
                return Err(GitError::Invalid(format!("invalid start point: {s:?}")));
            }
        }
        let mut args = if checkout {
            vec!["switch", "-c", name]
        } else {
            vec!["branch", name]
        };
        if let Some(s) = start {
            args.push(s);
        }
        self.write(&args)
    }

    /// Check out a commit without a branch (detached HEAD).
    pub fn checkout_detached(&self, oid: &str) -> Result<()> {
        check_oid(oid)?;
        self.write(&["switch", "--detach", oid])
    }

    /// Create a tag at `oid`: annotated when `message` is given, else lightweight.
    pub fn create_tag(&self, name: &str, oid: &str, message: Option<&str>) -> Result<()> {
        check_ref_name(name)?;
        check_oid(oid)?;
        match message.map(str::trim).filter(|m| !m.is_empty()) {
            Some(m) => {
                self.cli.run(
                    &self.root,
                    &["tag", "-a", name, oid, "-F", "-"],
                    RunOptions::write().stdin(m.as_bytes().to_vec()),
                )?;
                Ok(())
            }
            None => self.write(&["tag", name, oid]),
        }
    }

    fn is_merge(&self, oid: &str) -> bool {
        self.cli
            .run(
                &self.root,
                &["rev-parse", "--verify", "-q", &format!("{oid}^2")],
                RunOptions::read(),
            )
            .is_ok()
    }

    /// Apply a commit on top of HEAD. A merge commit is applied relative to
    /// its first parent. On conflicts git stops; see [`Repo::operation`].
    pub fn cherry_pick(&self, oid: &str) -> Result<()> {
        check_oid(oid)?;
        if self.is_merge(oid) {
            self.write(&["cherry-pick", "-m", "1", oid])
        } else {
            self.write(&["cherry-pick", oid])
        }
    }

    /// Commit the inverse of a commit, with git's default message.
    pub fn revert(&self, oid: &str) -> Result<()> {
        check_oid(oid)?;
        if self.is_merge(oid) {
            self.write(&["revert", "--no-edit", "-m", "1", oid])
        } else {
            self.write(&["revert", "--no-edit", oid])
        }
    }

    /// Move the current branch (or detached HEAD) to `oid`.
    pub fn reset(&self, oid: &str, mode: ResetMode) -> Result<()> {
        check_oid(oid)?;
        let flag = match mode {
            ResetMode::Soft => "--soft",
            ResetMode::Mixed => "--mixed",
            ResetMode::Hard => "--hard",
            ResetMode::Keep => "--keep",
        };
        self.write(&["reset", "-q", flag, oid])
    }

    /// Abort the merge, rebase, cherry-pick or revert in progress.
    pub fn abort_operation(&self) -> Result<()> {
        let cmd = match self.operation() {
            Some(RepoOperation::Merge) => "merge",
            Some(RepoOperation::Rebase) => "rebase",
            Some(RepoOperation::CherryPick) => "cherry-pick",
            Some(RepoOperation::Revert) => "revert",
            None => return Ok(()),
        };
        self.write(&[cmd, "--abort"])
    }

    /// Continue the operation in progress once conflicts are resolved and
    /// staged, keeping the prepared commit message (no editor is opened).
    pub fn continue_operation(&self) -> Result<()> {
        let cmd = match self.operation() {
            Some(RepoOperation::Merge) => "merge",
            Some(RepoOperation::Rebase) => "rebase",
            Some(RepoOperation::CherryPick) => "cherry-pick",
            Some(RepoOperation::Revert) => "revert",
            None => return Ok(()),
        };
        self.write(&[
            "-c",
            "core.editor=true",
            "-c",
            "sequence.editor=true",
            cmd,
            "--continue",
        ])
    }

    // ----------------------------------------------------------------- stash

    pub fn stash_list(&self) -> Result<Vec<StashEntry>> {
        Ok(parse::parse_stash_list(&self.read(&[
            "stash",
            "list",
            parse::STASH_FORMAT,
        ])?))
    }

    /// `git stash push`. Fails (and changes nothing) when there is nothing to stash.
    pub fn stash_push(&self, opts: &StashOptions) -> Result<()> {
        let (has_head, changes) = (self.has_head(), self.status()?.1);
        if !has_head {
            return Err(GitError::Invalid(
                "cannot stash before the first commit".into(),
            ));
        }
        let relevant = changes
            .iter()
            .any(|c| opts.include_untracked || c.unstaged != Some(ChangeKind::Untracked));
        if !relevant {
            return Err(GitError::Invalid("no local changes to stash".into()));
        }
        let mut args = vec!["stash", "push"];
        if opts.include_untracked {
            args.push("--include-untracked");
        }
        if opts.keep_index {
            args.push("--keep-index");
        }
        let message = opts
            .message
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty());
        let m_arg;
        if let Some(m) = message {
            m_arg = format!("--message={m}");
            args.push(&m_arg);
        }
        // git stash relies on magic pathspecs (":/") internally, so literal
        // pathspecs must be off; user paths are marked literal one by one.
        let mut input = Vec::new();
        if !opts.paths.is_empty() {
            args.extend(["--pathspec-from-file=-", "--pathspec-file-nul"]);
            for p in &opts.paths {
                input.extend_from_slice(format!(":(literal){p}").as_bytes());
                input.push(0);
            }
        }
        let mut run = RunOptions::write().env("GIT_LITERAL_PATHSPECS", "0");
        if !input.is_empty() {
            run = run.stdin(input);
        }
        self.cli.run(&self.root, &args, run)?;
        Ok(())
    }

    /// Run a `git stash` subcommand (needs non-literal pathspecs, see above).
    fn stash_cmd(&self, args: &[&str]) -> Result<()> {
        self.cli.run(
            &self.root,
            args,
            RunOptions::write().env("GIT_LITERAL_PATHSPECS", "0"),
        )?;
        Ok(())
    }

    /// Make sure `stash@{index}` still is `oid` (the list may have changed
    /// since the UI read it), and return the ref.
    fn stash_ref(&self, index: u32, oid: &str) -> Result<String> {
        check_oid(oid)?;
        let r = format!("stash@{{{index}}}");
        let actual = self
            .read(&["rev-parse", "--verify", "-q", &r])
            .unwrap_or_default();
        if actual.trim() != oid {
            return Err(GitError::Invalid(
                "the stash list changed; refresh and try again".into(),
            ));
        }
        Ok(r)
    }

    /// Apply a stash; with `pop`, drop it afterwards (git keeps it if the
    /// apply had conflicts). `restore_index` also restores what was staged.
    pub fn stash_apply(&self, index: u32, oid: &str, pop: bool, restore_index: bool) -> Result<()> {
        let r = self.stash_ref(index, oid)?;
        let mut args = vec!["stash", if pop { "pop" } else { "apply" }];
        if restore_index {
            args.push("--index");
        }
        args.push(&r);
        self.stash_cmd(&args)
    }

    pub fn stash_drop(&self, index: u32, oid: &str) -> Result<()> {
        let r = self.stash_ref(index, oid)?;
        self.stash_cmd(&["stash", "drop", "-q", &r])
    }

    /// What a stash contains: changes against the commit it was made on,
    /// plus untracked files (kind `Untracked`) when it has them.
    pub fn stash_details(&self, oid: &str) -> Result<CommitDetails> {
        let mut d = self.commit_details(oid)?;
        if let Some(untracked) = d.parents.get(2).cloned() {
            check_oid(&untracked)?;
            let out = self.cli.run(
                &self.root,
                &["ls-tree", "-r", "-z", "--name-only", &untracked],
                RunOptions::read(),
            )?;
            for path in out.stdout_str().split('\0').filter(|p| !p.is_empty()) {
                d.files.push(CommitFile {
                    path: path.to_string(),
                    orig_path: None,
                    kind: ChangeKind::Untracked,
                    added: None,
                    deleted: None,
                });
            }
        }
        Ok(d)
    }

    pub fn delete_branch(&self, name: &str, force: bool) -> Result<()> {
        self.write(&["branch", if force { "-D" } else { "-d" }, name])
    }

    pub fn remotes(&self) -> Result<Vec<String>> {
        Ok(self
            .read(&["remote"])?
            .lines()
            .map(str::to_string)
            .collect())
    }

    // --------------------------------------------------------------- network

    fn net_options(net: &NetOptions) -> RunOptions {
        RunOptions {
            cancel: net.cancel.clone(),
            progress: net.progress.clone(),
            ..RunOptions::write()
        }
    }

    /// The remote a new branch is pushed to: `origin` if present, else the first one.
    pub fn default_remote(&self) -> Result<Option<String>> {
        let remotes = self.remotes()?;
        Ok(if remotes.iter().any(|r| r == "origin") {
            Some("origin".to_string())
        } else {
            remotes.into_iter().next()
        })
    }

    pub fn fetch(&self, net: &NetOptions) -> Result<()> {
        self.cli.run(
            &self.root,
            &["fetch", "--all", "--prune", "--progress"],
            Self::net_options(net),
        )?;
        Ok(())
    }

    pub fn pull(&self, mode: PullMode, net: &NetOptions) -> Result<()> {
        let flag = match mode {
            PullMode::Merge => "--no-rebase",
            PullMode::Rebase => "--rebase",
            PullMode::FastForwardOnly => "--ff-only",
        };
        self.cli.run(
            &self.root,
            &["pull", "--no-edit", "--progress", flag],
            Self::net_options(net),
        )?;
        Ok(())
    }

    /// What a push of the current branch would send.
    pub fn push_preview(&self, limit: usize) -> Result<PushPreview> {
        let (head, _) = self.status()?;
        let Some(branch) = head.branch.clone() else {
            return Ok(PushPreview {
                branch: None,
                remote: None,
                target: None,
                new_branch: false,
                commits: Vec::new(),
                behind: 0,
            });
        };
        if let Some(upstream) = head.upstream.clone() {
            let range = format!("{upstream}..HEAD");
            let commits = if head.oid.is_some() {
                self.log(Some(&range), limit)?
            } else {
                Vec::new()
            };
            let remote = upstream.split_once('/').map(|(r, _)| r.to_string());
            return Ok(PushPreview {
                branch: Some(branch),
                remote,
                target: Some(upstream),
                new_branch: false,
                commits,
                behind: head.behind,
            });
        }
        let remote = self.default_remote()?;
        // No upstream yet: everything not already on some remote will be sent.
        let commits = if head.oid.is_some() {
            let n = format!("-n{limit}");
            let text = self.read(&[
                "log",
                crate::parse::LOG_FORMAT,
                "--decorate=short",
                &n,
                "HEAD",
                "--not",
                "--remotes",
                "--",
            ])?;
            crate::parse::parse_log(&text)
        } else {
            Vec::new()
        };
        Ok(PushPreview {
            target: remote.as_ref().map(|r| format!("{r}/{branch}")),
            branch: Some(branch),
            remote,
            new_branch: true,
            commits,
            behind: 0,
        })
    }

    /// Push the current branch; sets upstream on first push.
    pub fn push(&self, force_with_lease: bool, net: &NetOptions) -> Result<()> {
        let (head, _) = self.status()?;
        let branch = head
            .branch
            .ok_or_else(|| GitError::Invalid("cannot push a detached HEAD".into()))?;
        let mut args: Vec<String> = vec!["push".into(), "--progress".into()];
        if force_with_lease {
            args.push("--force-with-lease".into());
        }
        if head.upstream.is_none() {
            let remote = self
                .default_remote()?
                .ok_or_else(|| GitError::Invalid("no remote configured".into()))?;
            args.extend(["--set-upstream".into(), remote, branch]);
        }
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        self.cli.run(&self.root, &refs, Self::net_options(net))?;
        Ok(())
    }
}

/// Cancellation and progress reporting for network operations.
#[derive(Clone, Debug, Default)]
pub struct NetOptions {
    pub cancel: Option<CancelToken>,
    pub progress: Option<ProgressSink>,
}
