//! Integration tests against real repositories in temp dirs.

use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use glide_git_backend::*;

/// A git CLI isolated from the developer's global/system config.
fn cli() -> GitCli {
    GitCli::discover()
        .unwrap()
        .with_env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .with_env("GIT_CONFIG_NOSYSTEM", "1")
        .with_env("GIT_AUTHOR_NAME", "Test")
        .with_env("GIT_AUTHOR_EMAIL", "test@example.com")
        .with_env("GIT_COMMITTER_NAME", "Test")
        .with_env("GIT_COMMITTER_EMAIL", "test@example.com")
}

fn new_repo() -> (tempfile::TempDir, Repo) {
    let dir = tempfile::tempdir().unwrap();
    let c = cli();
    c.run(
        dir.path(),
        &["init", "-q", "-b", "main"],
        RunOptions::write(),
    )
    .unwrap();
    let repo = Repo::open(c, dir.path()).unwrap();
    (dir, repo)
}

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(p, content).unwrap();
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

fn find<'a>(changes: &'a [FileChange], path: &str) -> &'a FileChange {
    changes
        .iter()
        .find(|c| c.path == path)
        .unwrap_or_else(|| panic!("{path} not in {changes:?}"))
}

#[test]
fn open_rejects_non_repo() {
    let dir = tempfile::tempdir().unwrap();
    let err = Repo::open(cli(), dir.path()).unwrap_err();
    assert!(matches!(err, GitError::NotARepository(_)), "{err:?}");
}

#[test]
fn open_from_subdirectory_finds_root() {
    let (dir, _) = new_repo();
    fs::create_dir_all(dir.path().join("a/b")).unwrap();
    let repo = Repo::open(cli(), dir.path().join("a/b")).unwrap();
    assert_eq!(
        fs::canonicalize(repo.root()).unwrap(),
        fs::canonicalize(dir.path()).unwrap()
    );
}

#[test]
fn unborn_repo_stage_commit_flow() {
    let (dir, repo) = new_repo();
    write(dir.path(), "hello.txt", "hi\n");
    write(dir.path(), "dir with space/新文件.md", "x\n");

    let snap = repo.snapshot().unwrap();
    assert_eq!(snap.head.branch.as_deref(), Some("main"));
    assert!(snap.head.oid.is_none());
    assert_eq!(snap.changes.len(), 2);
    assert!(find(&snap.changes, "dir with space/新文件.md").is_untracked());
    assert!(repo.log(None, 10).unwrap().is_empty());

    // Diff preview of an untracked file and of a new file in an unborn repo.
    let d = repo.diff_file("hello.txt", DiffMode::Head, true).unwrap();
    assert_eq!(d.added, 1);

    repo.stage(&s(&["hello.txt"])).unwrap();
    let (_, ch) = repo.status().unwrap();
    assert_eq!(find(&ch, "hello.txt").staged, Some(ChangeKind::Added));
    let d = repo.diff_file("hello.txt", DiffMode::Head, false).unwrap();
    assert_eq!(d.added, 1);

    // Unstage works without HEAD.
    repo.unstage(&s(&["hello.txt"])).unwrap();
    let (_, ch) = repo.status().unwrap();
    assert!(find(&ch, "hello.txt").is_untracked());

    repo.stage(&s(&["hello.txt"])).unwrap();
    assert!(repo.commit("   ", false).is_err());
    let oid = repo.commit("first commit\n\nbody", false).unwrap();
    assert_eq!(oid.len(), 40);

    let log = repo.log(None, 10).unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].subject, "first commit");
    assert!(log[0].refs.iter().any(|r| r == "HEAD -> main"));
    assert_eq!(repo.last_commit_message().unwrap(), "first commit\n\nbody");
}

fn repo_with_commit() -> (tempfile::TempDir, Repo) {
    let (dir, repo) = new_repo();
    write(dir.path(), "a.txt", "one\ntwo\nthree\n");
    write(dir.path(), "b.txt", "bee\n");
    repo.stage_all().unwrap();
    repo.commit("init", false).unwrap();
    (dir, repo)
}

#[test]
fn partial_staging_and_diff_modes() {
    let (dir, repo) = repo_with_commit();
    write(dir.path(), "a.txt", "one\nTWO\nthree\n");
    repo.stage(&s(&["a.txt"])).unwrap();
    write(dir.path(), "a.txt", "one\nTWO\nthree\nfour\n");

    let (_, ch) = repo.status().unwrap();
    let a = find(&ch, "a.txt");
    assert_eq!(a.staged, Some(ChangeKind::Modified));
    assert_eq!(a.unstaged, Some(ChangeKind::Modified));

    let staged = repo.diff_file("a.txt", DiffMode::Staged, false).unwrap();
    assert_eq!((staged.added, staged.deleted), (1, 1));
    let wt = repo
        .diff_file("a.txt", DiffMode::WorkingTree, false)
        .unwrap();
    assert_eq!((wt.added, wt.deleted), (1, 0));
    let head = repo.diff_file("a.txt", DiffMode::Head, false).unwrap();
    assert_eq!((head.added, head.deleted), (2, 1));
}

#[test]
fn rename_is_detected() {
    let (_dir, repo) = repo_with_commit();
    repo.cli()
        .run(repo.root(), &["mv", "b.txt", "c.txt"], RunOptions::write())
        .unwrap();
    let (_, ch) = repo.status().unwrap();
    let c = find(&ch, "c.txt");
    assert_eq!(c.staged, Some(ChangeKind::Renamed));
    assert_eq!(c.orig_path.as_deref(), Some("b.txt"));
}

#[test]
fn rollback_tracked_and_untracked() {
    let (dir, repo) = repo_with_commit();
    write(dir.path(), "a.txt", "changed\n");
    repo.stage(&s(&["a.txt"])).unwrap();
    write(dir.path(), "new.txt", "n\n");
    repo.rollback(&s(&["a.txt"]), &s(&["new.txt"])).unwrap();
    let (_, ch) = repo.status().unwrap();
    assert!(ch.is_empty(), "{ch:?}");
    assert_eq!(
        fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "one\ntwo\nthree\n"
    );
    assert!(repo.rollback(&[], &s(&["../escape"])).is_err());
}

#[test]
fn amend_rewrites_last_commit() {
    let (dir, repo) = repo_with_commit();
    write(dir.path(), "b.txt", "bee2\n");
    repo.stage_all().unwrap();
    repo.commit("init (amended)", true).unwrap();
    let log = repo.log(None, 10).unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].subject, "init (amended)");
}

#[test]
fn branches_checkout_and_recent() {
    let (_dir, repo) = repo_with_commit();
    repo.create_branch("feature/x", None, true).unwrap();
    repo.create_branch("other", None, false).unwrap();
    repo.checkout_branch("main", false).unwrap();
    repo.checkout_branch("feature/x", false).unwrap();

    let b = repo.branches().unwrap();
    let names: Vec<&str> = b.local.iter().map(|b| b.name.as_str()).collect();
    assert!(names.contains(&"main") && names.contains(&"feature/x") && names.contains(&"other"));
    assert!(
        b.local
            .iter()
            .find(|b| b.name == "feature/x")
            .unwrap()
            .is_head
    );
    assert_eq!(b.recent.first().map(String::as_str), Some("feature/x"));

    repo.checkout_branch("main", false).unwrap();
    repo.delete_branch("other", false).unwrap();
    assert!(!repo
        .branches()
        .unwrap()
        .local
        .iter()
        .any(|b| b.name == "other"));
}

#[test]
fn remote_tracking_checkout_push_pull() {
    // Bare "remote" + a clone.
    let remote_dir = tempfile::tempdir().unwrap();
    let c = cli();
    c.run(
        remote_dir.path(),
        &["init", "-q", "--bare", "-b", "main"],
        RunOptions::write(),
    )
    .unwrap();

    let (dir, repo) = repo_with_commit();
    let url = remote_dir.path().to_string_lossy().to_string();
    repo.cli()
        .run(
            repo.root(),
            &["remote", "add", "origin", &url],
            RunOptions::write(),
        )
        .unwrap();
    // First push sets upstream.
    repo.push(false, &NetOptions::default()).unwrap();
    let snap = repo.snapshot().unwrap();
    assert_eq!(snap.head.upstream.as_deref(), Some("origin/main"));
    assert!(snap.branches.remote.iter().any(|b| b.name == "origin/main"));

    // Push a branch, delete it locally, then check it out from the remote.
    repo.create_branch("topic", None, true).unwrap();
    repo.push(false, &NetOptions::default()).unwrap();
    repo.checkout_branch("main", false).unwrap();
    repo.delete_branch("topic", true).unwrap();
    repo.checkout_branch("origin/topic", true).unwrap();
    let (head, _) = repo.status().unwrap();
    assert_eq!(head.branch.as_deref(), Some("topic"));
    assert_eq!(head.upstream.as_deref(), Some("origin/topic"));

    // Second clone pushes a commit; first repo pulls it.
    let other = tempfile::tempdir().unwrap();
    c.run(
        other.path(),
        &["clone", "-q", &url, "."],
        RunOptions::write(),
    )
    .unwrap();
    let other_repo = Repo::open(c.clone(), other.path()).unwrap();
    write(other.path(), "remote.txt", "r\n");
    other_repo.stage_all().unwrap();
    other_repo.commit("from other", false).unwrap();
    other_repo.push(false, &NetOptions::default()).unwrap();

    repo.checkout_branch("main", false).unwrap();
    repo.fetch(&NetOptions::default()).unwrap();
    assert_eq!(repo.status().unwrap().0.behind, 1);
    repo.pull(PullMode::FastForwardOnly, &NetOptions::default())
        .unwrap();
    assert!(dir.path().join("remote.txt").exists());
}

#[test]
fn merge_conflict_is_reported() {
    let (dir, repo) = repo_with_commit();
    repo.create_branch("side", None, true).unwrap();
    write(dir.path(), "a.txt", "side\n");
    repo.stage_all().unwrap();
    repo.commit("side", false).unwrap();
    repo.checkout_branch("main", false).unwrap();
    write(dir.path(), "a.txt", "main\n");
    repo.stage_all().unwrap();
    repo.commit("main", false).unwrap();

    let merge = repo
        .cli()
        .run(repo.root(), &["merge", "side"], RunOptions::write());
    assert!(merge.is_err());
    let snap = repo.snapshot().unwrap();
    assert_eq!(snap.operation, Some(RepoOperation::Merge));
    let a = find(&snap.changes, "a.txt");
    assert!(a.conflicted);
    // Combined diff of a conflicted file parses without panicking.
    let d = repo
        .diff_file("a.txt", DiffMode::WorkingTree, false)
        .unwrap();
    assert!(!d.hunks.is_empty());
}

#[test]
fn binary_file_diff() {
    let (dir, repo) = repo_with_commit();
    fs::write(dir.path().join("img.bin"), [0u8, 1, 2, 3, 0, 255]).unwrap();
    repo.stage_all().unwrap();
    let d = repo.diff_file("img.bin", DiffMode::Staged, false).unwrap();
    assert!(d.binary);
    let d = repo.diff_file("img.bin", DiffMode::Head, true).unwrap();
    assert!(d.binary);
}

#[test]
fn push_preview_and_progress() {
    let remote_dir = tempfile::tempdir().unwrap();
    let c = cli();
    c.run(
        remote_dir.path(),
        &["init", "-q", "--bare", "-b", "main"],
        RunOptions::write(),
    )
    .unwrap();
    let (dir, repo) = repo_with_commit();

    // No remote configured: preview has no target, push fails cleanly.
    let p = repo.push_preview(100).unwrap();
    assert!(p.remote.is_none() && p.new_branch);
    assert!(repo.push(false, &NetOptions::default()).is_err());

    let url = remote_dir.path().to_string_lossy().to_string();
    repo.cli()
        .run(
            repo.root(),
            &["remote", "add", "origin", &url],
            RunOptions::write(),
        )
        .unwrap();

    // First push: new branch, every local commit is outgoing.
    let p = repo.push_preview(100).unwrap();
    assert!(p.new_branch);
    assert_eq!(p.target.as_deref(), Some("origin/main"));
    assert_eq!(p.commits.len(), 1);

    // Progress lines are parsed and kept out of stderr.
    let seen = Arc::new(Mutex::new(Vec::<Progress>::new()));
    let sink = {
        let seen = seen.clone();
        ProgressSink::new(move |p| seen.lock().unwrap().push(p.clone()))
    };
    let net = NetOptions {
        cancel: None,
        progress: Some(sink),
    };
    repo.push(false, &net).unwrap();
    let seen = seen.lock().unwrap();
    assert!(
        seen.iter().any(|p| p.phase == "Writing objects"),
        "{seen:?}"
    );

    // After pushing nothing is outgoing; a new commit is.
    let p = repo.push_preview(100).unwrap();
    assert!(!p.new_branch && p.commits.is_empty());
    write(dir.path(), "x.txt", "x\n");
    repo.stage_all().unwrap();
    repo.commit("second", false).unwrap();
    let p = repo.push_preview(100).unwrap();
    assert_eq!(p.commits.len(), 1);
    assert_eq!(p.commits[0].subject, "second");
}

#[test]
fn network_error_message_has_no_progress_noise() {
    let (_dir, repo) = repo_with_commit();
    repo.cli()
        .run(
            repo.root(),
            &["remote", "add", "origin", "/nonexistent/repo.git"],
            RunOptions::write(),
        )
        .unwrap();
    let net = NetOptions {
        cancel: None,
        progress: Some(ProgressSink::new(|_| {})),
    };
    let err = repo.fetch(&net).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("does not appear to be a git repository") || msg.contains("fatal"),
        "{msg}"
    );
    assert!(!msg.contains('%'), "{msg}");
}

/// Cancelling a hung network operation returns promptly and kills the
/// transport helper too (it runs in git's process group).
#[cfg(unix)]
#[test]
fn cancel_hung_fetch() {
    use std::time::{Duration, Instant};
    let (_dir, repo) = repo_with_commit();
    let run = |args: &[&str]| {
        repo.cli()
            .run(repo.root(), args, RunOptions::write())
            .unwrap();
    };
    // `ext::` transport running a command that never answers.
    run(&["config", "protocol.ext.allow", "always"]);
    run(&["remote", "add", "slow", "ext::sh -c sleep% 30"]);

    let token = CancelToken::new();
    let t2 = token.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(400));
        t2.cancel();
    });
    let started = Instant::now();
    let net = NetOptions {
        cancel: Some(token),
        progress: None,
    };
    let err = repo.fetch(&net).unwrap_err();
    assert!(matches!(err, GitError::Cancelled), "{err:?}");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "took {:?}",
        started.elapsed()
    );
}

#[test]
fn commit_details_and_file_diffs() {
    let (dir, repo) = new_repo();
    let root = dir.path();
    write(root, "a.txt", "one\ntwo\n");
    write(root, "img.bin", "\x00\x01\x02");
    repo.stage_all().unwrap();
    let first = repo.commit("Initial\n\nWith a body.", false).unwrap();

    let d = repo.commit_details(&first).unwrap();
    assert_eq!(d.message, "Initial\n\nWith a body.");
    assert!(d.parents.is_empty());
    assert_eq!(d.author_name, "Test");
    let paths: Vec<_> = d
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.kind, f.added))
        .collect();
    assert_eq!(
        paths,
        [
            ("a.txt", ChangeKind::Added, Some(2)),
            ("img.bin", ChangeKind::Added, None)
        ]
    );
    let diff = repo.commit_file_diff(&first, None, "a.txt", None).unwrap();
    assert_eq!(diff.added, 2);

    // Rename with an edit, plus a modification.
    fs::rename(root.join("a.txt"), root.join("b.txt")).unwrap();
    write(root, "b.txt", "one\ntwo\nthree\n");
    write(root, "img.bin", "\x00\x01\x03");
    repo.stage_all().unwrap();
    let second = repo.commit("Rename", false).unwrap();
    let d = repo.commit_details(&second).unwrap();
    assert_eq!(d.parents, vec![first.clone()]);
    let b = d.files.iter().find(|f| f.path == "b.txt").unwrap();
    assert_eq!(b.kind, ChangeKind::Renamed);
    assert_eq!(b.orig_path.as_deref(), Some("a.txt"));
    assert_eq!((b.added, b.deleted), (Some(1), Some(0)));
    let diff = repo
        .commit_file_diff(&second, Some(&first), "b.txt", Some("a.txt"))
        .unwrap();
    assert_eq!((diff.added, diff.deleted), (1, 0));
    assert_eq!(diff.hunks[0].lines.last().unwrap().text, "three");

    assert!(matches!(
        repo.commit_details("--all"),
        Err(GitError::Invalid(_))
    ));
    assert!(repo.commit_details("deadbeef").is_err());
}

fn head(repo: &Repo) -> String {
    repo.cli()
        .run(repo.root(), &["rev-parse", "HEAD"], RunOptions::read())
        .unwrap()
        .stdout_str()
        .trim()
        .to_string()
}

#[test]
fn log_actions_tag_cherry_pick_revert_reset() {
    let (dir, repo) = new_repo();
    let root = dir.path();
    write(root, "a.txt", "1\n");
    repo.stage_all().unwrap();
    let c1 = repo.commit("one", false).unwrap();
    write(root, "a.txt", "1\n2\n");
    repo.stage_all().unwrap();
    let c2 = repo.commit("two", false).unwrap();

    // Tags: lightweight and annotated.
    repo.create_tag("v1", &c1, None).unwrap();
    repo.create_tag("v2", &c2, Some("Release 2")).unwrap();
    let tags = repo
        .cli()
        .run(root, &["tag", "-n1"], RunOptions::read())
        .unwrap()
        .stdout_str()
        .into_owned();
    assert!(tags.contains("v1") && tags.contains("Release 2"), "{tags}");
    assert!(repo.create_tag("-f", &c1, None).is_err());
    assert!(repo.create_tag("bad name", &c1, None).is_err());
    assert!(repo.create_branch("--force", None, false).is_err());

    // Branch from c1, cherry-pick c2 onto it.
    repo.create_branch("side", Some(&c1), true).unwrap();
    repo.cherry_pick(&c2).unwrap();
    assert_eq!(fs::read_to_string(root.join("a.txt")).unwrap(), "1\n2\n");
    // (Same parent, tree and timestamps: git may produce the identical commit.)

    // Revert it again.
    repo.revert(&head(&repo)).unwrap();
    assert_eq!(fs::read_to_string(root.join("a.txt")).unwrap(), "1\n");
    assert!(repo.operation().is_none());

    // Reset modes.
    let tip = head(&repo);
    // tip has c1's content; moving to c2 leaves that content staged.
    repo.reset(&c2, ResetMode::Soft).unwrap();
    assert_eq!(head(&repo), c2);
    assert!(
        !repo.status().unwrap().1.is_empty(),
        "soft keeps changes staged"
    );
    repo.reset(&tip, ResetMode::Hard).unwrap();
    assert_eq!(head(&repo), tip);
    write(root, "a.txt", "dirty\n");
    // c2 changes a.txt, which has local edits: --keep refuses.
    assert!(
        repo.reset(&c2, ResetMode::Keep).is_err(),
        "keep refuses to lose changes"
    );
    repo.reset(&c1, ResetMode::Hard).unwrap();
    assert!(repo.status().unwrap().1.is_empty());

    // Detached checkout.
    repo.checkout_detached(&c2).unwrap();
    assert_eq!(head(&repo), c2);
    assert!(repo.status().unwrap().0.branch.is_none());
}

#[test]
fn conflicting_cherry_pick_can_be_aborted_or_continued() {
    let (dir, repo) = new_repo();
    let root = dir.path();
    write(root, "a.txt", "base\n");
    repo.stage_all().unwrap();
    let base = repo.commit("base", false).unwrap();
    write(root, "a.txt", "main\n");
    repo.stage_all().unwrap();
    let on_main = repo.commit("main change", false).unwrap();
    repo.create_branch("other", Some(&base), true).unwrap();
    write(root, "a.txt", "other\n");
    repo.stage_all().unwrap();
    repo.commit("other change", false).unwrap();

    assert!(repo.cherry_pick(&on_main).is_err());
    assert_eq!(repo.operation(), Some(RepoOperation::CherryPick));
    repo.abort_operation().unwrap();
    assert_eq!(repo.operation(), None);
    assert_eq!(fs::read_to_string(root.join("a.txt")).unwrap(), "other\n");

    // Resolve and continue: the original message is kept, no editor opens.
    assert!(repo.cherry_pick(&on_main).is_err());
    write(root, "a.txt", "resolved\n");
    repo.stage_all().unwrap();
    repo.continue_operation().unwrap();
    assert_eq!(repo.operation(), None);
    assert_eq!(repo.last_commit_message().unwrap(), "main change");
}

#[test]
fn file_texts_for_each_side() {
    let (dir, repo) = new_repo();
    let root = dir.path();
    write(root, "a.txt", "one\n");
    repo.stage_all().unwrap();
    let c1 = repo.commit("one", false).unwrap();
    write(root, "a.txt", "one\nstaged\n");
    repo.stage(&s(&["a.txt"])).unwrap();
    write(root, "a.txt", "one\nstaged\nworktree\n");

    let t = repo
        .file_texts("a.txt", None, DiffMode::Head, false)
        .unwrap();
    assert_eq!(t.old.as_deref(), Some("one\n"));
    assert_eq!(t.new.as_deref(), Some("one\nstaged\nworktree\n"));
    let t = repo
        .file_texts("a.txt", None, DiffMode::Staged, false)
        .unwrap();
    assert_eq!(t.new.as_deref(), Some("one\nstaged\n"));
    let t = repo
        .file_texts("a.txt", None, DiffMode::WorkingTree, false)
        .unwrap();
    assert_eq!(t.old.as_deref(), Some("one\nstaged\n"));

    // Untracked and deleted files have one side only.
    write(root, "new.txt", "fresh\n");
    let t = repo
        .file_texts("new.txt", None, DiffMode::Head, true)
        .unwrap();
    assert_eq!((t.old, t.new.as_deref()), (None, Some("fresh\n")));
    repo.stage_all().unwrap();
    let c2 = repo.commit("two", false).unwrap();
    fs::remove_file(root.join("new.txt")).unwrap();
    let t = repo
        .file_texts("new.txt", None, DiffMode::Head, false)
        .unwrap();
    assert_eq!((t.old.as_deref(), t.new), (Some("fresh\n"), None));

    // Commits, including the root commit.
    let t = repo
        .commit_file_texts(&c2, Some(&c1), "a.txt", None)
        .unwrap();
    assert_eq!(t.old.as_deref(), Some("one\n"));
    assert_eq!(t.new.as_deref(), Some("one\nstaged\nworktree\n"));
    let t = repo.commit_file_texts(&c1, None, "a.txt", None).unwrap();
    assert_eq!((t.old, t.new.as_deref()), (None, Some("one\n")));

    // Binary and oversized files return no text.
    write(root, "bin.dat", "a\x00b");
    let t = repo
        .file_texts("bin.dat", None, DiffMode::Head, true)
        .unwrap();
    assert!(t.binary && t.new.is_none());
    let big = "x".repeat(9 * 1024 * 1024);
    write(root, "big.txt", &big);
    let t = repo
        .file_texts("big.txt", None, DiffMode::Head, true)
        .unwrap();
    assert!(t.too_large && t.new.is_none());
    repo.stage(&s(&["big.txt"])).unwrap();
    let t = repo
        .file_texts("big.txt", None, DiffMode::Staged, false)
        .unwrap();
    assert!(t.too_large, "index blob over the limit is not loaded");

    assert!(repo
        .file_texts("../escape", None, DiffMode::Head, true)
        .is_err());
}

fn staged_text(repo: &Repo, path: &str) -> String {
    repo.cli()
        .run(
            repo.root(),
            &["show", &format!(":{path}")],
            RunOptions::read(),
        )
        .unwrap()
        .stdout_str()
        .into_owned()
}

#[test]
fn stage_content_and_worktree_writes() {
    let (dir, repo) = new_repo();
    let root = dir.path();
    write(root, "a.txt", "1\n2\n3\n");
    repo.stage_all().unwrap();
    repo.commit("base", false).unwrap();
    write(root, "a.txt", "1\nTWO\n3\nfour\n");

    // The index version comes with HEAD ↔ worktree texts.
    let t = repo
        .file_texts("a.txt", None, DiffMode::Head, false)
        .unwrap();
    assert_eq!(t.index.as_deref(), Some("1\n2\n3\n"));
    assert!(!t.lossy);

    // Stage only the first block.
    repo.stage_content("a.txt", "1\nTWO\n3\n").unwrap();
    assert_eq!(staged_text(&repo, "a.txt"), "1\nTWO\n3\n");
    let st = repo.status().unwrap().1;
    let a = find(&st, "a.txt");
    assert_eq!(
        (a.staged, a.unstaged),
        (Some(ChangeKind::Modified), Some(ChangeKind::Modified))
    );
    let t = repo
        .file_texts("a.txt", None, DiffMode::Head, false)
        .unwrap();
    assert_eq!(t.index.as_deref(), Some("1\nTWO\n3\n"));

    // A new file can be staged partially too.
    write(root, "new.txt", "keep\ndrop\n");
    repo.stage_content("new.txt", "keep\n").unwrap();
    assert_eq!(staged_text(&repo, "new.txt"), "keep\n");

    // Discarding writes the worktree only if it is unchanged since loading.
    let err = repo
        .write_worktree_file("a.txt", "1\n2\n3\n", "stale content")
        .unwrap_err();
    assert!(matches!(err, GitError::Invalid(_)), "{err:?}");
    repo.write_worktree_file("a.txt", "1\nTWO\n3\n", "1\nTWO\n3\nfour\n")
        .unwrap();
    assert_eq!(
        fs::read_to_string(root.join("a.txt")).unwrap(),
        "1\nTWO\n3\n"
    );
    assert!(repo.write_worktree_file("../x", "", "").is_err());

    // Bare CR and invalid UTF-8 are flagged.
    fs::write(root.join("cr.txt"), b"a\rb\n").unwrap();
    assert!(
        repo.file_texts("cr.txt", None, DiffMode::Head, true)
            .unwrap()
            .lossy
    );
    fs::write(root.join("crlf.txt"), b"a\r\nb\r\n").unwrap();
    assert!(
        !repo
            .file_texts("crlf.txt", None, DiffMode::Head, true)
            .unwrap()
            .lossy
    );
}

#[cfg(unix)]
#[test]
fn stage_content_keeps_executable_mode() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, repo) = new_repo();
    let root = dir.path();
    write(root, "run.sh", "#!/bin/sh\necho 1\n");
    fs::set_permissions(root.join("run.sh"), fs::Permissions::from_mode(0o755)).unwrap();
    repo.stage_content("run.sh", "#!/bin/sh\n").unwrap();
    let ls = repo
        .cli()
        .run(root, &["ls-files", "-s", "run.sh"], RunOptions::read())
        .unwrap()
        .stdout_str()
        .into_owned();
    assert!(ls.starts_with("100755"), "{ls}");
}

#[test]
fn stash_push_list_apply_pop_drop() {
    let (dir, repo) = new_repo();
    let root = dir.path();
    write(root, "a.txt", "1\n");
    repo.stage_all().unwrap();
    repo.commit("base", false).unwrap();
    assert!(repo.stash_list().unwrap().is_empty());
    assert!(
        repo.stash_push(&StashOptions::default()).is_err(),
        "nothing to stash"
    );

    // Tracked change + untracked file, stashed with a message.
    write(root, "a.txt", "1\n2\n");
    write(root, "new.txt", "new\n");
    repo.stash_push(&StashOptions {
        message: Some("try parser".into()),
        include_untracked: true,
        ..StashOptions::default()
    })
    .unwrap();
    assert!(repo.status().unwrap().1.is_empty());
    let list = repo.stash_list().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(
        (
            list[0].index,
            list[0].message.as_str(),
            list[0].branch.as_deref()
        ),
        (0, "try parser", Some("main"))
    );

    // Details: the tracked change and the untracked file.
    let d = repo.stash_details(&list[0].oid).unwrap();
    let files: Vec<_> = d.files.iter().map(|f| (f.path.as_str(), f.kind)).collect();
    assert_eq!(
        files,
        [
            ("a.txt", ChangeKind::Modified),
            ("new.txt", ChangeKind::Untracked)
        ]
    );
    let t = repo
        .commit_file_texts(&d.oid, Some(&d.parents[0]), "a.txt", None)
        .unwrap();
    assert_eq!(
        (t.old.as_deref(), t.new.as_deref()),
        (Some("1\n"), Some("1\n2\n"))
    );
    let t = repo
        .commit_file_texts(&d.parents[2], None, "new.txt", None)
        .unwrap();
    assert_eq!(t.new.as_deref(), Some("new\n"));

    // A second stash (tracked only) shifts the first to stash@{1}.
    write(root, "a.txt", "1\nother\n");
    repo.stash_push(&StashOptions::default()).unwrap();
    let list = repo.stash_list().unwrap();
    assert_eq!(list.len(), 2);
    let (newest, older) = (list[0].clone(), list[1].clone());
    assert_eq!(older.message, "try parser");

    // A stale index/oid pair is refused.
    assert!(repo.stash_drop(0, &older.oid).is_err());

    // Apply keeps the stash; pop removes it.
    repo.stash_apply(newest.index, &newest.oid, false, false)
        .unwrap();
    assert_eq!(
        fs::read_to_string(root.join("a.txt")).unwrap(),
        "1\nother\n"
    );
    assert_eq!(repo.stash_list().unwrap().len(), 2);
    repo.rollback(&s(&["a.txt"]), &[]).unwrap();
    repo.stash_apply(older.index, &older.oid, true, false)
        .unwrap();
    assert_eq!(fs::read_to_string(root.join("new.txt")).unwrap(), "new\n");
    let list = repo.stash_list().unwrap();
    assert_eq!(list.len(), 1);

    // Drop the rest.
    repo.stash_drop(list[0].index, &list[0].oid).unwrap();
    assert!(repo.stash_list().unwrap().is_empty());
}

#[test]
fn stash_keep_index_and_restore_index() {
    let (dir, repo) = new_repo();
    let root = dir.path();
    write(root, "a.txt", "1\n");
    write(root, "b.txt", "1\n");
    repo.stage_all().unwrap();
    repo.commit("base", false).unwrap();
    write(root, "a.txt", "staged\n");
    repo.stage(&s(&["a.txt"])).unwrap();
    write(root, "b.txt", "unstaged\n");

    repo.stash_push(&StashOptions {
        keep_index: true,
        ..StashOptions::default()
    })
    .unwrap();
    // Staged change stays, unstaged one is stashed.
    let st = repo.status().unwrap().1;
    assert_eq!(st.len(), 1);
    assert_eq!(st[0].path, "a.txt");

    repo.stage_all().unwrap();
    repo.commit("staged part", false).unwrap();
    let e = repo.stash_list().unwrap()[0].clone();
    repo.stash_apply(e.index, &e.oid, true, true).unwrap();
    assert_eq!(
        fs::read_to_string(root.join("b.txt")).unwrap(),
        "unstaged\n"
    );
}

#[test]
fn stash_selected_paths_literally() {
    let (dir, repo) = new_repo();
    let root = dir.path();
    write(root, "a[1].txt", "1\n");
    write(root, "a1.txt", "1\n");
    repo.stage_all().unwrap();
    repo.commit("base", false).unwrap();
    write(root, "a[1].txt", "2\n");
    write(root, "a1.txt", "2\n");
    repo.stash_push(&StashOptions {
        paths: s(&["a[1].txt"]),
        ..StashOptions::default()
    })
    .unwrap();
    // Only the literal path was stashed; "a1.txt" (a glob match) was not.
    let st = repo.status().unwrap().1;
    assert_eq!(
        st.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(),
        ["a1.txt"]
    );
}

#[test]
fn refs_fingerprint_tracks_ref_changes_only() {
    let (dir, repo) = new_repo();
    let root = dir.path();
    write(root, "a.txt", "1\n");
    repo.stage_all().unwrap();
    repo.commit("one", false).unwrap();
    let git = |args: &[&str]| {
        repo.cli().run(root, args, RunOptions::write()).unwrap();
    };

    let fp = repo.refs_fingerprint();
    assert_eq!(fp, repo.refs_fingerprint(), "stable");
    write(root, "a.txt", "edited\n");
    write(root, "new.txt", "x\n");
    assert_eq!(fp, repo.refs_fingerprint(), "worktree edits do not count");

    let mut seen = vec![fp];
    let mut changed = |what: &str| {
        let now = repo.refs_fingerprint();
        assert!(!seen.contains(&now), "{what} must change the fingerprint");
        seen.push(now);
    };
    git(&["branch", "feature"]);
    changed("new branch");
    git(&["tag", "-a", "v1", "-m", "v1"]);
    changed("annotated tag");
    git(&["switch", "-q", "feature"]);
    changed("checkout");
    git(&["commit", "-q", "--allow-empty", "-m", "two"]);
    changed("commit");
    git(&["pack-refs", "--all"]);
    changed("pack-refs");
    git(&["update-ref", "refs/heads/main", "HEAD"]);
    changed("packed ref updated");

    // The snapshot's branch list follows at once, despite the cache.
    git(&["branch", "late"]);
    let snap = repo.snapshot().unwrap();
    assert!(snap.branches.local.iter().any(|b| b.name == "late"));
    git(&["branch", "-D", "late"]);
    let snap = repo.snapshot().unwrap();
    assert!(!snap.branches.local.iter().any(|b| b.name == "late"));
}

#[test]
fn commit_graph_is_written_once() {
    let (dir, repo) = new_repo();
    write(dir.path(), "a.txt", "1\n");
    repo.stage_all().unwrap();
    repo.commit("one", false).unwrap();
    assert!(repo.ensure_commit_graph().unwrap());
    assert!(!repo.ensure_commit_graph().unwrap(), "already there");
}

#[test]
fn refs_fingerprint_in_linked_worktree() {
    let (dir, repo) = new_repo();
    write(dir.path(), "a.txt", "1\n");
    repo.stage_all().unwrap();
    repo.commit("one", false).unwrap();
    let wt_dir = tempfile::tempdir().unwrap();
    let wt_path = wt_dir.path().join("wt");
    repo.cli()
        .run(
            dir.path(),
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "side",
                wt_path.to_str().unwrap(),
            ],
            RunOptions::write(),
        )
        .unwrap();
    let wt = Repo::open(cli(), &wt_path).unwrap();
    let fp = wt.refs_fingerprint();
    // A commit on main, made in the main worktree, moves a shared ref.
    repo.cli()
        .run(
            dir.path(),
            &["commit", "-q", "--allow-empty", "-m", "two"],
            RunOptions::write(),
        )
        .unwrap();
    assert_ne!(fp, wt.refs_fingerprint());
    assert_eq!(wt.snapshot().unwrap().head.branch.as_deref(), Some("side"));
}

/// A repo stopped in a merge with a conflict in a.txt.
fn conflicted_repo() -> (tempfile::TempDir, Repo) {
    let (dir, repo) = new_repo();
    let root = dir.path();
    write(root, "a.txt", "1\n2\n3\n");
    write(root, "gone.txt", "x\n");
    repo.stage_all().unwrap();
    repo.commit("base", false).unwrap();
    repo.create_branch("other", None, true).unwrap();
    write(root, "a.txt", "1\nTHEIRS\n3\n");
    fs::remove_file(root.join("gone.txt")).unwrap();
    repo.stage_all().unwrap();
    repo.commit("their change", false).unwrap();
    repo.checkout_branch("main", false).unwrap();
    write(root, "a.txt", "1\nOURS\n3\n");
    write(root, "gone.txt", "edited\n");
    repo.stage_all().unwrap();
    repo.commit("our change", false).unwrap();
    let merge = repo
        .cli()
        .run(root, &["merge", "other"], RunOptions::write())
        .unwrap_err();
    assert!(matches!(merge, GitError::Failed { .. }));
    (dir, repo)
}

#[test]
fn conflict_texts_and_resolution() {
    let (dir, repo) = conflicted_repo();
    let t = repo.conflict_texts("a.txt").unwrap();
    assert_eq!(t.base.as_deref(), Some("1\n2\n3\n"));
    assert_eq!(t.ours.as_deref(), Some("1\nOURS\n3\n"));
    assert_eq!(t.theirs.as_deref(), Some("1\nTHEIRS\n3\n"));
    assert_eq!(t.ours_label, "main");
    assert!(
        t.theirs_label.ends_with("their change"),
        "{}",
        t.theirs_label
    );

    // Modify/delete: they deleted gone.txt.
    let g = repo.conflict_texts("gone.txt").unwrap();
    assert_eq!(
        (g.ours.as_deref(), g.theirs.as_deref()),
        (Some("edited\n"), None)
    );

    // Resolve a.txt with merged content, gone.txt by taking their deletion.
    repo.resolve_conflict("a.txt", Some("1\nOURS\nTHEIRS\n3\n"))
        .unwrap();
    repo.resolve_with_side("gone.txt", ConflictSide::Theirs)
        .unwrap();
    let st = repo.status().unwrap().1;
    assert!(st.iter().all(|c| !c.conflicted), "{st:?}");
    assert!(!dir.path().join("gone.txt").exists());
    assert_eq!(
        fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "1\nOURS\nTHEIRS\n3\n"
    );
    repo.continue_operation().unwrap();
    assert_eq!(repo.operation(), None);
}

#[test]
fn resolve_with_ours() {
    let (dir, repo) = conflicted_repo();
    repo.resolve_with_side("a.txt", ConflictSide::Ours).unwrap();
    repo.resolve_with_side("gone.txt", ConflictSide::Ours)
        .unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "1\nOURS\n3\n"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("gone.txt")).unwrap(),
        "edited\n"
    );
    assert!(repo.status().unwrap().1.iter().all(|c| !c.conflicted));
    assert!(repo.conflict_texts("../x").is_err());
}

#[test]
fn rebase_progress_and_skip() {
    let (dir, repo) = new_repo();
    let root = dir.path();
    write(root, "a.txt", "base\n");
    repo.stage_all().unwrap();
    repo.commit("base", false).unwrap();
    repo.create_branch("feature", None, true).unwrap();
    write(root, "a.txt", "feature\n");
    repo.stage_all().unwrap();
    repo.commit("feature edit", false).unwrap();
    write(root, "b.txt", "b\n");
    repo.stage_all().unwrap();
    repo.commit("add b", false).unwrap();
    repo.checkout_branch("main", false).unwrap();
    write(root, "a.txt", "main\n");
    repo.stage_all().unwrap();
    repo.commit("main edit", false).unwrap();
    repo.checkout_branch("feature", false).unwrap();

    let err = repo
        .cli()
        .run(root, &["rebase", "main"], RunOptions::write())
        .unwrap_err();
    assert!(matches!(err, GitError::Failed { .. }));
    let snap = repo.snapshot().unwrap();
    assert_eq!(snap.operation, Some(RepoOperation::Rebase));
    let info = snap.operation_info.unwrap();
    assert_eq!((info.step, info.total), (Some(1), Some(2)));
    assert_eq!(info.branch.as_deref(), Some("feature"));
    assert!(info.onto.unwrap().ends_with("main edit"));
    assert!(info.commit.unwrap().ends_with("feature edit"));
    assert!(info.can_skip);

    // Skipping the conflicting commit applies the rest and finishes.
    repo.skip_operation().unwrap();
    assert_eq!(repo.operation(), None);
    assert_eq!(fs::read_to_string(root.join("a.txt")).unwrap(), "main\n");
    assert!(root.join("b.txt").exists());
    assert_eq!(repo.last_commit_message().unwrap(), "add b");
    assert!(repo.snapshot().unwrap().operation_info.is_none());
}

#[test]
fn merge_cannot_skip() {
    let (_dir, repo) = conflicted_repo();
    let info = repo.operation_info().unwrap();
    assert!(!info.can_skip);
    assert!(info.commit.unwrap().ends_with("their change"));
    assert!(matches!(repo.skip_operation(), Err(GitError::Invalid(_))));
}

fn subjects(repo: &Repo, n: usize) -> Vec<String> {
    repo.log(None, n)
        .unwrap()
        .into_iter()
        .map(|c| c.subject)
        .collect()
}

/// base + c1..c5, each adding its own file.
fn linear_repo() -> (tempfile::TempDir, Repo, Vec<String>) {
    let (dir, repo) = new_repo();
    let mut oids = Vec::new();
    for (i, name) in ["base", "c1", "c2", "c3", "c4", "c5"].iter().enumerate() {
        write(dir.path(), &format!("{name}.txt"), &format!("{i}\n"));
        repo.stage_all().unwrap();
        oids.push(repo.commit(name, false).unwrap());
    }
    (dir, repo, oids)
}

fn step(action: RebaseAction, oid: &str) -> RebaseStep {
    RebaseStep {
        action,
        oid: oid.to_string(),
        message: None,
    }
}

#[test]
fn interactive_rebase_reorders_rewords_squashes_and_drops() {
    let (dir, repo, oids) = linear_repo();
    let plan = repo.rebase_plan(&oids[1]).unwrap();
    assert_eq!(
        plan.iter().map(|c| c.subject.as_str()).collect::<Vec<_>>(),
        ["c1", "c2", "c3", "c4", "c5"]
    );

    // c3 left out: the plan must be a permutation of the branch's commits.
    let missing = [
        step(RebaseAction::Pick, &oids[2]),
        step(RebaseAction::Pick, &oids[1]),
        step(RebaseAction::Pick, &oids[4]),
        step(RebaseAction::Pick, &oids[5]),
    ];
    assert!(matches!(
        repo.interactive_rebase(&oids[1], &missing, false),
        Err(GitError::Invalid(_))
    ));
    assert_eq!(repo.operation(), None);

    // c2 first, then c1 with c4 squashed in, then c3 reworded with c5 fixed up.

    let steps = [
        step(RebaseAction::Pick, &oids[2]),
        step(RebaseAction::Pick, &oids[1]),
        step(RebaseAction::Squash, &oids[4]),
        RebaseStep {
            action: RebaseAction::Reword,
            oid: oids[3].clone(),
            message: Some("c3 renamed\n\nWith a body.".into()),
        },
        step(RebaseAction::Fixup, &oids[5]),
    ];
    repo.interactive_rebase(&oids[1], &steps, false).unwrap();
    assert_eq!(repo.operation(), None);
    assert_eq!(subjects(&repo, 4), ["c3 renamed", "c1", "c2", "base"]);
    assert_eq!(
        repo.last_commit_message().unwrap(),
        "c3 renamed\n\nWith a body."
    );
    // Squash kept both messages; every file is still there.
    let c1 = repo.log(None, 3).unwrap()[1].clone();
    let msg = repo
        .cli()
        .run(
            dir.path(),
            &["log", "-1", "--format=%B", &c1.oid],
            RunOptions::read(),
        )
        .unwrap()
        .stdout_str()
        .into_owned();
    assert!(msg.contains("c1") && msg.contains("c4"), "{msg}");
    for f in ["c1", "c2", "c3", "c4", "c5"] {
        assert!(dir.path().join(format!("{f}.txt")).exists(), "{f}");
    }
}

#[test]
fn interactive_rebase_drop_edit_root_and_refusals() {
    let (dir, repo, oids) = linear_repo();
    // Drop c4 and c5.
    let steps: Vec<_> = oids[3..]
        .iter()
        .enumerate()
        .map(|(i, o)| {
            step(
                if i == 0 {
                    RebaseAction::Pick
                } else {
                    RebaseAction::Drop
                },
                o,
            )
        })
        .collect();
    repo.interactive_rebase(&oids[3], &steps, false).unwrap();
    assert_eq!(subjects(&repo, 2), ["c3", "c2"]);
    assert!(!dir.path().join("c4.txt").exists());

    // `edit` stops; the banner's Continue finishes it.
    let plan = repo.rebase_plan(&oids[2]).unwrap();
    let steps = [
        step(RebaseAction::Edit, &plan[0].oid),
        step(RebaseAction::Pick, &plan[1].oid),
    ];
    let finished = repo
        .interactive_rebase(&plan[0].oid, &steps, false)
        .unwrap();
    assert!(!finished, "stopped at edit");
    assert_eq!(repo.operation(), Some(RepoOperation::Rebase));
    let info = repo.operation_info().unwrap();
    assert_eq!((info.step, info.total), (Some(1), Some(2)));
    repo.continue_operation().unwrap();
    assert_eq!(repo.operation(), None);

    // From the root commit: reword it.
    let plan = repo.rebase_plan(&oids[0]).unwrap();
    assert_eq!(plan[0].subject, "base");
    let mut steps: Vec<_> = plan
        .iter()
        .map(|c| step(RebaseAction::Pick, &c.oid))
        .collect();
    steps[0] = RebaseStep {
        action: RebaseAction::Reword,
        oid: plan[0].oid.clone(),
        message: Some("root".into()),
    };
    repo.interactive_rebase(&plan[0].oid, &steps, false)
        .unwrap();
    assert_eq!(subjects(&repo, 10).last().unwrap(), "root");

    // Refusals: squash first, a dirty tree without autostash, merges.
    let plan = repo
        .rebase_plan(&repo.log(None, 2).unwrap()[1].oid)
        .unwrap();
    let squash_first = [
        step(RebaseAction::Squash, &plan[0].oid),
        step(RebaseAction::Pick, &plan[1].oid),
    ];
    assert!(matches!(
        repo.interactive_rebase(&plan[0].oid, &squash_first, false),
        Err(GitError::Invalid(_))
    ));
    write(dir.path(), "c1.txt", "dirty\n");
    let swap = [
        step(RebaseAction::Pick, &plan[1].oid),
        step(RebaseAction::Pick, &plan[0].oid),
    ];
    assert!(repo.interactive_rebase(&plan[0].oid, &swap, false).is_err());
    assert_eq!(repo.operation(), None);
    let plan = repo
        .rebase_plan(&repo.log(None, 2).unwrap()[1].oid)
        .unwrap();
    let swap = [
        step(RebaseAction::Pick, &plan[1].oid),
        step(RebaseAction::Pick, &plan[0].oid),
    ];
    repo.interactive_rebase(&plan[0].oid, &swap, true).unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join("c1.txt")).unwrap(),
        "dirty\n",
        "autostash restored"
    );
}

#[test]
fn rebase_plan_refuses_merges_and_foreign_commits() {
    let (_dir, repo) = conflicted_repo();
    repo.abort_operation().unwrap();
    let base = repo.log(None, 3).unwrap()[1].oid.clone();
    let side = repo
        .cli()
        .run(repo.root(), &["rev-parse", "other"], RunOptions::read())
        .unwrap()
        .stdout_str()
        .trim()
        .to_string();
    assert!(repo.rebase_plan(&side).is_err(), "not an ancestor of HEAD");
    repo.cli()
        .run(
            repo.root(),
            &["merge", "-q", "-s", "ours", "-m", "merge", "other"],
            RunOptions::write(),
        )
        .unwrap();
    assert!(repo.rebase_plan(&base).is_err(), "merge in range");
}

#[test]
fn blame_worktree_and_revisions() {
    let (dir, repo) = new_repo();
    let root = dir.path();
    // Long enough that the rename below is detected (>50% similar).
    let tail = "keep 1\nkeep 2\nkeep 3\nkeep 4\nkeep 5\n";
    write(root, "old.txt", &format!("one\ntwo\n{tail}"));
    repo.stage_all().unwrap();
    let c1 = repo.commit("first", false).unwrap();
    fs::rename(root.join("old.txt"), root.join("a.txt")).unwrap();
    write(root, "a.txt", &format!("one\nTWO\nthree\n{tail}"));
    repo.stage_all().unwrap();
    let c2 = repo.commit("second", false).unwrap();
    write(root, "a.txt", &format!("one\nTWO\nthree\n{tail}local\n"));

    let b = repo.blame("a.txt", None).unwrap();
    assert_eq!(b.text, format!("one\nTWO\nthree\n{tail}local\n"));
    let who: Vec<&str> = b
        .lines
        .iter()
        .map(|&i| b.commits[i as usize].oid.as_str())
        .collect();
    assert_eq!(who[0], c1, "kept across the rename");
    assert_eq!((who[1], who[2]), (c2.as_str(), c2.as_str()));
    assert_eq!(who[3], c1);
    assert!(who[8].bytes().all(|x| x == b'0'), "uncommitted");
    let second = b.commits.iter().find(|c| c.oid == c2).unwrap();
    assert_eq!(second.summary, "second");
    assert_eq!(second.author_name, "Test");
    assert_eq!(second.previous_oid.as_deref(), Some(c1.as_str()));
    assert_eq!(second.previous_path.as_deref(), Some("old.txt"));

    // At a commit: the file as it was then.
    let b = repo.blame("old.txt", Some(&c1)).unwrap();
    assert_eq!(b.text, format!("one\ntwo\n{tail}"));
    assert_eq!(b.commits.len(), 1);
    assert!(repo.blame("a.txt", Some("--all")).is_err());
    assert!(repo.blame("../x", None).is_err());
}

#[test]
fn file_and_line_history() {
    let (dir, repo) = new_repo();
    let root = dir.path();
    let body =
        |head: &str, mid: &str| format!("{head}\nkeep 1\nkeep 2\nkeep 3\n{mid}\nkeep 4\nkeep 5\n");
    write(root, "old.txt", &body("h1", "m1"));
    repo.stage_all().unwrap();
    repo.commit("add", false).unwrap();
    write(root, "old.txt", &body("h2", "m1"));
    repo.stage_all().unwrap();
    repo.commit("edit head", false).unwrap();
    fs::rename(root.join("old.txt"), root.join("new.txt")).unwrap();
    repo.stage_all().unwrap();
    repo.commit("rename", false).unwrap();
    write(root, "new.txt", &body("h2", "m2"));
    repo.stage_all().unwrap();
    let last = repo.commit("edit middle", false).unwrap();
    write(root, "other.txt", "x\n");
    repo.stage_all().unwrap();
    repo.commit("unrelated", false).unwrap();

    let h = repo.file_history("new.txt", 0, 100).unwrap();
    let got: Vec<_> = h
        .iter()
        .map(|r| (r.commit.subject.as_str(), r.path.as_str(), r.kind))
        .collect();
    assert_eq!(
        got,
        [
            ("edit middle", "new.txt", Some(ChangeKind::Modified)),
            ("rename", "new.txt", Some(ChangeKind::Renamed)),
            ("edit head", "old.txt", Some(ChangeKind::Modified)),
            ("add", "old.txt", Some(ChangeKind::Added)),
        ]
    );
    assert_eq!(h[1].orig_path.as_deref(), Some("old.txt"));
    assert_eq!(h[0].commit.oid, last);
    // Paging.
    let page = repo.file_history("new.txt", 1, 2).unwrap();
    assert_eq!(page.len(), 2);
    assert_eq!(page[0].commit.subject, "rename");

    // Line 1 changed in "edit head" and "add"; line 5 in "edit middle" and "add".
    let subjects = |v: Vec<CommitSummary>| v.into_iter().map(|c| c.subject).collect::<Vec<_>>();
    assert_eq!(
        subjects(repo.line_history("new.txt", 1, 1, None).unwrap()),
        ["edit head", "add"]
    );
    assert_eq!(
        subjects(repo.line_history("new.txt", 5, 5, None).unwrap()),
        ["edit middle", "add"]
    );
    assert!(repo.line_history("new.txt", 3, 2, None).is_err());
}

#[test]
fn compare_branches_and_files() {
    let (dir, repo) = new_repo();
    let root = dir.path();
    write(root, "a.txt", "base\n");
    repo.stage_all().unwrap();
    let base = repo.commit("base", false).unwrap();
    repo.create_branch("feature", None, true).unwrap();
    write(root, "f.txt", "feature\n");
    repo.stage_all().unwrap();
    repo.commit("f1", false).unwrap();
    write(root, "a.txt", "feature edit\n");
    repo.stage_all().unwrap();
    let f2 = repo.commit("f2", false).unwrap();
    repo.checkout_branch("main", false).unwrap();
    write(root, "m.txt", "main\n");
    repo.stage_all().unwrap();
    let m1 = repo.commit("m1", false).unwrap();

    let c = repo.compare("feature", "main", 100).unwrap();
    assert_eq!(c.left_oid, f2);
    assert_eq!(c.right_oid, m1);
    assert_eq!(c.merge_base.as_deref(), Some(base.as_str()));
    let subj = |v: &[CommitSummary]| v.iter().map(|c| c.subject.clone()).collect::<Vec<_>>();
    assert_eq!(subj(&c.only_left), ["f2", "f1"]);
    assert_eq!(subj(&c.only_right), ["m1"]);
    assert_eq!((c.only_left_count, c.only_right_count), (2, 1));
    // The cap only limits the lists, not the counts.
    let capped = repo.compare("feature", "main", 1).unwrap();
    assert_eq!((capped.only_left.len(), capped.only_left_count), (1, 2));

    // Tip to tip: everything that differs. Since the fork: only feature's work.
    let paths = |v: Vec<CommitFile>| v.into_iter().map(|f| (f.path, f.kind)).collect::<Vec<_>>();
    assert_eq!(
        paths(repo.diff_files(&m1, &f2).unwrap()),
        [
            ("a.txt".to_string(), ChangeKind::Modified),
            ("f.txt".to_string(), ChangeKind::Added),
            ("m.txt".to_string(), ChangeKind::Deleted),
        ]
    );
    assert_eq!(
        paths(repo.diff_files(&base, &f2).unwrap()),
        [
            ("a.txt".to_string(), ChangeKind::Modified),
            ("f.txt".to_string(), ChangeKind::Added)
        ]
    );

    assert!(repo.compare("--all", "main", 10).is_err());
    assert!(repo.compare("nope", "main", 10).is_err());
    assert_eq!(repo.resolve_commit("HEAD").unwrap(), m1);
}
