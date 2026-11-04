//! Integration tests against real repositories in temp dirs.

use std::fs;
use std::path::Path;

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
    repo.push(false, None).unwrap();
    let snap = repo.snapshot().unwrap();
    assert_eq!(snap.head.upstream.as_deref(), Some("origin/main"));
    assert!(snap.branches.remote.iter().any(|b| b.name == "origin/main"));

    // Push a branch, delete it locally, then check it out from the remote.
    repo.create_branch("topic", None, true).unwrap();
    repo.push(false, None).unwrap();
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
    other_repo.push(false, None).unwrap();

    repo.checkout_branch("main", false).unwrap();
    repo.fetch(None).unwrap();
    assert_eq!(repo.status().unwrap().0.behind, 1);
    repo.pull(PullMode::FastForwardOnly, None).unwrap();
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
