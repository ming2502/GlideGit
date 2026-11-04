//! Repository-level operations. All methods are blocking; callers on an async
//! runtime should use `spawn_blocking` (see `glide-core`).

use std::path::{Path, PathBuf};

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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullMode {
    Merge,
    Rebase,
    FastForwardOnly,
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
        Ok(Self {
            cli,
            root: PathBuf::from(root),
            git_dir: PathBuf::from(git_dir),
        })
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
        let branches = self.branches()?;
        Ok(RepoSnapshot {
            root: self.root.to_string_lossy().into_owned(),
            head,
            operation: self.operation(),
            changes,
            branches,
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
