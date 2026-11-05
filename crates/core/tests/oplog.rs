//! Operation log: recording writes and undoing them on a real repository.

use std::fs;

use glide_core::{OpKind, OpMeta, RepoSession};
use glide_git_backend::{GitCli, ResetMode, RunOptions, StashOptions};

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

fn git(dir: &std::path::Path, args: &[&str]) -> String {
    cli()
        .run(dir, args, RunOptions::write())
        .unwrap()
        .stdout_str()
        .trim()
        .to_string()
}

/// A repository with one commit, and a session on it (with an on-disk log).
async fn setup() -> (
    tempfile::TempDir,
    tempfile::TempDir,
    std::sync::Arc<RepoSession>,
) {
    let dir = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    fs::write(dir.path().join("a.txt"), "one\n").unwrap();
    git(dir.path(), &["add", "a.txt"]);
    git(dir.path(), &["commit", "-q", "-m", "first"]);
    let repo = glide_git_backend::Repo::open(cli(), dir.path()).unwrap();
    let s = RepoSession::from_repo_with(repo, Some(cache.path().to_path_buf()))
        .await
        .unwrap();
    (dir, cache, s)
}

fn head(dir: &std::path::Path) -> String {
    git(dir, &["rev-parse", "HEAD"])
}

#[tokio::test(flavor = "multi_thread")]
async fn undo_commit_keeps_changes_staged() {
    let (dir, cache, s) = setup().await;
    let first = head(dir.path());
    fs::write(dir.path().join("a.txt"), "two\n").unwrap();
    s.write(|r| r.stage(&["a.txt".into()])).await.unwrap();
    s.write_op(OpMeta::new(OpKind::Commit, "提交 second"), |r| {
        r.commit("second", false)
    })
    .await
    .unwrap();
    let ops = s.operations();
    assert_eq!(ops.len(), 1);
    assert!(ops[0].undo.as_deref().unwrap().contains("暂存区"));

    s.undo(ops[0].id).await.unwrap();
    assert_eq!(head(dir.path()), first);
    assert_eq!(
        git(dir.path(), &["diff", "--cached", "--name-only"]),
        "a.txt"
    );
    let ops = s.operations();
    assert_eq!(ops[0].kind, OpKind::Undo);
    assert!(ops[1].undone);
    // Undoing twice is refused.
    assert!(s.undo(ops[1].id).await.is_err());

    // The log survives a restart.
    drop(s);
    let repo = glide_git_backend::Repo::open(cli(), dir.path()).unwrap();
    let s = RepoSession::from_repo_with(repo, Some(cache.path().to_path_buf()))
        .await
        .unwrap();
    assert_eq!(s.operations().len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn undo_branch_and_checkout() {
    let (dir, _cache, s) = setup().await;
    s.write_op(OpMeta::new(OpKind::Branch, "新建分支 x"), |r| {
        r.create_branch("x", None, true)
    })
    .await
    .unwrap();
    assert_eq!(git(dir.path(), &["symbolic-ref", "HEAD"]), "refs/heads/x");
    let id = s.operations()[0].id;
    s.undo(id).await.unwrap();
    assert_eq!(
        git(dir.path(), &["symbolic-ref", "HEAD"]),
        "refs/heads/main"
    );
    assert_eq!(git(dir.path(), &["branch", "--list", "x"]), "");

    // Deleting a branch comes back.
    git(dir.path(), &["branch", "old"]);
    let old = git(dir.path(), &["rev-parse", "old"]);
    s.write_op(OpMeta::new(OpKind::Branch, "删除 old"), |r| {
        r.delete_branch("old", true)
    })
    .await
    .unwrap();
    s.undo(s.operations()[0].id).await.unwrap();
    assert_eq!(git(dir.path(), &["rev-parse", "old"]), old);
}

#[tokio::test(flavor = "multi_thread")]
async fn undo_refused_after_later_changes() {
    let (dir, _cache, s) = setup().await;
    fs::write(dir.path().join("a.txt"), "two\n").unwrap();
    s.write(|r| r.stage_all()).await.unwrap();
    s.write_op(OpMeta::new(OpKind::Commit, "c2"), |r| r.commit("c2", false))
        .await
        .unwrap();
    let id = s.operations()[0].id;
    // Someone commits on top outside the app.
    fs::write(dir.path().join("a.txt"), "three\n").unwrap();
    git(dir.path(), &["commit", "-q", "-am", "c3"]);
    let err = s.undo(id).await.unwrap_err().to_string();
    assert!(err.contains("无法安全撤销"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn undo_rollback_restores_files() {
    let (dir, _cache, s) = setup().await;
    fs::write(dir.path().join("a.txt"), "changed\n").unwrap();
    fs::write(dir.path().join("new.txt"), "untracked\n").unwrap();
    fs::write(dir.path().join("keep.txt"), "other\n").unwrap();
    let meta = OpMeta::new(OpKind::Rollback, "回滚 2 个文件")
        .backup_files(vec!["a.txt".into(), "new.txt".into()]);
    s.write_op(meta, |r| r.rollback(&["a.txt".into()], &["new.txt".into()]))
        .await
        .unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "one\n"
    );
    assert!(!dir.path().join("new.txt").exists());

    let op = s.operations()[0].clone();
    assert!(op.undo.is_some(), "{op:?}");
    s.undo(op.id).await.unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "changed\n"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("new.txt")).unwrap(),
        "untracked\n"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("keep.txt")).unwrap(),
        "other\n"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn undo_rollback_refused_when_file_edited_again() {
    let (dir, _cache, s) = setup().await;
    fs::write(dir.path().join("a.txt"), "changed\n").unwrap();
    let meta = OpMeta::new(OpKind::Rollback, "回滚").backup_files(vec!["a.txt".into()]);
    s.write_op(meta, |r| r.rollback(&["a.txt".into()], &[]))
        .await
        .unwrap();
    fs::write(dir.path().join("a.txt"), "edited later\n").unwrap();
    let err = s.undo(s.operations()[0].id).await.unwrap_err().to_string();
    assert!(err.contains("a.txt"), "{err}");
    assert_eq!(
        fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "edited later\n"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn undo_hard_reset_and_stash() {
    let (dir, _cache, s) = setup().await;
    let first = head(dir.path());
    fs::write(dir.path().join("a.txt"), "two\n").unwrap();
    git(dir.path(), &["commit", "-q", "-am", "second"]);
    let second = head(dir.path());
    fs::write(dir.path().join("a.txt"), "dirty\n").unwrap();

    let meta = OpMeta::new(OpKind::Reset, "硬重置").backup_all();
    s.write_op(meta, move |r| r.reset(&first, ResetMode::Hard))
        .await
        .unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "one\n"
    );
    s.undo(s.operations()[0].id).await.unwrap();
    assert_eq!(head(dir.path()), second);
    assert_eq!(
        fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "dirty\n"
    );

    // Stash push → undo pops it back.
    let opts = StashOptions {
        message: Some("wip".into()),
        ..StashOptions::default()
    };
    s.write_op(OpMeta::new(OpKind::Stash, "stash"), move |r| {
        r.stash_push(&opts)
    })
    .await
    .unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "two\n"
    );
    s.undo(s.operations()[0].id).await.unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "dirty\n"
    );
    assert_eq!(git(dir.path(), &["stash", "list"]), "");

    // Stash drop → undo stores it again.
    git(dir.path(), &["stash", "push", "-q", "-m", "keep me"]);
    let e = s.repo().stash_list().unwrap().remove(0);
    let meta =
        OpMeta::new(OpKind::Stash, "删除 stash").dropped_stash(e.oid.clone(), e.message.clone());
    s.write_op(meta, move |r| r.stash_drop(e.index, &e.oid))
        .await
        .unwrap();
    s.undo(s.operations()[0].id).await.unwrap();
    assert!(git(dir.path(), &["stash", "list"]).contains("keep me"));
}

#[tokio::test(flavor = "multi_thread")]
async fn push_and_conflicts_are_not_undoable() {
    let (dir, _cache, s) = setup().await;
    git(dir.path(), &["checkout", "-q", "-b", "side"]);
    fs::write(dir.path().join("a.txt"), "side\n").unwrap();
    git(dir.path(), &["commit", "-q", "-am", "side"]);
    let side = head(dir.path());
    git(dir.path(), &["checkout", "-q", "main"]);
    fs::write(dir.path().join("a.txt"), "main\n").unwrap();
    git(dir.path(), &["commit", "-q", "-am", "main"]);
    // A conflicting cherry-pick stops half-way: logged, but not undoable.
    let r = s
        .write_op(OpMeta::new(OpKind::CherryPick, "cherry-pick"), move |r| {
            r.cherry_pick(&side)
        })
        .await;
    assert!(r.is_err());
    let op = s.operations()[0].clone();
    assert!(op.title.contains("未完成"));
    assert!(op.undo.is_none() && op.blocked.is_some());

    // A no-op is not listed.
    s.write(|r| r.abort_operation()).await.unwrap();
    let n = s.operations().len();
    s.write_op(OpMeta::new(OpKind::Checkout, "切换到 main"), |r| {
        r.checkout_branch("main", false)
    })
    .await
    .unwrap();
    assert_eq!(s.operations().len(), n);
}

#[tokio::test(flavor = "multi_thread")]
async fn undo_shelve_and_unshelve() {
    let (dir, _cache, s) = setup().await;
    fs::write(dir.path().join("a.txt"), "shelved\n").unwrap();
    fs::write(dir.path().join("n.txt"), "new\n").unwrap();
    let paths = vec!["a.txt".to_string(), "n.txt".to_string()];
    let meta = OpMeta::new(OpKind::Shelve, "搁置：w").backup_files(paths.clone());
    let p = paths.clone();
    let e = s
        .write_op(meta, move |r| r.shelve(&p, "w", None))
        .await
        .unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "one\n"
    );

    // Undo the shelve: the entry goes away and the files come back.
    let op = s.operations()[0].clone();
    assert!(op.undo.as_deref().unwrap().contains("搁置记录"), "{op:?}");
    s.undo(op.id).await.unwrap();
    assert!(s.repo().shelf_list().unwrap().is_empty());
    assert_eq!(
        fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "shelved\n"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("n.txt")).unwrap(),
        "new\n"
    );

    // Shelve again, unshelve, undo the unshelve: entry back, files reverted.
    let p = paths.clone();
    let e2 = s.write(move |r| r.shelve(&p, "w", None)).await.unwrap();
    assert!(!e2.id.is_empty() && !e.id.is_empty());
    let meta = OpMeta::new(OpKind::Shelve, "取消搁置：w").backup_files(paths.clone());
    let (id, oid) = (e2.id.clone(), e2.oid.clone());
    s.write_op(meta, move |r| r.unshelve(&id, &oid, false))
        .await
        .unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "shelved\n"
    );
    s.undo(s.operations()[0].id).await.unwrap();
    assert_eq!(s.repo().shelf_list().unwrap().len(), 1);
    assert_eq!(
        fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "one\n"
    );
    assert!(!dir.path().join("n.txt").exists());
}
